import Darwin
import Foundation
import VzDaemonKit

final class RestServer: @unchecked Sendable {
    private let listenSpec: RestListenSpec
    private let router: RestRouter
    private let lock = NSLock()
    private var listenerFD: Int32 = -1
    private var ownsUnixSocket = false
    private var unixPath: String?

    init(listenSpec: RestListenSpec, router: RestRouter) {
        self.listenSpec = listenSpec
        self.router = router
    }

    var description: String { listenSpec.description }

    func start() throws {
        switch listenSpec {
        case let .unix(path):
            try listenUnix(path: path)
        case let .tcp(host, port):
            try listenTCP(host: host, port: port)
        }
        let fd = lock.withLock { listenerFD }
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.acceptLoop(fd)
        }
    }

    func stop() {
        let state = lock.withLock { () -> (Int32, Bool, String?) in
            let fd = listenerFD
            let unlink = ownsUnixSocket
            let path = unixPath
            listenerFD = -1
            ownsUnixSocket = false
            unixPath = nil
            return (fd, unlink, path)
        }
        if state.0 >= 0 {
            Darwin.shutdown(state.0, SHUT_RDWR)
            Darwin.close(state.0)
        }
        if state.1, let path = state.2 {
            Darwin.unlink(path)
        }
    }

    private func acceptLoop(_ fd: Int32) {
        while true {
            let alive = lock.withLock { listenerFD >= 0 }
            guard alive else { break }
            let client = Darwin.accept(fd, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                break
            }
            var noSigPipe: Int32 = 1
            setsockopt(
                client,
                SOL_SOCKET,
                SO_NOSIGPIPE,
                &noSigPipe,
                socklen_t(MemoryLayout<Int32>.size)
            )
            DispatchQueue.global().async { [weak self] in
                self?.handleClient(client)
                Darwin.close(client)
            }
        }
    }

    private func handleClient(_ client: Int32) {
        if case .unix = listenSpec {
            guard peerUID(client) == geteuid() else { return }
        }

        var pending = Data()
        var buffer = [UInt8](repeating: 0, count: 16_384)
        while true {
            let count = Darwin.read(client, &buffer, buffer.count)
            if count <= 0 { return }
            pending.append(buffer, count: count)
            if let request = RestHTTP.parseRequest(from: pending) {
                // SSE / long-lived: router may take ownership and return nil body write.
                if router.handleStreaming(request: request, client: client) {
                    return
                }
                let response = router.handle(request)
                let encoded = RestHTTP.encodeResponse(response)
                _ = writeAll(encoded, to: client)
                return
            }
            if pending.count > 8_000_000 { return }
        }
    }

    private func listenUnix(path: String) throws {
        if FileManager.default.fileExists(atPath: path) {
            Darwin.unlink(path)
        }
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SupervisorError.system("rest unix socket", errno) }
        var address = try unixAddress(path: path)
        let bindResult = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bindResult == 0 else {
            Darwin.close(fd)
            throw SupervisorError.system("rest unix bind", errno)
        }
        guard chmod(path, 0o600) == 0 else {
            Darwin.close(fd)
            Darwin.unlink(path)
            throw SupervisorError.system("rest unix chmod", errno)
        }
        guard Darwin.listen(fd, 32) == 0 else {
            Darwin.close(fd)
            Darwin.unlink(path)
            throw SupervisorError.system("rest unix listen", errno)
        }
        lock.withLock {
            listenerFD = fd
            ownsUnixSocket = true
            unixPath = path
        }
    }

    private func listenTCP(host: String, port: UInt16) throws {
        let fd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SupervisorError.system("rest tcp socket", errno) }
        var reuse: Int32 = 1
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_REUSEADDR,
            &reuse,
            socklen_t(MemoryLayout<Int32>.size)
        )
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        if host == "::1" {
            Darwin.close(fd)
            try listenTCPv6(port: port)
            return
        }
        addr.sin_addr = in_addr(s_addr: inet_addr(host))
        let bindResult = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            Darwin.close(fd)
            throw SupervisorError.system("rest tcp bind", errno)
        }
        guard Darwin.listen(fd, 32) == 0 else {
            Darwin.close(fd)
            throw SupervisorError.system("rest tcp listen", errno)
        }
        lock.withLock { listenerFD = fd }
    }

    private func listenTCPv6(port: UInt16) throws {
        let fd = Darwin.socket(AF_INET6, SOCK_STREAM, 0)
        guard fd >= 0 else { throw SupervisorError.system("rest tcp6 socket", errno) }
        var reuse: Int32 = 1
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_REUSEADDR,
            &reuse,
            socklen_t(MemoryLayout<Int32>.size)
        )
        var addr = sockaddr_in6()
        addr.sin6_len = UInt8(MemoryLayout<sockaddr_in6>.size)
        addr.sin6_family = sa_family_t(AF_INET6)
        addr.sin6_port = port.bigEndian
        addr.sin6_addr = in6addr_loopback
        let bindResult = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in6>.size))
            }
        }
        guard bindResult == 0 else {
            Darwin.close(fd)
            throw SupervisorError.system("rest tcp6 bind", errno)
        }
        guard Darwin.listen(fd, 32) == 0 else {
            Darwin.close(fd)
            throw SupervisorError.system("rest tcp6 listen", errno)
        }
        lock.withLock { listenerFD = fd }
    }

    private func unixAddress(path: String) throws -> sockaddr_un {
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let maxLength = MemoryLayout.size(ofValue: address.sun_path) - 1
        let pathBytes = Array(path.utf8)
        guard pathBytes.count <= maxLength else { throw SupervisorError.socketPathTooLong }
        withUnsafeMutablePointer(to: &address.sun_path) { ptr in
            ptr.withMemoryRebound(to: CChar.self, capacity: maxLength + 1) { dest in
                for (i, byte) in pathBytes.enumerated() {
                    dest[i] = CChar(bitPattern: byte)
                }
                dest[pathBytes.count] = 0
            }
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
            var offset = 0
            while offset < data.count {
                let written = Darwin.write(fd, raw.baseAddress!.advanced(by: offset), data.count - offset)
                if written <= 0 { return false }
                offset += written
            }
            return true
        }
    }
}
