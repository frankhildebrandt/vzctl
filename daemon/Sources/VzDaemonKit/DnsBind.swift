import Darwin
import Foundation

/// Privileged bind helper protocol (SCM_RIGHTS over UDS).
/// Binds UDP (guest DNS :53) or TCP (ingress gateway :80/:443) on privileged ports.
public enum DnsBind {
    public static let defaultSocketPath = "/var/run/vzctl/dns-bind.sock"
    public static let label = "com.vzctl.dns-bind"
    public static let libexecBinary = "/usr/local/libexec/vzctl/vz-dns-bind"
    public static let launchDaemonPlist = "/Library/LaunchDaemons/com.vzctl.dns-bind.plist"
    public static let privilegedPortLimit: UInt16 = 1024
    public static let protoUDP = "udp"
    public static let protoTCP = "tcp"
    public static let opAliasEnsure = "alias.ensure"
    public static let opAliasRemove = "alias.remove"
    public static let opFirewallReconcile = "firewall.reconcile"
    public static let opCleanup = "cleanup"

    public static func socketPath(
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) -> String {
        environment["VZCTL_DNS_BIND_SOCK"] ?? defaultSocketPath
    }

    public static func needsPrivilege(port: UInt16) -> Bool {
        port > 0 && port < privilegedPortLimit
    }

    public struct BindRequest: Codable, Equatable, Sendable {
        public var op: String
        public var address: String
        public var port: UInt16
        /// `"udp"` (default) or `"tcp"`.
        public var proto: String

        public init(address: String, port: UInt16, proto: String = DnsBind.protoUDP) {
            self.op = "bind"
            self.address = address
            self.port = port
            self.proto = proto
        }

        enum CodingKeys: String, CodingKey {
            case op, address, port, proto
        }

