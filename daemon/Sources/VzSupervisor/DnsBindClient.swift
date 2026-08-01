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

    /// Ask the privileged helper to bind `address:port` and return the UDP FD.
    /// Caller owns the returned descriptor.
    static func bindUDP(
        address: String,
        port: UInt16,
        socketPath: String = DnsBind.socketPath(),
        timeoutSeconds: Int = 2
    ) throws -> Int32 {
        let request = DnsBind.BindRequest(address: address, port: port)
        try DnsBind.validate(request)

        let client = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard client >= 0 else { throw Error.system("socket", errno) }
        defer { Darwin.close(client) }

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
            throw Error.connect(String(cString: strerror(errno)))
        }

        var payload = try JSONEncoder().encode(request)
        payload.append(0x0A)
        let sent = payload.withUnsafeBytes { raw in
            Darwin.send(client, raw.baseAddress, raw.count, 0)
        }
        guard sent == payload.count else {
            throw Error.system("send", errno)
        }

        let (responseData, fileDescriptor) = try UnixFDPassing.receive(on: client)
        let trimmed = responseData.trimmingASCIINewlinesPublic()
        guard let response = try? JSONDecoder().decode(DnsBind.BindResponse.self, from: trimmed)
        else {
            throw Error.response("invalid JSON")
        }
        guard response.ok else {
            throw Error.helper(response.error ?? "bind failed")
        }
        guard let fileDescriptor else {
            throw Error.response("missing file descriptor")
        }
        return fileDescriptor
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
