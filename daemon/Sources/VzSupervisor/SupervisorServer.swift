import Darwin
import CryptoKit
import Foundation
import VzDaemonKit

enum SupervisorError: Error, CustomStringConvertible {
    case system(String, Int32)
    case socketInUse(String)
    case socketPathTooLong
    case database(String)

    var description: String {
        switch self {
        case let .system(operation, code):
            return "\(operation): \(String(cString: strerror(code)))"
        case let .socketInUse(path):
            return "supervisor already listens at \(path)"
        case .socketPathTooLong:
            return "Unix socket path is too long"
        case let .database(message):
            return "SQLite: \(message)"
        }
    }
}

final class SupervisorServer: @unchecked Sendable {
    static let proxiedAgentMethods: Set<String> = [
        "vm.agent.health",
        "vm.agent.version",
        "vm.agent.report_ip",
        "vm.agent.ca_inject",
        "vm.agent.network_probe",
    ]

    let socketPath: String
    let databasePath: String

    private let stateDirectory: URL
    private let startedAt = ContinuousClock.now
    private let stateLock = NSLock()
    private let edgeReconcileLock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false
    private let database: StateDatabase
    private let networkRegistry: NetworkRegistry
    private let edgeClient: VzEdgeClient
    private var helpers: [String: HelperRecord] = [:]
    private var helperProcesses: [String: Process] = [:]
    private var expectedHelperStops: Set<String> = []
    private var subscribers: [UUID: EventSubscriber] = [:]
    private var eventListeners: [UUID: EventListener] = [:]
    private var restServer: RestServer?
    private var networkResilience: NetworkResilienceController?
    private let restJobs: RestJobRunner
    private let restRouter: RestRouter
    private let apiListenSpec: RestListenSpec

    init(stateDirectory: URL, apiListen: RestListenSpec? = nil) throws {
        self.stateDirectory = stateDirectory
        try FileManager.default.createDirectory(
            at: stateDirectory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        guard chmod(stateDirectory.path, 0o700) == 0 else {
            throw SupervisorError.system("chmod state directory", errno)
        }
        socketPath = stateDirectory.appendingPathComponent("vz.sock").path
        databasePath = stateDirectory.appendingPathComponent("state.sqlite").path
        database = try StateDatabase(path: databasePath)
        networkRegistry = try NetworkRegistry(database: database, stateDirectory: stateDirectory)
        edgeClient = VzEdgeClient(
            socketPath: VzEdgeClient.defaultSocketPath(stateDirectory: stateDirectory)
        )
        apiListenSpec = try apiListen ?? RestConfig.resolve(
            stateDirectory: stateDirectory,
            flagValue: nil
        )
        restJobs = RestJobRunner(stateDirectory: stateDirectory)
        restRouter = RestRouter(
            jobs: restJobs,
            database: database,
            stateDirectory: stateDirectory
        )
    }

    func run() throws {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SupervisorError.system("socket", errno) }

        stateLock.withLock { listener = fd }
        do {
            try prepareSocketPath()
            var address = try unixAddress(path: socketPath)
            let bindResult = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard bindResult == 0 else { throw SupervisorError.system("bind", errno) }
            stateLock.withLock { ownsSocket = true }
            guard chmod(socketPath, 0o600) == 0 else {
                throw SupervisorError.system("chmod", errno)
            }
            guard Darwin.listen(fd, 16) == 0 else {
                throw SupervisorError.system("listen", errno)
            }
            reconcileEdge(reason: "startup")

            restRouter.server = self
            let rest = RestServer(listenSpec: apiListenSpec, router: restRouter)
            try rest.start()
            restServer = rest

            let resilience = NetworkResilienceController(
                probe: { [unowned self] in self.probeNetworkRecovery() },
                fallback: { [unowned self] result in self.attemptNetworkFallback(result) },
                event: { [unowned self] type, data in self.emit(type: type, data: data) }
            )
            networkResilience = resilience
            resilience.start()

            while true {
                let client = Darwin.accept(fd, nil, nil)
                if client < 0 {
                    if errno == EINTR { continue }
                    if stateLock.withLock({ listener < 0 }) { break }
                    throw SupervisorError.system("accept", errno)
                }
                var noSigPipe: Int32 = 1
                setsockopt(
                    client,
                    SOL_SOCKET,
                    SO_NOSIGPIPE,
                    &noSigPipe,
                    socklen_t(MemoryLayout<Int32>.size)
                )
                var sendTimeout = timeval(tv_sec: 2, tv_usec: 0)
                setsockopt(
                    client,
                    SOL_SOCKET,
                    SO_SNDTIMEO,
                    &sendTimeout,
                    socklen_t(MemoryLayout<timeval>.size)
                )
                DispatchQueue.global().async { [self] in
                    handle(client)
                    Darwin.close(client)
                }
            }
        } catch {
            stop()
            throw error
        }
        stop()
    }

    func stop() {
        networkResilience?.stop()
        networkResilience = nil
        let state = stateLock.withLock { () -> (Int32, Bool, [Int32]) in
            let current = listener
            let shouldUnlink = ownsSocket
            let clients = subscribers.values.map(\.fd)
            listener = -1
            ownsSocket = false
            subscribers.removeAll()
            return (current, shouldUnlink, clients)
        }
        if state.0 >= 0 {
            Darwin.shutdown(state.0, SHUT_RDWR)
            Darwin.close(state.0)
        }
        if state.1 {
            Darwin.unlink(socketPath)
        }
        for client in state.2 {
            Darwin.shutdown(client, SHUT_RDWR)
        }
        networkRegistry.shutdown()
        restServer?.stop()
        restServer = nil
    }

    /// In-process JSON-RPC dispatch for the REST control plane.
    func dispatchRPC(method: String, params: JSONValue? = nil) -> JSONRPCResponse {
        let request = JSONRPCRequest(method: method, params: params, id: .number(1))
        guard let data = try? JSONEncoder().encode(request) else {
            return JSONRPCResponse(
                error: JSONRPCError(code: -32700, message: "Encode error"),
                id: .null
            )
        }
        return response(for: data)
    }

    @discardableResult
    func addEventListener(
        filter: EventFilter,
        handler: @escaping @Sendable (EventEnvelope) -> Void
    ) -> UUID {
        let id = UUID()
        stateLock.withLock {
            eventListeners[id] = EventListener(filter: filter, handler: handler)
        }
        return id
    }

    func removeEventListener(_ id: UUID) {
        stateLock.withLock { _ = eventListeners.removeValue(forKey: id) }
    }

    var apiListenDescription: String { apiListenSpec.description }

    static func helperTerminationMessage(
        status: Int32,
        reason: Process.TerminationReason
    ) -> String {
        switch reason {
        case .exit:
            return "helper exited with status \(status) before reporting an error"
        case .uncaughtSignal:
            return "helper terminated by signal \(status) before reporting an error"
        @unknown default:
            return "helper terminated unexpectedly with status \(status)"
        }
    }

    static func helperTerminationState(expected: Bool, status: Int32) -> String {
        expected || status == 0 ? "stopped" : "failed"
    }