        public init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            op = try container.decode(String.self, forKey: .op)
            address = try container.decode(String.self, forKey: .address)
            port = try container.decode(UInt16.self, forKey: .port)
            proto = try container.decodeIfPresent(String.self, forKey: .proto) ?? DnsBind.protoUDP
        }
    }

    public struct BindResponse: Codable, Equatable, Sendable {
        public var ok: Bool
        public var error: String?
        /// TCP stream: `"listening"` then `"accept"` (with SCM_RIGHTS client FD).
        public var event: String?

        public init(ok: Bool, error: String? = nil, event: String? = nil) {
            self.ok = ok
            self.error = error
            self.event = event
        }
    }

    public struct AliasRequest: Codable, Equatable, Sendable {
        public var op: String
        public var cidr: String

        public init(op: String, cidr: String) {
            self.op = op
            self.cidr = cidr
        }
    }

    public struct FirewallBinding: Codable, Equatable, Sendable {
        public var cidr: String
        public var allowedSources: [String]
        public var tcpPorts: [UInt16]

        public init(cidr: String, allowedSources: [String], tcpPorts: [UInt16]) {
            self.cidr = cidr
            self.allowedSources = allowedSources
            self.tcpPorts = tcpPorts
        }

        enum CodingKeys: String, CodingKey {
            case cidr
            case allowedSources = "allowed_sources"
            case tcpPorts = "tcp_ports"
        }
    }

    public struct FirewallRequest: Codable, Equatable, Sendable {
        public var op: String
        public var bindings: [FirewallBinding]

        public init(bindings: [FirewallBinding]) {
            op = DnsBind.opFirewallReconcile
            self.bindings = bindings
        }
    }

    public struct CleanupRequest: Codable, Equatable, Sendable {
        public var op: String

        public init() {
            op = DnsBind.opCleanup
        }
    }

    public enum Request: Equatable, Sendable {
        case bind(BindRequest)
        case alias(AliasRequest)
        case firewall(FirewallRequest)
        case cleanup(CleanupRequest)
    }

    public enum ValidationError: Error, CustomStringConvertible, Equatable {
        case invalidJSON
        case unsupportedOp(String)
        case invalidAddress(String)
        case invalidProto(String)
        case portNotPrivileged(UInt16)
        case portInvalid
        case invalidCIDR(String)
        case invalidFirewall(String)

        public var description: String {
            switch self {
            case .invalidJSON:
                return "invalid bind request JSON"
            case let .unsupportedOp(op):
                return "unsupported dns-bind op: \(op)"
            case let .invalidAddress(address):
                return "invalid IPv4 address: \(address)"
            case let .invalidProto(proto):
                return "invalid proto \(proto); use udp or tcp"
            case let .portNotPrivileged(port):
                return "port \(port) is not privileged (< \(privilegedPortLimit))"
            case .portInvalid:
                return "port must be > 0"
            case let .invalidCIDR(cidr):
                return "invalid canonical IPv4 CIDR: \(cidr)"
            case let .invalidFirewall(message):
                return "invalid firewall request: \(message)"
            }
        }
    }

    private struct Operation: Decodable {
        let op: String
    }

    public static func parseOperation(_ data: Data) throws -> Request {
        let trimmed = data.trimmingASCIINewlines()
        guard let operation = try? JSONDecoder().decode(Operation.self, from: trimmed) else {
            throw ValidationError.invalidJSON
        }
        switch operation.op {
        case "bind":
            return .bind(try parseRequest(trimmed))
        case opAliasEnsure, opAliasRemove:
            guard let request = try? JSONDecoder().decode(AliasRequest.self, from: trimmed) else {
                throw ValidationError.invalidJSON
            }
            try validate(request)
            return .alias(request)
        case opFirewallReconcile:
            guard let request = try? JSONDecoder().decode(FirewallRequest.self, from: trimmed) else {
                throw ValidationError.invalidJSON
            }
            try validate(request)
            return .firewall(request)
        case opCleanup:
            guard let request = try? JSONDecoder().decode(CleanupRequest.self, from: trimmed) else {
                throw ValidationError.invalidJSON
            }
            return .cleanup(request)
        default:
            throw ValidationError.unsupportedOp(operation.op)
        }
    }

    public static func parseRequest(_ data: Data) throws -> BindRequest {
        let trimmed = data.trimmingASCIINewlines()
        guard let request = try? JSONDecoder().decode(BindRequest.self, from: trimmed) else {
            throw ValidationError.invalidJSON
        }
        try validate(request)
        return request
    }

    public static func validate(_ request: BindRequest) throws {
        guard request.op == "bind" else {
            throw ValidationError.unsupportedOp(request.op)
        }
        guard request.port > 0 else {
            throw ValidationError.portInvalid
        }
        guard needsPrivilege(port: request.port) else {
            throw ValidationError.portNotPrivileged(request.port)
        }
        let proto = request.proto.lowercased()
        guard proto == protoUDP || proto == protoTCP else {
            throw ValidationError.invalidProto(request.proto)
        }
        var addr = in_addr()
        guard inet_pton(AF_INET, request.address, &addr) == 1 else {
            throw ValidationError.invalidAddress(request.address)
        }
    }

    public static func validate(_ request: AliasRequest) throws {
        guard request.op == opAliasEnsure || request.op == opAliasRemove else {
            throw ValidationError.unsupportedOp(request.op)
        }
        guard (try? IPv4CIDR(request.cidr))?.canonical == request.cidr else {
            throw ValidationError.invalidCIDR(request.cidr)
        }
    }

    public static func validate(_ request: FirewallRequest) throws {
        guard request.op == opFirewallReconcile else {
            throw ValidationError.unsupportedOp(request.op)
        }
        var seen = Set<String>()
        for binding in request.bindings {
            guard (try? IPv4CIDR(binding.cidr))?.canonical == binding.cidr else {
                throw ValidationError.invalidCIDR(binding.cidr)
            }
            guard seen.insert(binding.cidr).inserted else {
                throw ValidationError.invalidFirewall("duplicate binding \(binding.cidr)")
            }
            guard !binding.tcpPorts.contains(0) else {
                throw ValidationError.invalidFirewall("TCP port must be greater than zero")
            }
            for source in binding.allowedSources {
                guard (try? IPv4CIDR(source))?.canonical == source else {
                    throw ValidationError.invalidCIDR(source)
                }
            }
        }
    }

    public static func firewallRules(
        bindings: [FirewallBinding],
        interfaceByCIDR: [String: String]
    ) throws -> String {
        try validate(FirewallRequest(bindings: bindings))
        var rules: [String] = []
        for binding in bindings.sorted(by: { $0.cidr < $1.cidr }) {
            guard let interface = interfaceByCIDR[binding.cidr],
                  !interface.isEmpty,
                  interface.allSatisfy({ $0.isLetter || $0.isNumber })
            else {
                throw ValidationError.invalidFirewall(
                    "missing or invalid interface for \(binding.cidr)"
                )
            }
            let hostService = IPv4CIDR.hostService(for: binding.cidr)
            let sources = Array(Set(binding.allowedSources)).sorted()
            let ports = Array(Set(binding.tcpPorts)).sorted()
            if !sources.isEmpty, !ports.isEmpty {
                rules.append(
                    "pass in quick on \(interface) inet proto tcp from { \(sources.joined(separator: ", ")) } to \(hostService) port { \(ports.map(String.init).joined(separator: ", ")) } flags S/SA keep state"
                )
            }
            rules.append("block in quick on \(interface) inet from any to \(hostService)")
        }
        return rules.isEmpty ? "" : rules.joined(separator: "\n") + "\n"
    }
}

public enum UnixFDPassing {
    public enum Error: Swift.Error, CustomStringConvertible {
        case system(String, Int32)
        case truncated
        case missingFD

        public var description: String {
            switch self {
            case let .system(operation, code):
                return "\(operation): \(String(cString: strerror(code)))"
            case .truncated:
                return "SCM_RIGHTS message truncated"
            case .missingFD:
                return "SCM_RIGHTS file descriptor missing"
            }
        }
    }

