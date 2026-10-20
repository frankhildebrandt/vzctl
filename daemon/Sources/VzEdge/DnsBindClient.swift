import Darwin
import Foundation
import VzDaemonKit

enum DnsBindClient {
    enum Error: Swift.Error, CustomStringConvertible {
        case connect(String)
        case response(String)
        case helper(String)
        case system(String, Int32)

        var description: String {
            switch self {
            case let .connect(message):
                return "dns-bind connect: \(message)"
            case let .response(message):
                return "dns-bind response: \(message)"
            case let .helper(message):
                return "dns-bind: \(message)"
            case let .system(operation, code):
                return "dns-bind \(operation): \(String(cString: strerror(code)))"
            }
        }
    }

    private static let reconcileTimeoutSeconds = 5

    /// Ask the privileged helper to bind `address:port` (UDP) and return the FD.
    /// Caller owns the returned descriptor.
    static func bindUDP(
        address: String,
        port: UInt16,
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws -> Int32 {
        try withRetry {
            let client = try connect(
                socketPath: socketPath,
                timeoutSeconds: timeoutSeconds
            )
            defer { Darwin.close(client) }
            try sendRequest(
                DnsBind.BindRequest(address: address, port: port, proto: DnsBind.protoUDP),
                on: client
            )
            let (responseData, fileDescriptor) = try UnixFDPassing.receive(on: client)
            var transferred = false
            defer { if !transferred, let fileDescriptor { Darwin.close(fileDescriptor) } }
            let response = try decodeResponse(responseData)
            guard response.ok else {
                throw Error.helper(response.error ?? "bind failed")
            }
            guard let fileDescriptor else {
                throw Error.response("missing file descriptor")
            }
            transferred = true
            return fileDescriptor
        }
    }

    static func ensureAlias(
        cidr: String,
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws {
        let request = DnsBind.AliasRequest(op: DnsBind.opAliasEnsure, cidr: cidr)
        try DnsBind.validate(request)
        try withRetry {
            try simpleRequest(request, socketPath: socketPath, timeoutSeconds: timeoutSeconds)
        }
    }

    static func removeAlias(
        cidr: String,
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws {
        let request = DnsBind.AliasRequest(op: DnsBind.opAliasRemove, cidr: cidr)
        try DnsBind.validate(request)
        try simpleRequest(request, socketPath: socketPath, timeoutSeconds: timeoutSeconds)
    }

    static func reconcileFirewall(
        _ bindings: [DnsBind.FirewallBinding],
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws {
        let request = DnsBind.FirewallRequest(bindings: bindings)
        try DnsBind.validate(request)
        try withRetry {
            try simpleRequest(request, socketPath: socketPath, timeoutSeconds: timeoutSeconds)
        }
    }

    static func cleanupNetworking(
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws {
        try simpleRequest(
            DnsBind.CleanupRequest(), socketPath: socketPath,
            timeoutSeconds: timeoutSeconds
        )
    }

    /// Open a privileged TCP accept stream: helper listens and forwards accepted clients.
    static func openTCPAcceptStream(
        address: String,
        port: UInt16,
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = reconcileTimeoutSeconds
    ) throws -> TCPAcceptStream {
        try withRetry {
            let client = try connect(
                socketPath: socketPath,
                timeoutSeconds: timeoutSeconds
            )
            do {
                try sendRequest(
                    DnsBind.BindRequest(address: address, port: port, proto: DnsBind.protoTCP),
                    on: client
                )
                let (responseData, fd) = try UnixFDPassing.receive(on: client)
                if let fd { Darwin.close(fd) }
                let response = try decodeResponse(responseData)
                guard response.ok else {
                    throw Error.helper(response.error ?? "bind failed")
                }
                guard response.event == "listening" else {
                    throw Error.response("expected listening event, got \(response.event ?? "nil")")
                }
                var clear = timeval(tv_sec: 0, tv_usec: 0)
                setsockopt(
                    client,
                    SOL_SOCKET,
                    SO_RCVTIMEO,
                    &clear,
                    socklen_t(MemoryLayout<timeval>.size)
                )
                setsockopt(
                    client,
                    SOL_SOCKET,
                    SO_SNDTIMEO,
                    &clear,
                    socklen_t(MemoryLayout<timeval>.size)
                )
                return TCPAcceptStream(socket: client)
            } catch {
                Darwin.close(client)
                throw error
            }
        }
    }

    /// Long-lived UDS session that yields accepted client FDs from the helper.
    final class TCPAcceptStream: @unchecked Sendable {
        private let socket: Int32
        private let lock = NSLock()
        private var closed = false

        init(socket: Int32) {
            self.socket = socket
        }

        /// Blocks until the helper accepts a TCP client. Returns owned client FD.
        func accept() throws -> Int32 {
            lock.lock()
            if closed {
                lock.unlock()
                throw Error.response("accept stream closed")
            }
            let socket = self.socket
            lock.unlock()

            let (responseData, fileDescriptor) = try UnixFDPassing.receive(on: socket)
            var transferred = false
            defer { if !transferred, let fileDescriptor { Darwin.close(fileDescriptor) } }
            let response = try decodeResponse(responseData)
            guard response.ok else {
                throw Error.helper(response.error ?? "accept failed")
            }
            guard response.event == "accept" else {
                throw Error.response("expected accept event, got \(response.event ?? "nil")")
            }
            guard let fileDescriptor else {
                throw Error.response("missing accepted file descriptor")
            }
            transferred = true
            return fileDescriptor
        }

        func close() {
            lock.lock()
            defer { lock.unlock() }
            guard !closed else { return }
            closed = true
            Darwin.shutdown(socket, SHUT_RDWR)
            Darwin.close(socket)
        }

        deinit {
            close()
        }
    }

    private static func isTransient(_ error: Error) -> Bool {
        switch error {
        case let .connect(message):
            return message == "Resource temporarily unavailable"
                || message.contains("Connection refused")
                || message.contains("Connection reset")
        case let .system(_, code):
            return code == EAGAIN
                || code == EINTR
                || code == EWOULDBLOCK
                || code == ETIMEDOUT
                || code == ECONNREFUSED
        case let .helper(message):
            return message.contains("Resource temporarily unavailable")
                || message.contains("Address already in use")
        default:
            return false
        }
    }

    private static func withRetry<T>(
        attempts: Int = 5,
        initialDelay: TimeInterval = 0.1,
        _ body: () throws -> T
    ) throws -> T {
        var lastError: Swift.Error?
        for attempt in 0 ..< attempts {
            do {
                return try body()
            } catch {
                lastError = error
                if let bindError = error as? Error,
                   isTransient(bindError),
                   attempt < attempts - 1 {
                    Thread.sleep(forTimeInterval: initialDelay + Double(attempt) * 0.1)
                    continue
                }
                throw error
            }
        }
        throw lastError ?? Error.response("dns-bind retry exhausted")
    }

    private static func connect(socketPath: String, timeoutSeconds: Int) throws -> Int32 {
        let client = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard client >= 0 else { throw Error.system("socket", errno) }

        var timeval = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(
            client,
            SOL_SOCKET,
            SO_RCVTIMEO,
            &timeval,
            socklen_t(MemoryLayout<timeval>.size)
        )
        setsockopt(
            client,
            SOL_SOCKET,
            SO_SNDTIMEO,
            &timeval,
            socklen_t(MemoryLayout<timeval>.size)
        )

        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = socketPath.utf8CString
        guard pathBytes.count <= MemoryLayout.size(ofValue: addr.sun_path) else {
            Darwin.close(client)
            throw Error.connect("socket path too long")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { buffer in
            pathBytes.withUnsafeBytes { source in
                buffer.copyMemory(from: source)
            }
        }
        let connected = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(client, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else {
            let code = errno
            Darwin.close(client)
            throw Error.connect(String(cString: strerror(code)))
        }
        return client
    }

    private static func sendRequest(_ request: DnsBind.BindRequest, on client: Int32) throws {
        try DnsBind.validate(request)
        try sendEncodable(request, on: client)
    }

    private static func simpleRequest<T: Encodable>(
        _ request: T,
        socketPath: String,
        timeoutSeconds: Int
    ) throws {
        let client = try connect(socketPath: socketPath, timeoutSeconds: timeoutSeconds)
        defer { Darwin.close(client) }
        try sendEncodable(request, on: client)
        let (responseData, fileDescriptor) = try UnixFDPassing.receive(on: client)
        if let fileDescriptor { Darwin.close(fileDescriptor) }
        let response = try decodeResponse(responseData)
        guard response.ok else {
            throw Error.helper(response.error ?? "operation failed")
        }
    }

    private static func sendEncodable<T: Encodable>(_ request: T, on client: Int32) throws {
        var payload = try JSONEncoder().encode(request)
        payload.append(0x0A)
        let sent = payload.withUnsafeBytes { raw -> Int in
            guard let base = raw.baseAddress else { return 0 }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.send(client, base.advanced(by: offset), raw.count - offset, 0)
                if count <= 0 { return offset }
                offset += count
            }
            return offset
        }
        guard sent == payload.count else {
            throw Error.system("send", errno)
        }
    }

    fileprivate static func decodeResponse(_ data: Data) throws -> DnsBind.BindResponse {
        let trimmed = data.trimmingASCIINewlinesPublic()
        guard let response = try? JSONDecoder().decode(DnsBind.BindResponse.self, from: trimmed)
        else {
            throw Error.response("invalid JSON")
        }
        return response
    }
}

private extension Data {
    func trimmingASCIINewlinesPublic() -> Data {
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
