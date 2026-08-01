import Darwin
import Foundation

/// Privileged UDP bind helper protocol (SCM_RIGHTS over UDS).
public enum DnsBind {
    public static let defaultSocketPath = "/var/run/vzctl/dns-bind.sock"
    public static let label = "com.vzctl.dns-bind"
    public static let libexecBinary = "/usr/local/libexec/vzctl/vz-dns-bind"
    public static let launchDaemonPlist = "/Library/LaunchDaemons/com.vzctl.dns-bind.plist"
    public static let privilegedPortLimit: UInt16 = 1024

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

        public init(address: String, port: UInt16) {
            self.op = "bind"
            self.address = address
            self.port = port
        }
    }

    public struct BindResponse: Codable, Equatable, Sendable {
        public var ok: Bool
        public var error: String?

        public init(ok: Bool, error: String? = nil) {
            self.ok = ok
            self.error = error
        }
    }

    public enum ValidationError: Error, CustomStringConvertible, Equatable {
        case invalidJSON
        case unsupportedOp(String)
        case invalidAddress(String)
        case portNotPrivileged(UInt16)
        case portInvalid

        public var description: String {
            switch self {
            case .invalidJSON:
                return "invalid bind request JSON"
            case let .unsupportedOp(op):
                return "unsupported dns-bind op: \(op)"
            case let .invalidAddress(address):
                return "invalid IPv4 address: \(address)"
            case let .portNotPrivileged(port):
                return "port \(port) is not privileged (< \(privilegedPortLimit))"
            case .portInvalid:
                return "port must be > 0"
            }
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
        var addr = in_addr()
        guard inet_pton(AF_INET, request.address, &addr) == 1 else {
            throw ValidationError.invalidAddress(request.address)
        }
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
