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
    let socketPath: String
    let databasePath: String

    private let stateDirectory: URL
    private let startedAt = ContinuousClock.now
    private let stateLock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false
    private let database: StateDatabase
    private let networkRegistry: NetworkRegistry
    private var helpers: [String: HelperRecord] = [:]
    private var subscribers: [UUID: EventSubscriber] = [:]

    init(stateDirectory: URL) throws {
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
        networkRegistry = try NetworkRegistry(database: database)
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
            return JSONRPCResponse(
                result: .object([
                    "ok": .bool(true),
                    "version": .string(VzDaemonKit.version),
                    "pid": .number(Double(getpid())),
                    "uptime_ms": .number(Double(uptimeMilliseconds)),
                    "db_ok": .bool(true),
                    "networks": .number(Double(networkSnapshot?.networks.count ?? 0)),
                    "network_orphans": .number(Double(orphanedNetworks)),
                ]),
                id: request.id ?? .null
            )
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
        case "net.create":
            do {
                let params = try networkParams(request.params)
                let record = try networkRegistry.create(
                    name: try requiredString("name", from: params),
                    cidr: try requiredString("cidr", from: params),
                    mode: try optionalString("mode", from: params) ?? "shared",
                    labels: try labels(from: params),
                    project: try optionalString("project", from: params),
                    stack: try optionalString("stack", from: params)
                )
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
        case "route.apply":
            do {
                let requestedRouter: String?
                if request.params == nil {
                    requestedRouter = nil
                } else {
                    let params = try networkParams(request.params)
                    requestedRouter = try optionalString("router", from: params)
                }
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
                var results: [JSONValue] = []
                var anyChanged = false
                for router in routers.sorted(by: { $0.vmID < $1.vmID }) {
                    guard try vmHasRouterRole(bundle: router.bundle) else {
                        throw RouteApplyError.invalid(
                            "VM \(router.vmID) does not declare roles: [router]"
                        )
                    }
                    let plan = try RouterPlan(
                        vmID: router.vmID,
                        networkRecords: snapshot.networks,
                        attachments: snapshot.attachments
                    )
                    let changed = try HelperRouteClient.apply(
                        plan,
                        stateDirectory: stateDirectory
                    )
                    anyChanged = anyChanged || changed
                    results.append(
                        .object([
                            "vm_id": .string(plan.vmID),
                            "changed": .bool(changed),
                            "networks": .array(plan.networks.map(\.json)),
                            "forward_policy": .string("drop"),
                        ])
                    )
                    emit(
                        type: "route.applied",
                        data: ["vm_id": .string(plan.vmID), "changed": .bool(changed)]
                    )
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
                ]
            )
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
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
        case "apply.stub":
            guard case let .object(params)? = request.params,
                  case let .string(invocationID)? = params["invocation_id"],
                  case let .string(mode)? = params["mode"],
                  !invocationID.isEmpty,
                  ["apply", "resume", "abort"].contains(mode)
            else {
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32602, message: "Invalid apply event params"),
                    id: request.id ?? .null
                )
            }
            let common: [String: JSONValue] = [
                "invocation_id": .string(invocationID),
                "mode": .string(mode),
            ]
            emit(type: "apply.started", data: common)
            emit(
                type: "apply.step",
                data: common.merging([
                    "step": .string("reconcile"),
                    "status": .string("unavailable"),
                ]) { _, new in new }
            )
            emit(
                type: "apply.failed",
                data: common.merging([
                    "exit_code": .number(12),
                    "error": .string("not_implemented"),
                ]) { _, new in new }
            )
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
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
        stateLock.withLock {
            let failed = subscribers.compactMap { id, subscriber -> UUID? in
                guard subscriber.filter.matches(type) else { return nil }
                return writeAll(encoded, to: subscriber.fd) ? nil : id
            }
            for id in failed {
                subscribers.removeValue(forKey: id)
            }
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

private struct HelperRecord: Sendable {
    let vmID: String
    let state: String
    let pid: Int
    let bundle: String
    let updatedAt: String

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
        self.vmID = vmID
        self.state = state
        self.pid = Int(pid)
        self.bundle = bundle
        updatedAt = ISO8601DateFormatter().string(from: Date())
    }

    var json: JSONValue {
        .object([
            "vm_id": .string(vmID),
            "state": .string(state),
            "pid": .number(Double(pid)),
            "bundle": .string(bundle),
            "updated_at": .string(updatedAt),
        ])
    }
}