    private func handle(_ client: Int32) {
        guard peerUID(client) == geteuid() else { return }

        var pending = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = Darwin.read(client, &buffer, buffer.count)
            if count <= 0 { return }
            pending.append(buffer, count: count)

            while let newline = pending.firstIndex(of: 0x0A) {
                let line = pending[..<newline]
                pending.removeSubrange(...newline)
                guard !line.isEmpty else { continue }
                if let request = try? JSONRPCFraming.decode(JSONRPCRequest.self, from: Data(line)),
                   request.method == "events.subscribe"
                {
                    subscribe(client, request: request)
                    return
                }
                let response = response(for: Data(line))
                guard let encoded = try? JSONRPCFraming.encode(response) else { return }
                if !writeAll(encoded, to: client) { return }
            }
        }
    }

    private func response(for data: Data) -> JSONRPCResponse {
        let request: JSONRPCRequest
        do {
            request = try JSONRPCFraming.decode(JSONRPCRequest.self, from: data)
        } catch {
            return JSONRPCResponse(
                error: JSONRPCError(code: -32700, message: "Parse error"),
                id: .null
            )
        }
        guard request.jsonrpc == "2.0" else {
            return JSONRPCResponse(
                error: JSONRPCError(code: -32600, message: "Invalid Request"),
                id: request.id ?? .null
            )
        }

        switch request.method {
        case "daemon.health":
            let uptime = startedAt.duration(to: .now)
            let uptimeMilliseconds = uptime.components.seconds * 1_000
                + Int64(uptime.components.attoseconds / 1_000_000_000_000_000)
            let networkSnapshot = try? networkRegistry.snapshot()
            let orphanedNetworks = networkSnapshot?.networks.filter {
                $0.runtimeState != "active"
            }.count ?? 0
            let edgeHealth = vzEdgeHealth()
            let netHealth = vzNetHealth()
            // CP `ok` tracks local health; vz-net is reported separately so doctor
            // can WARN without treating the control plane as down.
            return JSONRPCResponse(
                result: .object([
                    "ok": .bool(true),
                    "version": .string(VzDaemonKit.version),
                    "pid": .number(Double(getpid())),
                    "uptime_ms": .number(Double(uptimeMilliseconds)),
                    "db_ok": .bool(true),
                    "networks": .number(Double(networkSnapshot?.networks.count ?? 0)),
                    "network_orphans": .number(Double(orphanedNetworks)),
                    "dns_ok": .bool(edgeHealth.ok),
                    "dns": edgeHealth.dns,
                    "vz_edge_ok": .bool(edgeHealth.ok),
                    "vz_edge": edgeHealth.json,
                    "vz_net_ok": .bool(netHealth.ok),
                    "vz_net": netHealth.json,
                    "network_resilience": networkResilience?.health() ?? .object([
                        "state": .string("stopped"),
                    ]),
                ]),
                id: request.id ?? .null
            )
        case "dns.status":
            return JSONRPCResponse(
                result: vzEdgeHealth().dns,
                id: request.id ?? .null
            )
        case "dns.lookup":
            do {
                let params = try networkParams(request.params)
                let name = try requiredString("name", from: params)
                return JSONRPCResponse(result: try edgeClient.lookup(name: name), id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "resilience.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                let probeEnabled = optionalBool("probe_enabled", from: params) ?? true
                let probeURL = try requiredString("probe_url", from: params)
                let restartVMs = optionalBool("restart_vms", from: params) ?? false
                guard let url = URL(string: probeURL),
                      ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
                      url.host != nil,
                      url.user == nil,
                      url.password == nil
                else {
                    throw NetworkRegistryError.invalid("invalid resilience probe_url")
                }
                try database.setNetworkResiliencePolicy(
                    project: project,
                    stack: stack,
                    probeEnabled: probeEnabled,
                    probeURL: probeURL,
                    restartVMs: restartVMs
                )
                return JSONRPCResponse(result: .object([
                    "project": .string(project),
                    "stack": .string(stack),
                    "probe_enabled": .bool(probeEnabled),
                    "probe_url": .string(probeURL),
                    "restart_vms": .bool(restartVMs),
                ]), id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "resilience.remove":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.removeNetworkResiliencePolicy(project: project)
                return JSONRPCResponse(
                    result: .object(["removed": .bool(true), "project": .string(project)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "daemon.version":
            return JSONRPCResponse(result: .string(VzDaemonKit.version), id: request.id ?? .null)
        case "vm.list":
            let records = stateLock.withLock {
                helpers.values.sorted { $0.vmID < $1.vmID }
            }
            return JSONRPCResponse(
                result: .array(records.map(\.json)),
                id: request.id ?? .null
            )
        case "vm.start":
            do {
                let params = try objectParams(request.params, context: "vm.start")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let bundle = try requiredReconcileString("bundle", from: params)
                if let current = stateLock.withLock({ helpers[vmID] }),
                   current.state == "starting" || current.state == "running"
                {
                    return JSONRPCResponse(result: current.json, id: request.id ?? .null)
                }
                let helperURL = URL(fileURLWithPath: CommandLine.arguments[0])
                    .deletingLastPathComponent()
                    .appendingPathComponent("vz-helper")
                guard FileManager.default.isExecutableFile(atPath: helperURL.path) else {
                    throw ReconcileRPCError.invalid("vz-helper not found at \(helperURL.path)")
                }
                let process = Process()
                process.executableURL = helperURL
                process.arguments = [
                    "run", "--vm-id", vmID, "--bundle", bundle,
                    "--supervisor-sock", socketPath,
                ]
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.standardError
                process.terminationHandler = { [weak self] process in
                    guard let self else { return }
                    let termination = self.stateLock.withLock { () -> (String, String?) in
                        self.helperProcesses.removeValue(forKey: vmID)
                        let expected = self.expectedHelperStops.remove(vmID) != nil
                        let state = Self.helperTerminationState(
                            expected: expected,
                            status: process.terminationStatus
                        )
                        if state == "stopped" {
                            self.helpers.removeValue(forKey: vmID)
                            return (state, nil)
                        }
                        if self.helpers[vmID]?.state == "failed" {
                            return (state, self.helpers[vmID]?.lastError)
                        }
                        let message = Self.helperTerminationMessage(
                            status: process.terminationStatus,
                            reason: process.terminationReason
                        )
                        self.helpers[vmID] = HelperRecord(
                            vmID: vmID,
                            state: "failed",
                            pid: Int(process.processIdentifier),
                            bundle: bundle,
                            lastError: message
                        )
                        return (state, message)
                    }
                    self.emit(type: "vm.state", data: [
                        "vm_id": .string(vmID),
                        "state": .string(termination.0),
                        "pid": .number(Double(process.processIdentifier)),
                        "bundle": .string(bundle),
                        "last_error": termination.1.map(JSONValue.string) ?? .null,
                    ])
                }
                stateLock.withLock {
                    _ = helpers.removeValue(forKey: vmID)
                    _ = expectedHelperStops.remove(vmID)
                }
                try process.run()
                stateLock.withLock { helperProcesses[vmID] = process }
                let result = JSONValue.object([
                    "vm_id": .string(vmID),
                    "state": .string("starting"),
                    "pid": .number(Double(process.processIdentifier)),
                    "bundle": .string(bundle),
                ])
                emit(type: "vm.state", data: [
                    "vm_id": .string(vmID),
                    "state": .string("starting"),
                    "pid": .number(Double(process.processIdentifier)),
                    "bundle": .string(bundle),
                ])
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "vm.stop":
            do {
                let params = try objectParams(request.params, context: "vm.stop")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let force = optionalBool("force", from: params) ?? false
                let signal = force ? SIGKILL : SIGTERM
                let record = stateLock.withLock { helpers[vmID] }
                if let record, record.state == "starting" || record.state == "running" {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    let killResult = Darwin.kill(pid_t(record.pid), signal)
                    let killErrno = errno
                    if killResult != 0, killErrno != ESRCH {
                        _ = stateLock.withLock { expectedHelperStops.remove(vmID) }
                        throw SupervisorError.system("stop helper \(vmID)", killErrno)
                    }
                    // Force / ESRCH / already-dead: drop bookkeeping immediately.
                    if force || killErrno == ESRCH || !helperProcessIsRunning(vmID) {
                        stateLock.withLock {
                            helperProcesses.removeValue(forKey: vmID)
                            helpers.removeValue(forKey: vmID)
                        }
                    }
                } else if let process = stateLock.withLock({ helperProcesses[vmID] }),
                          process.isRunning
                {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    if force {
                        _ = Darwin.kill(process.processIdentifier, SIGKILL)
                    } else {
                        process.terminate()
                    }
                    if force {
                        stateLock.withLock {
                            helperProcesses.removeValue(forKey: vmID)
                            helpers.removeValue(forKey: vmID)
                        }
                    }
                } else {
                    // No live helper — clear any leftover bookkeeping so vm.list drops the id.
                    stateLock.withLock {
                        helperProcesses.removeValue(forKey: vmID)
                        helpers.removeValue(forKey: vmID)
                    }
                }
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "state": .string("stopped"),
                        "force": .bool(force),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "vm.purge":
            do {
                let params = try objectParams(request.params, context: "vm.purge")
                let vmID = try requiredReconcileString("vm_id", from: params)
                // Hard-kill + clear helper bookkeeping even if the process is already gone.
                let record = stateLock.withLock { helpers[vmID] }
                if let record, record.state == "starting" || record.state == "running" {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    _ = Darwin.kill(pid_t(record.pid), SIGKILL)
                } else if let process = stateLock.withLock({ helperProcesses[vmID] }),
                          process.isRunning
                {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    _ = Darwin.kill(process.processIdentifier, SIGKILL)
                }
                stateLock.withLock {
                    helperProcesses.removeValue(forKey: vmID)
                    helpers.removeValue(forKey: vmID)
                }

                let detached = try networkRegistry.detachAll(vmID: vmID, vmIsStopped: true)
                let portsRemoved = try database.deletePortForwards(vmID: vmID)
                try reconcileEdgeThrowing(reason: "vm.purge")
                emit(
                    type: "vm.purged",
                    data: [
                        "vm_id": .string(vmID),
                        "detached_networks": .array(detached.map(JSONValue.string)),
                        "ports_removed": .number(Double(portsRemoved)),
                    ]
                )
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "purged": .bool(true),
                        "detached_networks": .array(detached.map(JSONValue.string)),
                        "ports_removed": .number(Double(portsRemoved)),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "net.create":
            do {
                let params = try networkParams(request.params)
                let natEgress = optionalBool("nat_egress", from: params) ?? true
                let backend = try optionalString("backend", from: params) ?? NetworkRecord.backendVmnet
                let record = try networkRegistry.create(
                    name: try requiredString("name", from: params),
                    cidr: try requiredString("cidr", from: params),
                    mode: try optionalString("mode", from: params) ?? "shared",
                    natEgress: natEgress,
                    backend: backend,
                    labels: try labels(from: params),
                    project: try optionalString("project", from: params),
                    stack: try optionalString("stack", from: params)
                )
                reconcileEdge(reason: "net.create")
                emit(type: "net.created", data: ["network": .string(record.name)])
                return JSONRPCResponse(result: record.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.attach":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let record = try networkRegistry.attach(
                    vmID: vmID,
                    networkName: try requiredString("network", from: params),
                    ip: try requiredString("ip", from: params),
                    labels: try labels(from: params),
                    project: try optionalString("project", from: params),
                    stack: try optionalString("stack", from: params),
                    vmIsStopped: vmIsStopped(vmID)
                )
                reconcileEdge(reason: "net.attach")
                emit(
                    type: "net.attached",
                    data: [
                        "vm_id": .string(record.vmID),
                        "network": .string(record.networkName),
                        "ip": .string(record.ip),
                    ]
                )
                return JSONRPCResponse(result: record.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.detach":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let network = try requiredString("network", from: params)
                try networkRegistry.detach(
                    vmID: vmID,
                    networkName: network,
                    vmIsStopped: vmIsStopped(vmID)
                )
                reconcileEdge(reason: "net.detach")
                emit(
                    type: "net.detached",
                    data: ["vm_id": .string(vmID), "network": .string(network)]
                )
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "network": .string(network),
                        "detached": .bool(true),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.delete":
            do {
                let params = try networkParams(request.params)
                let name = try requiredString("name", from: params)
                try networkRegistry.delete(name: name)
                reconcileEdge(reason: "net.delete")
                emit(type: "net.deleted", data: ["network": .string(name)])
                return JSONRPCResponse(
                    result: .object(["name": .string(name), "deleted": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.list":
            do {
                return JSONRPCResponse(
                    result: try networkRegistry.snapshot().json,
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.default.show":
            do {
                let result: JSONValue
                if let (configured, network) = try networkRegistry.defaultNetwork() {
                    result = configured.json(network: network)
                } else {
                    result = .null
                }
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.default.set":
            do {
                let params = try networkParams(request.params)
                let configured = try networkRegistry.setDefault(
                    name: try requiredString("name", from: params),
                    cidr: try requiredString("cidr", from: params)
                )
                let network = try networkRegistry.defaultNetwork()?.1
                reconcileEdge(reason: "net.default.set")
                emit(type: "net.default.changed", data: [
                    "network": .string(configured.name),
                    "cidr": .string(configured.cidr),
                ])
                return JSONRPCResponse(
                    result: configured.json(network: network),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                let portsValue = params["ports"] ?? .array([])
                guard case let .array(items) = portsValue else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "ports must be an array"),
                        id: request.id ?? .null
                    )
                }
                var records: [PortForwardRecord] = []
                for item in items {
                    guard case let .object(object) = item else { continue }
                    let bind = try requiredString("bind", from: object)
                    let hostPort = try requiredPort("host_port", from: object)
                    let guestIP = try requiredString("guest_ip", from: object)
                    let guestPort = try requiredPort("guest_port", from: object)
                    let vmID = try requiredString("vm_id", from: object)
                    let source = try optionalString("source", from: object) ?? "\(bind):\(hostPort)"
                    records.append(
                        PortForwardRecord(
                            bind: bind,
                            hostPort: hostPort,
                            guestIP: guestIP,
                            guestPort: guestPort,
                            vmID: vmID,
                            source: source,
                            project: project,
                            stack: stack
                        )
                    )
                }
                try database.replacePortForwards(project: project, stack: stack, records: records)
                try reconcileEdgeThrowing(reason: "port.ensure")
                emit(
                    type: "port.ensured",
                    data: [
                        "project": .string(project),
                        "stack": .string(stack),
                        "count": .number(Double(records.count)),
                    ]
                )
                return JSONRPCResponse(
                    result: .object(["ports": .array(records.map(\.json))]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.list":
            do {
                let params = (try? networkParams(request.params)) ?? [:]
                let project = try optionalString("project", from: params)
                let stack = try optionalString("stack", from: params)
                let records = try database.portForwards(project: project, stack: stack)
                return JSONRPCResponse(
                    result: .object(["ports": .array(records.map(\.json))]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                try database.deletePortForwards(project: project, stack: stack)
                try reconcileEdgeThrowing(reason: "port.purge")
                emit(
                    type: "port.purged",
                    data: ["project": .string(project), "stack": .string(stack)]
                )
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "stack": .string(stack),
                        "purged": .bool(true),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.records.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let records = params["records"] ?? .array([])
                guard case .array = records else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "records must be an array"),
                        id: request.id ?? .null
                    )
                }
                try database.setEdgeDNSRecords(project: project, records: records)
                let edge = try reconcileEdgeThrowing(reason: "dns.records.ensure")
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "records": records,
                        "dns": edgeDNS(from: edge),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.records.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeDNSRecords(project: project, records: .array([]))
                try reconcileEdgeThrowing(reason: "dns.records.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.host_services.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let hostsValue = params["hosts"] ?? .array([])
                guard case let .array(items) = hostsValue else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "hosts must be an array"),
                        id: request.id ?? .null
                    )
                }
                let hosts = items.compactMap { item -> String? in
                    if case let .string(value) = item { return value }
                    return nil
                }
                let hostsJSON = JSONValue.array(hosts.map(JSONValue.string))
                try database.setEdgeHostServices(project: project, hosts: hostsJSON)
                let edge = try reconcileEdgeThrowing(reason: "dns.host_services.ensure")
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "hosts": hostsJSON,
                        "dns": edgeDNS(from: edge),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "ingress.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                _ = try requiredString("caddyfile", from: params)
                var stored = params
                if stored["binary"] == nil {
                    stored["binary"] = .string(stateDirectory.appendingPathComponent("bin/caddy").path)
                }
                try database.setEdgeIngress(project: project, value: .object(stored))
                let edge = try reconcileEdgeThrowing(reason: "ingress.ensure")

                emit(
                    type: "ingress.ensured",
                    data: [
                        "project": .string(project),
                        "edge": edge,
                    ]
                )
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "edge": edge,
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "ingress.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeIngress(project: project, value: nil)
                try database.setEdgeHostServices(project: project, hosts: .array([]))
                try reconcileEdgeThrowing(reason: "ingress.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "oidc.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                _ = try requiredString("config", from: params)
                var stored = params
                if stored["binary"] == nil {
                    stored["binary"] = .string(stateDirectory.appendingPathComponent("bin/dex").path)
                }
                if stored["processName"] == nil { stored["processName"] = .string("dex-\(project)") }
                try database.setEdgeOIDC(project: project, value: .object(stored))
                let edge = try reconcileEdgeThrowing(reason: "oidc.ensure")
                let processName = try requiredString("processName", from: stored)
                emit(type: "oidc.ensured", data: [
                    "project": .string(project),
                    "process": .string(processName),
                ])
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "process": .string(processName),
                        "edge": edge,
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "oidc.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeOIDC(project: project, value: nil)
                try reconcileEdgeThrowing(reason: "oidc.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "vm.network.ensure":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let selection = try networkRegistry.ensureVMNetwork(
                    vmID: vmID,
                    requestedNetwork: try optionalString("network", from: params),
                    vmIsStopped: vmIsStopped(vmID)
                )
                if selection.created {
                    reconcileEdge(reason: "vm.network.ensure")
                    emit(type: "net.attached", data: [
                        "vm_id": .string(selection.attachment.vmID),
                        "network": .string(selection.attachment.networkName),
                        "ip": .string(selection.attachment.ip),
                        "automatic": .bool(selection.automatic),
                    ])
                }
                return JSONRPCResponse(result: selection.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "vm.exec":
            do {
                let params = try objectParams(request.params, context: "vm.exec")
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let timeoutSeconds = agentProxyTimeoutSeconds(from: params)
                let result = try HelperAgentClient.run(
                    method: "agent.exec",
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: timeoutSeconds
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "vm.exec_tty":
            do {
                let params = try objectParams(request.params, context: "vm.exec_tty")
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let result = try HelperAgentClient.run(
                    method: "agent.exec_tty",
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 60
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case let method where Self.proxiedAgentMethods.contains(method):
            do {
                let params = try objectParams(request.params, context: method)
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let helperMethod = "agent." + String(method.dropFirst("vm.agent.".count))
                let result = try HelperAgentClient.run(
                    method: helperMethod,
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: method == "vm.agent.ca_inject" ? 65 : 35
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "vm.mount.list", "vm.mount.add", "vm.mount.remove":
            do {
                let params = try objectParams(request.params, context: request.method)
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let helperMethod = "mount." + String(request.method.dropFirst("vm.mount.".count))
                let result = try HelperAgentClient.run(
                    method: helperMethod,
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 45
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "route.apply", "route.plan", "route.status":
            do {
                let operation = RouterOperation(
                    rawValue: String(request.method.dropFirst("route.".count))
                )!
                let params = try request.params.map { try networkParams($0) } ?? [:]
                let requestedRouter = try optionalString("router", from: params)
                let policies = operation == .status
                    ? []
                    : try forwardPolicies(from: params["policies"])
                let snapshot = try networkRegistry.snapshot()
                let records = stateLock.withLock {
                    helpers.values.filter { $0.state == "running" }
                }
                let routers = try records.filter { record in
                    if let requestedRouter { return record.vmID == requestedRouter }
                    return try vmHasRouterRole(bundle: record.bundle)
                }
                guard !routers.isEmpty else {
                    throw RouteApplyError.invalid(
                        requestedRouter.map { "router VM \($0) is not running" }
                            ?? "no running VM with roles: [router]"
                    )
                }
                var policyMatches: [String: Int] = [:]
                let plans = try routers.sorted(by: { $0.vmID < $1.vmID }).map { router in
                    guard try vmHasRouterRole(bundle: router.bundle) else {
                        throw RouteApplyError.invalid(
                            "VM \(router.vmID) does not declare roles: [router]"
                        )
                    }
                    let topology = try RouterPlan(
                        vmID: router.vmID,
                        networkRecords: snapshot.networks,
                        attachments: snapshot.attachments
                    )
                    let attached = Set(topology.networks.map(\.name))
                    let hasNatEgress = topology.networks.contains(where: \.natEgress)
                    let selectedPolicies = policies.filter { policy in
                        if let via = policy.via,
                           !ForwardPolicy.matchesVia(vmID: router.vmID, via: via)
                        {
                            return false
                        }
                        guard attached.contains(policy.network) else { return false }
                        var hasInternet = false
                        var hasNonInternet = false
                        for allow in policy.allow {
                            if allow.to == internetPolicyTarget {
                                hasInternet = true
                                continue
                            }
                            hasNonInternet = true
                            guard attached.contains(allow.to) else { return false }
                        }
                        // Internet-only policies bind to the NAT router; mixed
                        // policies (e.g. lan→containers + lan→internet) bind to
                        // the router that owns the non-internet destinations.
                        if hasInternet, !hasNonInternet, !hasNatEgress {
                            return false
                        }
                        policyMatches[policy.name, default: 0] += 1
                        return true
                    }
                    let staticRoutes = DockerBackendRoutes.staticRoutes(
                        forRouter: router.vmID,
                        networks: snapshot.networks,
                        attachments: snapshot.attachments
                    )
                    return try RouterPlan(
                        vmID: topology.vmID,
                        networks: topology.networks,
                        policies: selectedPolicies,
                        staticRoutes: staticRoutes
                    )
                }
                for policy in policies {
                    let matches = policyMatches[policy.name, default: 0]
                    guard matches == 1 else {
                        if let via = policy.via {
                            throw RouteApplyError.invalid(
                                matches == 0
                                    ? "policy \(policy.name) via \(via) does not match a running router"
                                    : "policy \(policy.name) via \(via) matches more than one running router"
                            )
                        }
                        throw RouteApplyError.invalid(
                            matches == 0
                                ? "policy \(policy.name) does not match a running router; set policies.*.via to pin a router"
                                : "policy \(policy.name) matches more than one running router; set policies.*.via to pin a router"
                        )
                    }
                }
                var results: [JSONValue] = []
                var anyChanged = false
                for plan in plans {
                    let helperResult = try HelperRouteClient.run(
                        operation,
                        plan,
                        stateDirectory: stateDirectory
                    )
                    guard case var .object(values) = helperResult,
                          case let .bool(changed)? = values["changed"]
                    else {
                        throw RouteApplyError.guest(
                            "router helper \(plan.vmID) returned invalid status"
                        )
                    }
                    anyChanged = anyChanged || changed
                    values["vm_id"] = .string(plan.vmID)
                    values["networks"] = .array(plan.networks.map(\.json))
                    results.append(.object(values))
                    if operation == .apply {
                        emit(
                            type: "route.applied",
                            data: ["vm_id": .string(plan.vmID), "changed": .bool(changed)]
                        )
                    }
                }
                return JSONRPCResponse(
                    result: .object([
                        "changed": .bool(anyChanged),
                        "routers": .array(results),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "helper.hello", "helper.state":
            guard let record = HelperRecord(params: request.params) else {
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32602, message: "Invalid helper params"),
                    id: request.id ?? .null
                )
            }
            stateLock.withLock {
                helpers[record.vmID] = record
            }
            emit(
                type: "vm.state",
                data: [
                    "vm_id": .string(record.vmID),
                    "state": .string(record.state),
                    "pid": .number(Double(record.pid)),
                    "bundle": .string(record.bundle),
                    "last_error": record.lastError.map(JSONValue.string) ?? .null,
                ]
            )
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
        case "helper.networks":
            do {
                let params = try objectParams(request.params, context: "helper.networks")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let attachments = try networkRegistry.serializedAttachments(for: vmID)
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "attachments": .array(attachments.map(\.json)),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "vm.clock_corrected":
            guard case let .object(params)? = request.params,
                  case .string? = params["vm_id"],
                  case .string? = params["reason"],
                  case .number? = params["observed_guest_unix_ms"],
                  case .number? = params["offset_ms"],
                  params["action"] == .string("stepped")
            else {
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32602, message: "Invalid clock event params"),
                    id: request.id ?? .null
                )
            }
            emit(type: "vm.clock_corrected", data: params)
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
        case "stack.inspect":
            do {
                let params = try objectParams(request.params, context: "stack.inspect")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let state = try database.stackState(stackID: stackID)
                return JSONRPCResponse(result: state.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.begin":
            do {
                let params = try objectParams(request.params, context: "stack.begin")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let desiredHash = try requiredReconcileString("desired_hash", from: params)
                let mode = try requiredReconcileString("mode", from: params)
                guard ["up", "apply", "down", "resume"].contains(mode) else {
                    throw ReconcileRPCError.invalid("invalid reconcile mode")
                }
                let purge = optionalBool("purge", from: params) ?? false
                let payload = try jsonString([
                    "desired_hash": .string(desiredHash),
                    "mode": .string(mode),
                    "purge": .bool(purge),
                ])
                let journal = try database.beginApply(
                    stackID: stackID,
                    holder: holder,
                    desiredHash: desiredHash,
                    payload: payload,
                    resume: mode == "resume"
                )
                let common: [String: JSONValue] = [
                    "invocation_id": .string(journal.id),
                    "mode": .string(mode),
                    "stack_id": .string(stackID),
                    "generation": .number(Double(journal.generation)),
                ]
                emit(type: "apply.started", data: common)
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.step":
            do {
                let params = try objectParams(request.params, context: "stack.step")
                let requestedStatus = try requiredReconcileString("status", from: params)
                guard ["running", "completed", "failed"].contains(requestedStatus) else {
                    throw ReconcileRPCError.invalid("invalid journal step status")
                }
                let journal = try database.advanceApply(
                    id: try requiredReconcileString("id", from: params),
                    stackID: try requiredReconcileString("stack_id", from: params),
                    holder: try requiredReconcileString("holder", from: params),
                    step: try requiredReconcileString("step", from: params),
                    status: requestedStatus == "completed" ? "running" : requestedStatus,
                    error: optionalReconcileString("error", from: params)
                )
                emit(type: "apply.step", data: [
                    "invocation_id": .string(journal.id),
                    "step": .string(journal.step),
                    "status": .string(requestedStatus == "completed" ? "done" : journal.status),
                    "error": journal.error.map(JSONValue.string) ?? .null,
                ])
                if requestedStatus == "failed" {
                    emit(type: "apply.failed", data: [
                        "invocation_id": .string(journal.id),
                        "mode": .string(journal.operationMode ?? "apply"),
                        "step": .string(journal.step),
                        "exit_code": .number(24),
                        "error": journal.error.map(JSONValue.string) ?? .string("step_failed"),
                    ])
                }
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.finish":
            do {
                let params = try objectParams(request.params, context: "stack.finish")
                let id = try requiredReconcileString("id", from: params)
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let operationMode = try database.stackState(stackID: stackID)
                    .journal?.operationMode ?? "apply"
                guard let resources = params["resources"] else {
                    throw ReconcileRPCError.invalid("missing resources")
                }
                try database.finishApply(
                    id: id,
                    stackID: stackID,
                    holder: holder,
                    resourcesJSON: try jsonString(resources)
                )
                emit(type: "apply.finished", data: [
                    "invocation_id": .string(id),
                    "mode": .string(operationMode),
                    "stack_id": .string(stackID),
                    "exit_code": .number(0),
                ])
                return JSONRPCResponse(
                    result: .object(["ok": .bool(true), "id": .string(id)]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.abort":
            do {
                let params = try objectParams(request.params, context: "stack.abort")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let journal = try database.abortApply(stackID: stackID, holder: holder)
                emit(type: "apply.finished", data: [
                    "invocation_id": .string(journal.id),
                    "mode": .string("abort"),
                    "stack_id": .string(stackID),
                    "exit_code": .number(0),
                ])
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        default:
            return JSONRPCResponse(
                error: JSONRPCError(code: -32601, message: "Method not found"),
                id: request.id ?? .null
            )
        }
    }

    private func subscribe(_ client: Int32, request: JSONRPCRequest) {
        let expression: String?
        if request.params == nil {
            expression = nil
        } else if case let .object(params)? = request.params {
            if params["filter"] == nil || params["filter"] == .null {
                expression = nil
            } else if case let .string(filter)? = params["filter"] {
                expression = filter
            } else {
                writeSubscriptionError(client, request: request)
                return
            }
        } else {
            writeSubscriptionError(client, request: request)
            return
        }

        guard let filter = try? EventFilter(expression) else {
            writeSubscriptionError(client, request: request)
            return
        }

        let id = UUID()
        let response = JSONRPCResponse(
            result: .object(["ok": .bool(true), "v": .number(1)]),
            id: request.id ?? .null
        )
        guard let encoded = try? JSONRPCFraming.encode(response) else { return }
        let registered = stateLock.withLock { () -> Bool in
            guard writeAll(encoded, to: client) else { return false }
            subscribers[id] = EventSubscriber(fd: client, filter: filter)
            return true
        }
        guard registered else { return }
        defer {
            _ = stateLock.withLock {
                subscribers.removeValue(forKey: id)
            }
        }

        var byte: UInt8 = 0
        while Darwin.read(client, &byte, 1) > 0 {}
    }

    private func helperProcessIsRunning(_ vmID: String) -> Bool {
        stateLock.withLock {
            helperProcesses[vmID]?.isRunning == true
        }
    }

    private func requireRunningHelper(vmID: String) throws {
        let running = stateLock.withLock {
            helpers[vmID]?.state == "running"
        }
        guard running else {
            throw RouteApplyError.unavailable("VM \(vmID) is not running")
        }
    }

    private func agentProxyTimeoutSeconds(from params: [String: JSONValue]) -> Int {
        guard case let .number(value)? = params["timeout_ms"],
              value.rounded() == value,
              value > 0
        else {
            return 35
        }
        return max(35, Int(value / 1_000) + 5)
    }

    private func vmIsStopped(_ vmID: String) -> Bool {
        let reportsRunning = stateLock.withLock {
            guard let helper = helpers[vmID] else { return false }
            return helper.state == "starting" || helper.state == "running"
        }
        guard !reportsRunning else { return false }

        let path = stateDirectory
            .appendingPathComponent("helpers", isDirectory: true)
            .appendingPathComponent("\(StateFileName.component(vmID)).lock")
            .path
        let descriptor = Darwin.open(path, O_RDONLY)
        if descriptor < 0 {
            return errno == ENOENT
        }
        defer { Darwin.close(descriptor) }
        if flock(descriptor, LOCK_EX | LOCK_NB) == 0 {
            flock(descriptor, LOCK_UN)
            return true
        }
        return false
    }

    private func networkParams(_ value: JSONValue?) throws -> [String: JSONValue] {
        guard case let .object(params)? = value else {
            throw NetworkRegistryError.invalid("network params must be an object")
        }
        return params
    }

    private func requiredString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String {
        guard case let .string(value)? = params[key], !value.isEmpty else {
            throw NetworkRegistryError.invalid("missing or invalid \(key)")
        }
        return value
    }

    private func optionalString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String? {
        guard let raw = params[key], raw != .null else { return nil }
        guard case let .string(value) = raw, !value.isEmpty else {
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
        return value
    }

    private func optionalStringArray(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> [String]? {
        guard let raw = params[key], raw != .null else { return nil }
        guard case let .array(items) = raw else {
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
        return try items.map { item in
            guard case let .string(value) = item else {
                throw NetworkRegistryError.invalid("invalid \(key) entry")
            }
            return value
        }
    }

    private func requiredPort(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> UInt16 {
        guard let raw = params[key] else {
            throw NetworkRegistryError.invalid("missing \(key)")
        }
        switch raw {
        case let .number(value):
            guard value >= 1, value <= 65535, value.rounded() == value else {
                throw NetworkRegistryError.invalid("invalid \(key)")
            }
            return UInt16(value)
        case let .string(value):
            guard let parsed = UInt16(value), parsed > 0 else {
                throw NetworkRegistryError.invalid("invalid \(key)")
            }
            return parsed
        default:
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
    }

    private func optionalPort(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> UInt16? {
        guard let raw = params[key], raw != .null else { return nil }
        return try requiredPort(key, from: params)
    }

    private func labels(from params: [String: JSONValue]) throws -> [String: String] {
        guard let raw = params["labels"], raw != .null else { return [:] }
        guard case let .object(values) = raw else {
            throw NetworkRegistryError.invalid("labels must be an object")
        }
        var labels: [String: String] = [:]
        for (key, rawValue) in values {
            guard case let .string(value) = rawValue else {
                throw NetworkRegistryError.invalid("label \(key) must be a string")
            }
            labels[key] = value
        }
        return labels
    }

    private func networkErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let networkError: NetworkRegistryError
        if let error = error as? NetworkRegistryError {
            networkError = error
        } else if let error = error as? NetworkValidationError {
            networkError = .invalid(error.description)
        } else {
            networkError = .runtime(String(describing: error))
        }
        return JSONRPCResponse(
            error: JSONRPCError(code: networkError.rpcCode, message: networkError.description),
            id: request.id ?? .null
        )
    }

    private func routeErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let routeError = error as? RouteApplyError
            ?? .guest(String(describing: error))
        return JSONRPCResponse(
            error: JSONRPCError(code: routeError.rpcCode, message: routeError.description),
            id: request.id ?? .null
        )
    }

    private func reconcileErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let code: Int
        let message: String
        switch error {
        case let ReconcileDatabaseError.incomplete(journal):
            code = 5
            message = "incomplete journal \(journal.id) at \(journal.step); "
                + "run `vzctl apply --resume` or `vzctl apply --abort`"
        case let ReconcileDatabaseError.leaseHeld(lease):
            code = 6
            message = "stack lease held by \(lease.holder) until \(lease.expiresAtText)"
        case ReconcileDatabaseError.generationChanged:
            code = 5
            message = "desired config changed; run `vzctl apply --abort` before applying"
        case ReconcileDatabaseError.noIncomplete:
            code = 5
            message = "no incomplete journal"
        case ReconcileDatabaseError.leaseLost:
            code = 6
            message = "stack lease was lost"
        case let ReconcileRPCError.invalid(value):
            code = -32602
            message = value
        default:
            code = -32010
            message = String(describing: error)
        }
        return JSONRPCResponse(
            error: JSONRPCError(code: code, message: message),
            id: request.id ?? .null
        )
    }

    private func objectParams(
        _ value: JSONValue?,
        context: String
    ) throws -> [String: JSONValue] {
        guard case let .object(params)? = value else {
            throw ReconcileRPCError.invalid("\(context) params must be an object")
        }
        return params
    }

    private func requiredReconcileString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String {
        guard case let .string(value)? = params[key], !value.isEmpty else {
            throw ReconcileRPCError.invalid("missing or invalid \(key)")
        }
        return value
    }

    private func optionalReconcileString(
        _ key: String,
        from params: [String: JSONValue]
    ) -> String? {
        guard case let .string(value)? = params[key] else { return nil }
        return value
    }

    private func optionalBool(
        _ key: String,
        from params: [String: JSONValue]
    ) -> Bool? {
        guard case let .bool(value)? = params[key] else { return nil }
        return value
    }

    private func jsonString(_ value: JSONValue) throws -> String {
        String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }

    private func jsonString(_ value: [String: JSONValue]) throws -> String {
        try jsonString(.object(value))
    }

    private func vmHasRouterRole(bundle: String) throws -> Bool {
        let manifest = URL(fileURLWithPath: bundle, isDirectory: true)
            .appendingPathComponent("vm.json")
        let data = try Data(contentsOf: manifest)
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let roles = root["roles"] as? [String]
        else {
            throw RouteApplyError.invalid(
                "VM manifest \(manifest.path) has no roles array"
            )
        }
        return roles.contains("router")
    }

    private func forwardPolicies(from value: JSONValue?) throws -> [ForwardPolicy] {
        guard let value else { return [] }
        guard case let .array(items) = value else {
            throw RouteApplyError.invalid("policies must be an array")
        }
        return try items.map { item in
            guard case let .object(policy) = item,
                  case let .string(name)? = policy["name"],
                  case let .string(network)? = policy["network"],
                  case let .string(forward)? = policy["forward"]
            else {
                throw RouteApplyError.invalid("invalid forward policy")
            }
            let rawAllows: [JSONValue]
            if case let .array(values)? = policy["allow"] {
                rawAllows = values
            } else {
                rawAllows = []
            }
            let allows = try rawAllows.map { item -> PolicyAllow in
                guard case let .object(allow) = item,
                      case let .string(to)? = allow["to"],
                      case let .string(proto)? = allow["proto"]
                else {
                    throw RouteApplyError.invalid("invalid allow rule in policy \(name)")
                }
                let ports: [Int]
                if case let .array(values)? = allow["ports"] {
                    ports = try values.map {
                        guard case let .number(value) = $0, value.rounded() == value else {
                            throw RouteApplyError.invalid("invalid port in policy \(name)")
                        }
                        return Int(value)
                    }
                } else {
                    ports = []
                }
                return PolicyAllow(to: to, proto: proto, ports: ports)
            }
            return ForwardPolicy(
                name: name,
                network: network,
                forward: forward,
                allow: allows,
                via: {
                    if case let .string(via)? = policy["via"] { return via }
                    return nil
                }()
            )
        }
    }

    private func writeSubscriptionError(_ client: Int32, request: JSONRPCRequest) {
        let response = JSONRPCResponse(
            error: JSONRPCError(code: -32602, message: "Invalid event filter"),
            id: request.id ?? .null
        )
        if let encoded = try? JSONRPCFraming.encode(response) {
            _ = writeAll(encoded, to: client)
        }
    }

    private func emit(type: String, data: [String: JSONValue]) {
        let event = EventEnvelope(type: type, data: data)
        guard let encoded = try? JSONRPCFraming.encode(event) else { return }
        let listeners: [EventListener] = stateLock.withLock {
            let failed = subscribers.compactMap { id, subscriber -> UUID? in
                guard subscriber.filter.matches(type) else { return nil }
                return writeAll(encoded, to: subscriber.fd) ? nil : id
            }
            for id in failed {
                subscribers.removeValue(forKey: id)
            }
            return Array(eventListeners.values)
        }
        for listener in listeners where listener.filter.matches(type) {
            listener.handler(event)
        }
    }

    private func vzNetHealth() -> (ok: Bool, json: JSONValue) {
        let client = VzNetClient(
            socketPath: VzNetClient.defaultSocketPath(stateDirectory: stateDirectory)
        )
        do {
            let health = try client.health()
            return (
                health.ok,
                .object([
                    "ok": .bool(health.ok),
                    "version": .string(health.version),
                    "networks": .number(Double(health.networks)),
                    "socket": .string(client.socketPath),
                ])
            )
        } catch {
            return (
                false,
                .object([
                    "ok": .bool(false),
                    "socket": .string(client.socketPath),
                    "error": .string(String(describing: error)),
                ])
            )
        }
    }

    private func probeNetworkRecovery() -> NetworkRecoveryResult {
        let snapshot: NetworkSnapshot
        do {
            snapshot = try networkRegistry.snapshot()
        } catch {
            return NetworkRecoveryResult(
                internalOK: false,
                hostEgress: .unknown,
                networkEgress: [:],
                conflicts: [],
                error: "network snapshot: \(error)"
            )
        }
        let activeNetworks = snapshot.networks.filter {
            $0.runtimeState == "active" && !$0.isDockerBackend
        }
        let conflicts = HostRouteScanner.conflicts(networks: activeNetworks)
        var internalOK = true
        var errors: [String] = []

        do {
            let client = VzNetClient(
                socketPath: VzNetClient.defaultSocketPath(stateDirectory: stateDirectory),
                timeoutSeconds: 5
            )
            let verifications = try client.verify()
            if verifications.contains(where: { !$0.ok }) {
                internalOK = false
                errors.append("vz-net verification failed")
            }
        } catch {
            internalOK = false
            errors.append("vz-net: \(error)")
        }

        do {
            let edgeStatus = try reconcileEdgeThrowing(reason: "host.network_changed")
            guard case let .object(values) = edgeStatus,
                  values["ok"] == .bool(true)
            else {
                throw NetworkRegistryError.runtime("vz-edge status is degraded")
            }
        } catch {
            internalOK = false
            errors.append("vz-edge: \(error)")
        }

        let running = stateLock.withLock {
            Set(helpers.values.filter { $0.state == "running" }.map(\.vmID))
        }
        var representatives: [String: (attachment: NetworkAttachmentRecord, canProbe: Bool)] = [:]
        for network in activeNetworks {
            guard let attachment = snapshot.attachments.first(where: {
                $0.networkName == network.name && running.contains($0.vmID)
            }) else { continue }
            do {
                let version = try HelperAgentClient.run(
                    method: "agent.version",
                    params: .object(["vm_id": .string(attachment.vmID)]),
                    vmID: attachment.vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 3
                )
                representatives[network.name] = (
                    attachment, Self.agentSupportsNetworkProbe(version)
                )
            } catch {
                internalOK = false
                errors.append("guest agent \(network.name): \(error)")
            }
        }

        let policies = (try? database.networkResiliencePolicies()) ?? []
        let defaultProbeURL = URL(string: "https://captive.apple.com/")!
        let enabledPolicies = policies.filter(\.probeEnabled)
        let hostProbeURL = enabledPolicies.first.flatMap { URL(string: $0.probeURL) }
            ?? (policies.isEmpty ? defaultProbeURL : nil)
        let hostEgress = hostProbeURL.map { HostEgressProber().probe(url: $0) } ?? .unknown
        var guestEgress: [String: NetworkEgressResult] = [:]
        if hostEgress.classification == "online" {
            for network in activeNetworks where network.natEgress {
                let policy = policies.first { $0.project == network.project }
                if policy?.probeEnabled == false {
                    guestEgress[network.name] = .unknown
                    continue
                }
                let probeURL = policy.flatMap { URL(string: $0.probeURL) } ?? defaultProbeURL
                guard let representative = representatives[network.name] else {
                    guestEgress[network.name] = .unknown
                    continue
                }
                let attachment = representative.attachment
                do {
                    guard representative.canProbe else {
                        // Older agents remain compatible. Their missing optional probe
                        // must not turn a healthy dataplane into a false degradation.
                        guestEgress[network.name] = .unknown
                        continue
                    }
                    let raw = try HelperAgentClient.run(
                        method: "agent.network_probe",
                        params: .object([
                            "vm_id": .string(attachment.vmID),
                            "url": .string(probeURL.absoluteString),
                            "timeout_ms": .number(5_000),
                        ]),
                        vmID: attachment.vmID,
                        stateDirectory: stateDirectory,
                        timeoutSeconds: 10
                    )
                    guestEgress[network.name] = try Self.parseEgressResult(raw)
                } catch {
                    guestEgress[network.name] = NetworkEgressResult(
                        classification: "offline",
                        phase: "agent",
                        statusCode: nil,
                        latencyMS: 0,
                        errorCode: "agent"
                    )
                    errors.append("guest egress \(network.name): \(error)")
                }
            }
        }
        return NetworkRecoveryResult(
            internalOK: internalOK,
            hostEgress: hostEgress,
            networkEgress: guestEgress,
            conflicts: conflicts,
            error: errors.isEmpty ? nil : errors.joined(separator: "; ")
        )
    }

    static func agentSupportsNetworkProbe(_ value: JSONValue) -> Bool {
        guard case let .object(values) = value,
              case let .array(capabilities)? = values["capabilities"]
        else { return false }
        return capabilities.contains(.string("network_probe"))
    }

    private static func parseEgressResult(_ value: JSONValue) throws -> NetworkEgressResult {
        guard case let .object(values) = value,
              case let .string(classification)? = values["classification"],
              case let .string(phase)? = values["phase"],
              case let .number(latency)? = values["latency_ms"]
        else {
            throw RouteApplyError.guest("network_probe returned invalid result")
        }
        let statusCode: Int?
        if case let .number(value)? = values["status_code"] {
            statusCode = Int(value)
        } else {
            statusCode = nil
        }
        let errorCode: String?
        if case let .string(value)? = values["error_code"] {
            errorCode = value
        } else {
            errorCode = nil
        }
        return NetworkEgressResult(
            classification: classification,
            phase: phase,
            statusCode: statusCode,
            latencyMS: Int64(latency),
            errorCode: errorCode
        )
    }

    private func attemptNetworkFallback(_ result: NetworkRecoveryResult) -> Bool {
        guard result.internalOK,
              result.hostEgress.classification == "online",
              result.conflicts.isEmpty
        else { return false }
        let failedNetworks = result.networkEgress
            .filter { $0.value.classification == "offline" }
            .map(\.key)
            .sorted()
        guard !failedNetworks.isEmpty,
              let snapshot = try? networkRegistry.snapshot(),
              let policies = try? database.networkResiliencePolicies(),
              Self.networkFallbackPolicyAllows(
                  networks: failedNetworks,
                  snapshot: snapshot,
                  policies: policies
              )
        else { return false }

        var running: [String: String] = [:]
        for name in failedNetworks {
            let attachments = snapshot.attachments.filter { $0.networkName == name }
            let records = stateLock.withLock { helpers }
            for attachment in attachments {
                if let record = records[attachment.vmID], record.state == "running" {
                    running[record.vmID] = record.bundle
                }
            }
        }
        guard !running.isEmpty else { return false }

        emit(type: "network.fallback_restart", data: [
            "networks": .array(failedNetworks.map(JSONValue.string)),
            "vms": .array(running.keys.sorted().map(JSONValue.string)),
        ])
        for vmID in running.keys.sorted().reversed() {
            let response = dispatchRPC(
                method: "vm.stop",
                params: .object(["vm_id": .string(vmID), "force": .bool(false)])
            )
            if response.error != nil { return false }
        }
        let stopDeadline = DispatchTime.now().uptimeNanoseconds + 30_000_000_000
        while DispatchTime.now().uptimeNanoseconds < stopDeadline {
            let active = stateLock.withLock {
                running.keys.contains { vmID in
                    helpers[vmID].map { $0.state == "starting" || $0.state == "running" } ?? false
                        || helperProcesses[vmID]?.isRunning == true
                }
            }
            if !active { break }
            Thread.sleep(forTimeInterval: 0.1)
        }
        let stillActive = stateLock.withLock {
            running.keys.contains { helperProcesses[$0]?.isRunning == true }
        }
        guard !stillActive else { return false }

        do {
            for name in failedNetworks {
                try networkRegistry.recreateRuntime(name: name)
            }
        } catch {
            emit(type: "network.degraded", data: [
                "state": .string("degraded"),
                "error": .string("fallback network recreate failed: \(error)"),
            ])
            return false
        }

        for vmID in running.keys.sorted() {
            guard let bundle = running[vmID] else { continue }
            let response = dispatchRPC(
                method: "vm.start",
                params: .object([
                    "vm_id": .string(vmID),
                    "bundle": .string(bundle),
                ])
            )
            if response.error != nil { return false }
        }
        let startDeadline = DispatchTime.now().uptimeNanoseconds + 30_000_000_000
        while DispatchTime.now().uptimeNanoseconds < startDeadline {
            let ready = stateLock.withLock {
                running.keys.allSatisfy { helpers[$0]?.state == "running" }
            }
            if ready { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return false
    }

    static func networkFallbackPolicyAllows(
        networks names: [String],
        snapshot: NetworkSnapshot,
        policies: [NetworkResiliencePolicyRecord]
    ) -> Bool {
        names.allSatisfy { name in
            guard let network = snapshot.networks.first(where: { $0.name == name }),
                  let project = network.project,
                  let stack = network.stack,
                  policies.contains(where: {
                      $0.project == project && $0.stack == stack && $0.restartVMs
                  })
            else { return false }
            let attachments = snapshot.attachments.filter { $0.networkName == name }
            return !attachments.isEmpty && attachments.allSatisfy {
                $0.project == project && $0.stack == stack
            }
        }
    }

    private func reconcileEdge(reason: String) {
        do {
            let result = try reconcileEdgeThrowing(reason: reason)
            emit(type: "edge.reconciled", data: [
                "reason": .string(reason),
                "status": result,
            ])
        } catch {
            emit(type: "edge.reconcile_failed", data: [
                "reason": .string(reason),
                "error": .string(String(describing: error)),
            ])
        }
    }

    @discardableResult
    private func reconcileEdgeThrowing(reason: String) throws -> JSONValue {
        try edgeReconcileLock.withLock {
            let projects = try database.edgeProjects()
            let hostServices = try projects.flatMap { record -> [JSONValue] in
                guard case let .array(values) = record.hostServices else {
                    throw SupervisorError.database("edge host services must be an array")
                }
                return values
            }
            let dnsRecords = try projects.flatMap { record -> [JSONValue] in
                guard case let .array(values) = record.dnsRecords else {
                    throw SupervisorError.database("edge DNS records must be an array")
                }
                return values
            }
            let ingress = projects.compactMap(\.ingress)
            let oidc = projects.compactMap(\.oidc)
            let ports = try database.portForwards()
            let desired: JSONValue = .object([
                "network_snapshot": try networkRegistry.snapshot().json,
                "host_services": .array(hostServices),
                "dns_records": .array(dnsRecords),
                "port_forwards": .array(ports.map(\.json)),
                "ingress": .array(ingress),
                "oidc": .array(oidc),
            ])
            let generation = try database.nextEdgeGeneration()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode(desired)
            let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            return try edgeClient.reconcile(
                generation: generation, digest: digest, desired: desired
            )
        }
    }

    private func vzEdgeHealth() -> (ok: Bool, json: JSONValue, dns: JSONValue) {
        do {
            let health = try edgeClient.health()
            guard case let .object(values) = health else {
                return (false, health, .object(["ok": .bool(false)]))
            }
            let ok = values["ok"] == .bool(true)
            return (ok, health, values["dns"] ?? .object(["ok": .bool(false)]))
        } catch {
            let failure: JSONValue = .object([
                "ok": .bool(false),
                "socket": .string(edgeClient.socketPath),
                "error": .string(String(describing: error)),
            ])
            return (false, failure, failure)
        }
    }

    private func edgeDNS(from status: JSONValue) -> JSONValue {
        guard case let .object(values) = status else { return .null }
        return values["dns"] ?? .null
    }

    private func prepareSocketPath() throws {
        if FileManager.default.fileExists(atPath: socketPath) {
            let probe = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
            if probe >= 0 {
                defer { Darwin.close(probe) }
                var address = try unixAddress(path: socketPath)
                let connected = withUnsafePointer(to: &address) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        Darwin.connect(probe, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                    }
                }
                if connected == 0 {
                    throw SupervisorError.socketInUse(socketPath)
                }
            }
            guard Darwin.unlink(socketPath) == 0 else {
                throw SupervisorError.system("unlink stale socket", errno)
            }
        }
    }

    private func unixAddress(path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw SupervisorError.socketPathTooLong
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        return address
    }

    private func peerUID(_ fd: Int32) -> uid_t? {
        var credentials = xucred()
        var length = socklen_t(MemoryLayout<xucred>.size)
        let result = withUnsafeMutablePointer(to: &credentials) {
            getsockopt(fd, SOL_LOCAL, LOCAL_PEERCRED, $0, &length)
        }
        return result == 0 ? credentials.cr_uid : nil
    }

    private func writeAll(_ data: Data, to fd: Int32) -> Bool {
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return true }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
                if count <= 0 { return false }
                offset += count
            }
            return true
        }
    }
}

private struct EventSubscriber: Sendable {
    let fd: Int32
    let filter: EventFilter
}

private struct EventListener: Sendable {
    let filter: EventFilter
    let handler: @Sendable (EventEnvelope) -> Void
}

private enum ReconcileRPCError: Error {
    case invalid(String)
}

private extension StackStateRecord {
    var json: JSONValue {
        .object([
            "resources": .array(resources.map(\.json)),
            "journal": journal.map(\.json) ?? .null,
            "lease": lease.map(\.json) ?? .null,
        ])
    }
}

private extension StackResourceRecord {
    var json: JSONValue {
        .object([
            "kind": .string(kind),
            "name": .string(name),
            "labels": .object(labels.mapValues(JSONValue.string)),
            "state": .string(state),
        ])
    }
}

private extension JournalRecord {
    var json: JSONValue {
        .object([
            "id": .string(id),
            "stack_id": .string(stackID),
            "generation": .number(Double(generation)),
            "step": .string(step),
            "status": .string(status),
            "payload": .string(payload),
            "error": error.map(JSONValue.string) ?? .null,
            "created_at": .string(createdAt),
            "updated_at": .string(updatedAt),
        ])
    }
}

private extension LeaseRecord {
    var json: JSONValue {
        .object([
            "holder": .string(holder),
            "expires_at": .string(expiresAtText),
        ])
    }
}

private struct HelperRecord: Sendable {
    let vmID: String
    let state: String
    let pid: Int
    let bundle: String
    let lastError: String?
    let updatedAt: String

    init(vmID: String, state: String, pid: Int, bundle: String, lastError: String?) {
        self.vmID = vmID
        self.state = state
        self.pid = pid
        self.bundle = bundle
        self.lastError = lastError
        updatedAt = ISO8601DateFormatter().string(from: Date())
    }

    init?(params: JSONValue?) {
        guard case let .object(values) = params,
              case let .string(vmID)? = values["vm_id"],
              case let .string(state)? = values["state"],
              case let .number(pid)? = values["pid"],
              case let .string(bundle)? = values["bundle"],
              !vmID.isEmpty,
              !bundle.isEmpty,
              pid.isFinite,
              pid >= 1,
              pid <= Double(Int.max),
              pid.rounded() == pid,
              ["starting", "running", "stopped", "failed"].contains(state)
        else {
            return nil
        }
        let lastError: String?
        if case let .string(error)? = values["error"], !error.isEmpty {
            lastError = error
        } else {
            lastError = nil
        }
        self.init(
            vmID: vmID,
            state: state,
            pid: Int(pid),
            bundle: bundle,
            lastError: lastError
        )
    }

    var json: JSONValue {
        .object([
            "vm_id": .string(vmID),
            "state": .string(state),
            "pid": .number(Double(pid)),
            "bundle": .string(bundle),
            "last_error": lastError.map(JSONValue.string) ?? .null,
            "updated_at": .string(updatedAt),
        ])
    }
}
