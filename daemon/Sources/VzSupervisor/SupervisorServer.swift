import Darwin
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
        "vm.agent.stats",
        "vm.agent.services.list",
        "vm.agent.services.http",
        "vm.agent.services.stream",
        "vm.agent.systemd.status",
        "vm.agent.systemd.list",
        "vm.agent.systemd.show",
        "vm.agent.systemd.control",
        "vm.agent.systemd.events",
    ]

    let socketPath: String
    let databasePath: String

    let stateDirectory: URL
    private let startedAt = ContinuousClock.now
    let stateLock = NSLock()
    let edgeReconcileLock = NSLock()
    var listener: Int32 = -1
    private var ownsSocket = false
    let database: StateDatabase
    let networkRegistry: NetworkRegistry
    let edgeClient: VzEdgeClient
    var helpers: [String: HelperRecord] = [:]
    var helperProcesses: [String: Process] = [:]
    var helperStartsInFlight: Set<String> = []
    var helperStartupTimeout: TimeInterval = 5
    var expectedHelperStops: Set<String> = []
    var subscribers: [UUID: EventSubscriber] = [:]
    var eventListeners: [UUID: EventListener] = [:]
    private var restServer: RestServer?
    private var networkResilience: NetworkResilienceController?
    private var systemdEvents: SystemdEventBridge?
    private(set) var controllerReconciler: ControllerReconciler!
    var lastIngressWatchdogReconcile = Date.distantPast
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
            socketPath: VzEdgeClient.defaultSocketPath(stateDirectory: stateDirectory),
            timeoutSeconds: 90
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
        controllerReconciler = ControllerReconciler(stateDirectory: stateDirectory) { [weak self] type, data in self?.emit(type: type, data: data) }
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

            let systemdBridge = SystemdEventBridge(
                stateDirectory: stateDirectory,
                runningVMs: { [unowned self] in
                    self.stateLock.withLock {
                        self.helpers.values
                            .filter { $0.state == "running" }
                            .map(\.vmID)
                    }
                },
                hasSubscriber: { [unowned self] in
                    self.stateLock.withLock {
                        self.subscribers.values.contains {
                            $0.filter.matches("vm.systemd.unit")
                        }
                    }
                },
                emit: { [unowned self] type, data in self.emit(type: type, data: data) }
            )
            systemdEvents = systemdBridge
            systemdBridge.start()
            controllerReconciler.start()

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
        controllerReconciler.stop()
        networkResilience?.stop()
        networkResilience = nil
        systemdEvents?.stop()
        systemdEvents = nil
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

        return handleHealthRPC(request)
            ?? handleVMRPC(request)
            ?? handleNetworkRPC(request)
            ?? handleEdgeRPC(request)
            ?? handleApplyRPC(request)
            ?? handleControllerRPC(request)
            ?? JSONRPCResponse(
                error: JSONRPCError(code: -32601, message: "Method not found"),
                id: request.id ?? .null
            )
    }

    func handleHealthRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
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
        case "daemon.version":
            return JSONRPCResponse(result: .string(VzDaemonKit.version), id: request.id ?? .null)
        default: return nil
        }
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

    func writeAll(_ data: Data, to fd: Int32) -> Bool {
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
