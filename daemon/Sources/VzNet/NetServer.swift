import Darwin
import Foundation
import VzDaemonKit

enum NetServerError: Error, CustomStringConvertible {
    case system(String, Int32)
    case socketInUse(String)
    case socketPathTooLong

    var description: String {
        switch self {
        case let .system(operation, code):
            return "\(operation): \(String(cString: strerror(code)))"
        case let .socketInUse(path):
            return "vz-net already listens at \(path)"
        case .socketPathTooLong:
            return "Unix socket path is too long"
        }
    }
}

final class NetServer: @unchecked Sendable {
    let socketPath: String
    private let store = NetRuntimeStore()
    private let stateLock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false

    init(stateDirectory: URL) throws {
        try FileManager.default.createDirectory(
            at: stateDirectory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        guard chmod(stateDirectory.path, 0o700) == 0 else {
            throw NetServerError.system("chmod state directory", errno)
        }
        socketPath = stateDirectory.appendingPathComponent("net.sock").path
    }

    func run() throws {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw NetServerError.system("socket", errno) }
        stateLock.withLock { listener = fd }
        do {
            try prepareSocketPath()
            var address = try unixAddress(path: socketPath)
            let bindResult = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard bindResult == 0 else { throw NetServerError.system("bind", errno) }
            stateLock.withLock { ownsSocket = true }
            guard chmod(socketPath, 0o600) == 0 else {
                throw NetServerError.system("chmod", errno)
            }
            guard Darwin.listen(fd, 16) == 0 else {
                throw NetServerError.system("listen", errno)
            }
            while true {
                let client = Darwin.accept(fd, nil, nil)
                if client < 0 {
                    if errno == EINTR { continue }
                    let current = stateLock.withLock { listener }
                    if current < 0 { break }
                    throw NetServerError.system("accept", errno)
                }
                DispatchQueue.global().async { [weak self] in
                    self?.handle(client)
                }
            }
        } catch {
            stop()
            throw error
        }
    }

    func stop() {
        store.shutdown()
        let fd = stateLock.withLock { () -> Int32 in
            let current = listener
            listener = -1
            return current
        }
        if fd >= 0 {
            Darwin.close(fd)
        }
        if stateLock.withLock({ ownsSocket }) {
            unlink(socketPath)
            stateLock.withLock { ownsSocket = false }
        }
    }

    private func handle(_ client: Int32) {
        defer { Darwin.close(client) }
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = Darwin.read(client, &chunk, chunk.count)
            if count <= 0 { return }
            buffer.append(contentsOf: chunk.prefix(count))
            while let newline = buffer.firstIndex(of: 0x0A) {
                let line = Data(buffer[..<newline])
                buffer.removeSubrange(...newline)
                if line.isEmpty { continue }
                let response = response(for: line)
                guard let encoded = try? JSONRPCFraming.encode(response) else { return }
                guard writeAll(encoded, to: client) else { return }
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
        guard request.jsonrpc == "2.0", !request.method.isEmpty else {
            return JSONRPCResponse(
                error: JSONRPCError(code: -32600, message: "Invalid Request"),
                id: request.id ?? .null
            )
        }
        return dispatch(request)
    }

    private func dispatch(_ request: JSONRPCRequest) -> JSONRPCResponse {
        let id = request.id ?? .null
        do {
            switch request.method {
            case "health":
                return JSONRPCResponse(
                    result: .object([
                        "ok": .bool(true),
                        "version": .string(VzDaemonKit.version),
                        "networks": .number(Double(store.networkCount())),
                    ]),
                    id: id
                )
            case "net.acquire":
                let params = try objectParams(request.params)
                guard case let .string(name)? = params["name"],
                      case let .string(cidr)? = params["cidr"]
                else {
                    throw NetRuntimeError.invalid("name and cidr are required")
                }
                let mode: String
                if case let .string(value)? = params["mode"] {
                    mode = value
                } else {
                    mode = "shared"
                }
                let natEgress: Bool
                if case let .bool(value)? = params["nat_egress"] {
                    natEgress = value
                } else {
                    natEgress = true
                }
                let info = try store.acquire(
                    name: name,
                    cidr: cidr,
                    mode: mode,
                    natEgress: natEgress
                )
                return JSONRPCResponse(result: info.json, id: id)
            case "net.release":
                let params = try objectParams(request.params)
                guard case let .string(name)? = params["name"] else {
                    throw NetRuntimeError.invalid("name is required")
                }
                let info = try store.release(name: name)
                return JSONRPCResponse(
                    result: .object([
                        "released": .bool(true),
                        "name": .string(info.name),
                    ]),
                    id: id
                )
            case "net.list":
                let networks = try store.list()
                return JSONRPCResponse(
                    result: .object([
                        "networks": .array(networks.map(\.json)),
                    ]),
                    id: id
                )
            case "net.serialize":
                let params = try objectParams(request.params)
                guard case let .string(name)? = params["name"] else {
                    throw NetRuntimeError.invalid("name is required")
                }
                let blob = try store.serialize(name: name)
                return JSONRPCResponse(
                    result: .object([
                        "name": .string(name),
                        "serialization": .string(VmnetSerialization.base64(from: blob)),
                    ]),
                    id: id
                )
            case "net.verify":
                return JSONRPCResponse(
                    result: .object(["networks": .array(try store.verify())]),
                    id: id
                )
            default:
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32601, message: "Method not found"),
                    id: id
                )
            }
        } catch let error as NetRuntimeError {
            return JSONRPCResponse(
                error: JSONRPCError(code: error.rpcCode, message: error.description),
                id: id
            )
        } catch {
            return JSONRPCResponse(
                error: JSONRPCError(code: -32032, message: String(describing: error)),
                id: id
            )
        }
    }

    private func objectParams(_ params: JSONValue?) throws -> [String: JSONValue] {
        guard let params else { return [:] }
        guard case let .object(values) = params else {
            throw NetRuntimeError.invalid("params must be an object")
        }
        return values
    }

    private func prepareSocketPath() throws {
        if FileManager.default.fileExists(atPath: socketPath) {
            var probe = try unixAddress(path: socketPath)
            let probeFd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
            if probeFd >= 0 {
                defer { Darwin.close(probeFd) }
                let connected = withUnsafePointer(to: &probe) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        Darwin.connect(probeFd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                    }
                }
                if connected == 0 {
                    throw NetServerError.socketInUse(socketPath)
                }
            }
            unlink(socketPath)
        }
    }

    private func unixAddress(path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw NetServerError.socketPathTooLong
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        return address
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
