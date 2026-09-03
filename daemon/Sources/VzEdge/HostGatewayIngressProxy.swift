import Darwin
import Dispatch
import Foundation
import VzDaemonKit

/// TCP proxy: guest-facing gateway `.0:80/:443` (+ loopback) → Caddy on unprivileged ports.
final class HostGatewayIngressProxy: @unchecked Sendable {
    struct Binding: Equatable, Sendable {
        let gatewayIP: String
        let port: UInt16
        let backendHost: String
        let backendPort: UInt16

        var key: String { "\(gatewayIP):\(port)" }
    }

    /// Keep the first binding per `gatewayIP:port` so two stacks cannot double-map :80/:443.
    static func uniqueBindings(_ bindings: [Binding]) -> [Binding] {
        var seen = Set<String>()
        var unique: [Binding] = []
        for binding in bindings where seen.insert(binding.key).inserted {
            unique.append(binding)
        }
        return unique
    }

    struct EnsureResult: Sendable {
        let active: [Binding]
        let skipped: [(Binding, String)]
    }

    private let lock = NSLock()
    private var listeners: [String: Listener] = [:]
    private var cachedBindings: [Binding] = []
    private var cachedLoopbackGateways = Set<String>(["127.0.0.1"])
    private var cachedHTTPPort: UInt16 = 80
    private var cachedHTTPSPort: UInt16 = 443

    /// Ensures gateway listeners. Per-binding bind failures are skipped
    /// (same soft-fail pattern as DNS `.0:53`) so host loopback Caddy still starts.
    func ensure(_ desired: [Binding]) -> EnsureResult {
        lock.lock()
        defer { lock.unlock() }

        let desiredKeys = Set(desired.map(\.key))
        for key in listeners.keys where !desiredKeys.contains(key) {
            listeners.removeValue(forKey: key)?.close()
        }

        var active: [Binding] = []
        var skipped: [(Binding, String)] = []
        for binding in desired {
            listeners.removeValue(forKey: binding.key)?.close()
            do {
                let listener = try openListener(binding)
                listeners[binding.key] = listener
                active.append(binding)
            } catch {
                skipped.append((binding, "\(error)"))
            }
        }
        return EnsureResult(
            active: active.sorted { $0.key < $1.key },
            skipped: skipped.sorted { $0.0.key < $1.0.key }
        )
    }

    /// Ensures listeners and probes loopback reachability with short retries.
    func ensureWithProbe(
        _ desired: [Binding],
        loopbackGateways: Set<String>,
        httpPort: UInt16,
        httpsPort: UInt16
    ) -> EnsureResult {
        lock.lock()
        cachedBindings = desired
        cachedLoopbackGateways = loopbackGateways
        cachedHTTPPort = httpPort
        cachedHTTPSPort = httpsPort
        lock.unlock()

        var result = ensure(desired)
        for _ in 0 ..< 2 where !loopbackGateways.isEmpty {
            if Self.loopbackReachable(
                gateways: loopbackGateways,
                httpPort: httpPort,
                httpsPort: httpsPort
            ) {
                break
            }
            Thread.sleep(forTimeInterval: 0.2)
            result = ensure(desired)
        }
        return result
    }

    /// Re-opens cached loopback bindings when dns-bind accept streams have died.
    @discardableResult
    func repairLoopbackIfNeeded() -> Bool {
        let snapshot = lock.withLock {
            (cachedBindings, cachedLoopbackGateways, cachedHTTPPort, cachedHTTPSPort)
        }
        guard !snapshot.0.isEmpty, !snapshot.1.isEmpty else { return true }
        if Self.loopbackReachable(
            gateways: snapshot.1,
            httpPort: snapshot.2,
            httpsPort: snapshot.3
        ) {
            return true
        }
        _ = ensure(snapshot.0)
        return Self.loopbackReachable(
            gateways: snapshot.1,
            httpPort: snapshot.2,
            httpsPort: snapshot.3
        )
    }

    func loopbackHealthy() -> Bool {
        let snapshot = lock.withLock {
            (cachedLoopbackGateways, cachedHTTPPort, cachedHTTPSPort, cachedBindings.isEmpty)
        }
        guard !snapshot.3 else { return true }
        return Self.loopbackReachable(
            gateways: snapshot.0,
            httpPort: snapshot.1,
            httpsPort: snapshot.2
        )
    }

    static func loopbackReachable(
        gateways: Set<String>,
        httpPort: UInt16,
        httpsPort: UInt16
    ) -> Bool {
        guard !gateways.isEmpty else { return true }
        for gateway in gateways {
            for port in [httpPort, httpsPort] {
                if !canConnect(host: gateway, port: port) { return false }
            }
        }
        return true
    }