    /// Send `payload` and optionally one file descriptor over a connected Unix socket.
    public static func send(payload: Data, fileDescriptor: Int32?, on socket: Int32) throws {
        try payload.withUnsafeBytes { rawBuffer in
            guard let base = rawBuffer.baseAddress else {
                throw Error.system("sendmsg", EINVAL)
            }
            var iov = iovec(
                iov_base: UnsafeMutableRawPointer(mutating: base),
                iov_len: rawBuffer.count
            )
            if let fileDescriptor {
                let fd = fileDescriptor
                let controlLength = socklen_t(MemoryLayout<cmsghdr>.stride + MemoryLayout<Int32>.stride)
                let control = UnsafeMutableRawPointer.allocate(
                    byteCount: Int(controlLength),
                    alignment: MemoryLayout<cmsghdr>.alignment
                )
                defer { control.deallocate() }
                control.initializeMemory(as: UInt8.self, repeating: 0, count: Int(controlLength))
                let header = control.bindMemory(to: cmsghdr.self, capacity: 1)
                header.pointee.cmsg_len = controlLength
                header.pointee.cmsg_level = SOL_SOCKET
                header.pointee.cmsg_type = SCM_RIGHTS
                control
                    .advanced(by: MemoryLayout<cmsghdr>.stride)
                    .bindMemory(to: Int32.self, capacity: 1)
                    .pointee = fd

                var message = msghdr()
                message.msg_iov = withUnsafeMutablePointer(to: &iov) { $0 }
                message.msg_iovlen = 1
                message.msg_control = control
                message.msg_controllen = controlLength
                let sent = withUnsafePointer(to: &message) { Darwin.sendmsg(socket, $0, 0) }
                guard sent >= 0 else { throw Error.system("sendmsg", errno) }
            } else {
                var message = msghdr()
                message.msg_iov = withUnsafeMutablePointer(to: &iov) { $0 }
                message.msg_iovlen = 1
                let sent = withUnsafePointer(to: &message) { Darwin.sendmsg(socket, $0, 0) }
                guard sent >= 0 else { throw Error.system("sendmsg", errno) }
            }
        }
    }

    /// Receive payload and at most one SCM_RIGHTS file descriptor.
    public static func receive(
        on socket: Int32,
        maxPayload: Int = 4096
    ) throws -> (payload: Data, fileDescriptor: Int32?) {
        var buffer = [UInt8](repeating: 0, count: maxPayload)
        let controlLength = socklen_t(MemoryLayout<cmsghdr>.stride + MemoryLayout<Int32>.stride)
        let control = UnsafeMutableRawPointer.allocate(
            byteCount: Int(controlLength),
            alignment: MemoryLayout<cmsghdr>.alignment
        )
        defer { control.deallocate() }
        control.initializeMemory(as: UInt8.self, repeating: 0, count: Int(controlLength))

        let count: Int = buffer.withUnsafeMutableBytes { rawBuffer in
            var iov = iovec(iov_base: rawBuffer.baseAddress, iov_len: rawBuffer.count)
            var message = msghdr()
            message.msg_iov = withUnsafeMutablePointer(to: &iov) { $0 }
            message.msg_iovlen = 1
            message.msg_control = control
            message.msg_controllen = controlLength
            return withUnsafeMutablePointer(to: &message) { pointer in
                let received = Darwin.recvmsg(socket, pointer, 0)
                if received < 0 { return -1 }
                if pointer.pointee.msg_flags & MSG_CTRUNC != 0 {
                    errno = EMSGSIZE
                    return -2
                }
                return received
            }
        }
        if count == -1 { throw Error.system("recvmsg", errno) }
        if count == -2 { throw Error.truncated }
        guard count >= 0 else { throw Error.system("recvmsg", errno) }

        let payload = Data(buffer.prefix(count))
        var receivedFD: Int32?
        var cursor = control
        let end = control.advanced(by: Int(controlLength))
        while cursor < end {
            let header = cursor.bindMemory(to: cmsghdr.self, capacity: 1).pointee
            if header.cmsg_len == 0 { break }
            if header.cmsg_level == SOL_SOCKET, header.cmsg_type == SCM_RIGHTS {
                let fdPointer = cursor
                    .advanced(by: MemoryLayout<cmsghdr>.stride)
                    .bindMemory(to: Int32.self, capacity: 1)
                receivedFD = fdPointer.pointee
                break
            }
            let aligned = Int(header.cmsg_len)
            let padded = (aligned + MemoryLayout<cmsghdr>.alignment - 1)
                & ~(MemoryLayout<cmsghdr>.alignment - 1)
            cursor = cursor.advanced(by: max(padded, MemoryLayout<cmsghdr>.stride))
        }
        return (payload, receivedFD)
    }
}

private extension Data {
    func trimmingASCIINewlines() -> Data {
        var start = startIndex
        var end = endIndex
        while start < end, self[start] == 0x0A || self[start] == 0x0D || self[start] == 0x20 {
            start += 1
        }
        while end > start {
            let previous = index(before: end)
            if self[previous] == 0x0A || self[previous] == 0x0D || self[previous] == 0x20 {
                end = previous
            } else {
                break
            }
        }
        return self[start..<end]
    }
}
