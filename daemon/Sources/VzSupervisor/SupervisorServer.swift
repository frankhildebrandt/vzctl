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

    private let startedAt = ContinuousClock.now
    private let stateLock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false
    private let database: StateDatabase
    private var helpers: [String: HelperRecord] = [:]
    private var subscribers: [UUID: EventSubscriber] = [:]

    init(stateDirectory: URL) throws {
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
            return JSONRPCResponse(
                result: .object([
                    "ok": .bool(true),
                    "version": .string(VzDaemonKit.version),
                    "pid": .number(Double(getpid())),
                    "uptime_ms": .number(Double(uptimeMilliseconds)),
                    "db_ok": .bool(true),
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