    private static func canConnect(host: String, port: UInt16) -> Bool {
        let descriptor = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard descriptor >= 0 else { return false }
        defer { Darwin.close(descriptor) }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        guard inet_pton(AF_INET, host, &address.sin_addr) == 1 else { return false }
        return withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size)) == 0
            }
        }
    }

    private func openListener(_ binding: Binding) throws -> Listener {
        var lastError: Error?
        for attempt in 0 ..< 8 {
            do {
                return try Listener(binding: binding)
            } catch {
                lastError = error
                let text = "\(error)"
                if text.contains("Address already in use")
                    || text.contains("EADDRINUSE")
                    || text.contains("Resource temporarily unavailable")
                    || text.contains("EAGAIN") {
                    Thread.sleep(forTimeInterval: 0.15 + Double(attempt) * 0.05)
                    continue
                }
                throw error
            }
        }
        throw lastError ?? PortProxyError.bindFailed(binding.gatewayIP, binding.port, EADDRINUSE)
    }

    func list() -> [Binding] {
        lock.withLock {
            listeners.values.map(\.binding).sorted { $0.key < $1.key }
        }
    }

    func purge() {
        lock.lock()
        defer { lock.unlock() }
        for key in Array(listeners.keys) {
            listeners.removeValue(forKey: key)?.close()
        }
        cachedBindings = []
    }

    func shutdown() {
        purge()
    }

    private final class Listener: @unchecked Sendable {
        private(set) var binding: Binding
        private let queue: DispatchQueue
        private var sessions: [UUID: Session] = [:]
        private let sessionLock = NSLock()
        private var stopped = false
        private var localFD: Int32 = -1
        private var acceptStream: DnsBindClient.TCPAcceptStream?

        init(binding: Binding) throws {
            self.binding = binding
            queue = DispatchQueue(label: "vzctl.ingress.gw.\(binding.key)")
            try openAcceptSource()
            queue.async { [self] in
                self.acceptLoop()
            }
        }

        private func openAcceptSource() throws {
            if DnsBind.needsPrivilege(port: binding.port) {
                acceptStream?.close()
                acceptStream = nil
                acceptStream = try DnsBindClient.openTCPAcceptStream(
                    address: binding.gatewayIP,
                    port: binding.port
                )
                return
            }
            if localFD >= 0 {
                Darwin.shutdown(localFD, SHUT_RDWR)
                Darwin.close(localFD)
                localFD = -1
            }
            let sock = try Self.bindLocally(address: binding.gatewayIP, port: binding.port)
            guard Darwin.listen(sock, 32) == 0 else {
                let code = errno
                Darwin.close(sock)
                throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, code)
            }
            localFD = sock
        }

        private static func bindLocally(address: String, port: UInt16) throws -> Int32 {
            let sock = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
            guard sock >= 0 else {
                throw PortProxyError.bindFailed(address, port, errno)
            }
            var yes: Int32 = 1
            setsockopt(
                sock,
                SOL_SOCKET,
                SO_REUSEADDR,
                &yes,
                socklen_t(MemoryLayout.size(ofValue: yes))
            )
            var addr = sockaddr_in()
            addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = port.bigEndian
            guard inet_pton(AF_INET, address, &addr.sin_addr) == 1 else {
                Darwin.close(sock)
                throw PortProxyError.invalidAddress(address)
            }
            let bindResult = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard bindResult == 0 else {
                let code = errno
                Darwin.close(sock)
                if code == EADDRINUSE {
                    throw PortProxyError.collision(address, port)
                }
                throw PortProxyError.bindFailed(address, port, code)
            }
            return sock
        }

        func close() {
            stopped = true
            acceptStream?.close()
            acceptStream = nil
            if localFD >= 0 {
                Darwin.shutdown(localFD, SHUT_RDWR)
                Darwin.close(localFD)
                localFD = -1
            }
            sessionLock.lock()
            let values = Array(sessions.values)
            sessions.removeAll()
            sessionLock.unlock()
            for session in values {
                session.close()
            }
        }

        private func acceptLoop() {
            var backoff = 0.1
            while !stopped {
                do {
                    try ensureAcceptSourceReady()
                    let client = try acceptClient()
                    backoff = 0.1
                    handleClient(client)
                } catch AcceptLoopError.interrupted {
                    continue
                } catch {
                    if stopped { return }
                    acceptStream?.close()
                    acceptStream = nil
                    if localFD >= 0 {
                        Darwin.shutdown(localFD, SHUT_RDWR)
                        Darwin.close(localFD)
                        localFD = -1
                    }
                    Thread.sleep(forTimeInterval: backoff)
                    backoff = min(backoff * 1.5, 2.0)
                }
            }
        }

        private enum AcceptLoopError: Error {
            case interrupted
        }

        private func ensureAcceptSourceReady() throws {
            if DnsBind.needsPrivilege(port: binding.port) {
                if acceptStream == nil {
                    try openAcceptSource()
                }
                return
            }
            if localFD < 0 {
                try openAcceptSource()
            }
        }

        private func acceptClient() throws -> Int32 {
            if let stream = acceptStream {
                return try stream.accept()
            }
            if localFD >= 0 {
                let accepted = Darwin.accept(localFD, nil, nil)
                if accepted < 0 {
                    if errno == EINTR { throw AcceptLoopError.interrupted }
                    let code = errno
                    Darwin.shutdown(localFD, SHUT_RDWR)
                    Darwin.close(localFD)
                    localFD = -1
                    throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, code)
                }
                return accepted
            }
            throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, EINVAL)
        }

        private func handleClient(_ client: Int32) {
            var noSigPipe: Int32 = 1
            setsockopt(
                client,
                SOL_SOCKET,
                SO_NOSIGPIPE,
                &noSigPipe,
                socklen_t(MemoryLayout<Int32>.size)
            )
            do {
                let backend = try connectBackend()
                let session = Session(client: client, backend: backend) { [weak self] id in
                    self?.sessionLock.lock()
                    self?.sessions.removeValue(forKey: id)
                    self?.sessionLock.unlock()
                }
                sessionLock.lock()
                sessions[session.id] = session
                sessionLock.unlock()
                session.start()
            } catch {
                Darwin.close(client)
            }
        }

        private func connectBackend() throws -> Int32 {
            let sock = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
            guard sock >= 0 else { throw PortProxyError.invalidAddress(binding.backendHost) }
            var noSigPipe: Int32 = 1
            setsockopt(
                sock,
                SOL_SOCKET,
                SO_NOSIGPIPE,
                &noSigPipe,
                socklen_t(MemoryLayout<Int32>.size)
            )
            var addr = sockaddr_in()
            addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = binding.backendPort.bigEndian
            guard inet_pton(AF_INET, binding.backendHost, &addr.sin_addr) == 1 else {
                Darwin.close(sock)
                throw PortProxyError.invalidAddress(binding.backendHost)
            }
            let result = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard result == 0 else {
                let code = errno
                Darwin.close(sock)
                throw PortProxyError.bindFailed(binding.backendHost, binding.backendPort, code)
            }
            return sock
        }
    }

    private final class Session: @unchecked Sendable {
        let id = UUID()
        private let client: Int32
        private let backend: Int32
        private let onClose: (UUID) -> Void
        private let lock = NSLock()
        private var closed = false

        init(client: Int32, backend: Int32, onClose: @escaping (UUID) -> Void) {
            self.client = client
            self.backend = backend
            self.onClose = onClose
        }

        func start() {
            let clientFD = client
            let backendFD = backend
            let group = DispatchGroup()
            group.enter()
            DispatchQueue.global(qos: .userInitiated).async {
                Self.relay(from: clientFD, to: backendFD)
                Darwin.shutdown(backendFD, SHUT_WR)
                Darwin.shutdown(clientFD, SHUT_RD)
                group.leave()
            }
            group.enter()
            DispatchQueue.global(qos: .userInitiated).async {
                Self.relay(from: backendFD, to: clientFD)
                Darwin.shutdown(clientFD, SHUT_WR)
                Darwin.shutdown(backendFD, SHUT_RD)
                group.leave()
            }
            let onClose = self.onClose
            let id = self.id
            group.notify(queue: .global(qos: .utility)) {
                Darwin.close(clientFD)
                Darwin.close(backendFD)
                onClose(id)
            }
        }

        private static func relay(from: Int32, to: Int32) {
            var buffer = [UInt8](repeating: 0, count: 65_536)
            while true {
                let count: Int = buffer.withUnsafeMutableBytes { raw in
                    guard let base = raw.baseAddress else { return -1 }
                    return Darwin.read(from, base, raw.count)
                }
                if count <= 0 { return }
                var offset = 0
                while offset < count {
                    let written: Int = buffer.withUnsafeBytes { raw in
                        guard let base = raw.baseAddress else { return -1 }
                        return Darwin.write(to, base.advanced(by: offset), count - offset)
                    }
                    if written <= 0 { return }
                    offset += written
                }
            }
        }

        func close() {
            lock.lock()
            defer { lock.unlock() }
            guard !closed else { return }
            closed = true
            Darwin.shutdown(client, SHUT_RDWR)
            Darwin.shutdown(backend, SHUT_RDWR)
            onClose(id)
        }
    }
}
