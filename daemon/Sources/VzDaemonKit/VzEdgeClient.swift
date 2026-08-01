import Darwin
import Foundation

public enum VzEdgeClientError: Error, CustomStringConvertible, Sendable {
    case unavailable(String)
    case rpc(Int, String)
    case invalidResponse(String)

    public var description: String {
        switch self {
        case let .unavailable(message), let .invalidResponse(message): return message
        case let .rpc(_, message): return message
        }
    }
}

public struct VzEdgeClient: Sendable {
    public let socketPath: String
    public let timeoutSeconds: Int

    public init(socketPath: String, timeoutSeconds: Int = 20) {
        self.socketPath = socketPath
        self.timeoutSeconds = timeoutSeconds
    }

    public static func defaultSocketPath(stateDirectory: URL) -> String {
        stateDirectory.appendingPathComponent("edge.sock").path
    }

    public func health() throws -> JSONValue {
        try call(method: "edge.health", params: .object([:]))
    }

    public func status() throws -> JSONValue {
        try call(method: "edge.status", params: .object([:]))
    }

    public func lookup(name: String) throws -> JSONValue {
        try call(method: "dns.lookup", params: .object(["name": .string(name)]))
    }

    public func reconcile(generation: Int64, digest: String, desired: JSONValue) throws -> JSONValue {
        try call(
            method: "edge.reconcile",
            params: .object([
                "generation": .number(Double(generation)),
                "digest": .string(digest),
                "desired": desired,
            ])
        )
    }

    private func call(method: String, params: JSONValue) throws -> JSONValue {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw VzEdgeClientError.unavailable("vz-edge socket: \(errno)") }
        defer { Darwin.close(fd) }

        var receiveTimeout = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &receiveTimeout,
                   socklen_t(MemoryLayout<timeval>.size))

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw VzEdgeClientError.unavailable("vz-edge socket path is too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            throw VzEdgeClientError.unavailable("vz-edge is unavailable at \(socketPath)")
        }

        let request = JSONRPCRequest(method: method, params: params, id: .number(1))
        let encoded = try JSONRPCFraming.encode(request)
        guard writeAll(encoded, to: fd), let line = readLine(from: fd) else {
            throw VzEdgeClientError.unavailable("vz-edge did not respond")
        }
        let response = try JSONRPCFraming.decode(JSONRPCResponse.self, from: line)
        if let error = response.error { throw VzEdgeClientError.rpc(error.code, error.message) }
        guard let result = response.result else {
            throw VzEdgeClientError.invalidResponse("vz-edge response has no result")
        }
        return result
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

    private func readLine(from fd: Int32) -> Data? {
        var data = Data()
        var byte: UInt8 = 0
        while true {
            let count = Darwin.read(fd, &byte, 1)
            if count <= 0 { return data.isEmpty ? nil : data }
            if byte == 0x0A { return data }
            data.append(byte)
        }
    }
}
