import Darwin
import Foundation

/// JSON-RPC client for `vz-net` (`net.sock`). See docs/specs/vz-net-v1.md.
public enum VzNetClientError: Error, CustomStringConvertible, Sendable {
    case unavailable(String)
    case rpc(Int, String)
    case invalidResponse(String)

    public var description: String {
        switch self {
        case let .unavailable(message):
            return message
        case let .rpc(_, message):
            return message
        case let .invalidResponse(message):
            return message
        }
    }

    public var rpcCode: Int {
        switch self {
        case .unavailable, .invalidResponse:
            return -32032
        case let .rpc(code, _):
            return code
        }
    }
}

public struct VzNetNetworkInfo: Equatable, Sendable {
    public var name: String
    public var cidr: String
    public var mode: String
    public var natEgress: Bool
    public var gateway: String

    public init(
        name: String,
        cidr: String,
        mode: String,
        natEgress: Bool,
        gateway: String
    ) {
        self.name = name
        self.cidr = cidr
        self.mode = mode
        self.natEgress = natEgress
        self.gateway = gateway
    }

    public init(json: JSONValue) throws {
        guard case let .object(values) = json,
              case let .string(name)? = values["name"],
              case let .string(cidr)? = values["cidr"],
              case let .string(mode)? = values["mode"],
              case let .bool(natEgress)? = values["nat_egress"],
              case let .string(gateway)? = values["gateway"]
        else {
            throw VzNetClientError.invalidResponse("invalid network object")
        }
        self.init(
            name: name,
            cidr: cidr,
            mode: mode,
            natEgress: natEgress,
            gateway: gateway
        )
    }

    public var json: JSONValue {
        .object([
            "name": .string(name),
            "cidr": .string(cidr),
            "mode": .string(mode),
            "nat_egress": .bool(natEgress),
            "gateway": .string(gateway),
        ])
    }
}

public struct VzNetClient: Sendable {
    public let socketPath: String
    public let timeoutSeconds: Int

    public init(socketPath: String, timeoutSeconds: Int = 20) {
        self.socketPath = socketPath
        self.timeoutSeconds = timeoutSeconds
    }

    public static func defaultSocketPath(stateDirectory: URL) -> String {
        stateDirectory.appendingPathComponent("net.sock").path
    }

    public func health() throws -> (ok: Bool, version: String, networks: Int) {
        let result = try call(method: "health", params: .object([:]))
        guard case let .object(values) = result,
              case let .bool(ok)? = values["ok"],
              case let .string(version)? = values["version"],
              case let .number(networks)? = values["networks"]
        else {
            throw VzNetClientError.invalidResponse("invalid health result")
        }
        return (ok, version, Int(networks))
    }

    public func acquire(
        name: String,
        cidr: String,
        mode: String = "shared",
        natEgress: Bool = true
    ) throws -> VzNetNetworkInfo {
        let result = try call(
            method: "net.acquire",
            params: .object([
                "name": .string(name),
                "cidr": .string(cidr),
                "mode": .string(mode),
                "nat_egress": .bool(natEgress),
            ])
        )
        return try VzNetNetworkInfo(json: result)
    }

    public func release(name: String) throws {
        _ = try call(
            method: "net.release",
            params: .object(["name": .string(name)])
        )
    }

    public func list() throws -> [VzNetNetworkInfo] {
        let result = try call(method: "net.list", params: .object([:]))
        guard case let .object(values) = result,
              case let .array(networks)? = values["networks"]
        else {
            throw VzNetClientError.invalidResponse("invalid list result")
        }
        return try networks.map { try VzNetNetworkInfo(json: $0) }
    }

    public func serialize(name: String) throws -> Data {
        let result = try call(
            method: "net.serialize",
            params: .object(["name": .string(name)])
        )
        guard case let .object(values) = result,
              case let .string(base64)? = values["serialization"]
        else {
            throw VzNetClientError.invalidResponse("invalid serialize result")
        }
        return try VmnetSerialization.blob(fromBase64: base64)
    }

    private func call(method: String, params: JSONValue) throws -> JSONValue {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else {
            throw VzNetClientError.unavailable("vz-net socket: \(errno)")
        }
        defer { Darwin.close(fd) }

        var receiveTimeout = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_RCVTIMEO,
            &receiveTimeout,
            socklen_t(MemoryLayout<timeval>.size)
        )

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw VzNetClientError.unavailable("vz-net socket path is too long")
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
            throw VzNetClientError.unavailable(
                "vz-net is unavailable at \(socketPath)"
            )
        }

        let request = JSONRPCRequest(method: method, params: params, id: .number(1))
        let encoded = try JSONRPCFraming.encode(request)
        guard writeAll(encoded, to: fd), let line = readLine(from: fd) else {
            throw VzNetClientError.unavailable("vz-net did not respond")
        }
        let response = try JSONRPCFraming.decode(JSONRPCResponse.self, from: line)
        if let error = response.error {
            throw VzNetClientError.rpc(error.code, error.message)
        }
        guard let result = response.result else {
            throw VzNetClientError.invalidResponse("missing result")
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
        var buffer = Data()
        var byte: UInt8 = 0
        while true {
            let count = Darwin.read(fd, &byte, 1)
            if count <= 0 { return buffer.isEmpty ? nil : buffer }
            if byte == 0x0A { return buffer }
            buffer.append(byte)
        }
    }
}
