import Darwin
import Dispatch
import Foundation
import VzDaemonKit

/// TCP proxy: guest-facing gateway `.0:80/:443` → host loopback Caddy.
final class HostGatewayIngressProxy: @unchecked Sendable {
    struct Binding: Equatable, Sendable {
        let gatewayIP: String
        let port: UInt16
        let backendHost: String
        let backendPort: UInt16

        var key: String { "\(gatewayIP):\(port)" }
    }

    private let lock = NSLock()
    private var listeners: [String: Listener] = [:]

    /// Ensures gateway listeners. Per-binding bind failures are skipped
    /// (same soft-fail pattern as DNS `.0:53`) so host loopback Caddy still starts.
    func ensure(_ desired: [Binding]) -> [Binding] {
        lock.lock()
        defer { lock.unlock() }

        let desiredKeys = Set(desired.map(\.key))
        for key in listeners.keys where !desiredKeys.contains(key) {
            listeners.removeValue(forKey: key)?.close()
        }

        var active: [Binding] = []
        for binding in desired {
            if let existing = listeners[binding.key], existing.matches(binding) {
                active.append(binding)
                continue
            }
            listeners.removeValue(forKey: binding.key)?.close()
            do {
                let listener = try Listener(binding: binding)
                listeners[binding.key] = listener
                active.append(binding)
            } catch {
                // EADDRNOTAVAIL until host bridge is up; collision; etc.
                continue
            }
        }
        return active.sorted { $0.key < $1.key }
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
    }

    func shutdown() {
        purge()
    }

    private final class Listener: @unchecked Sendable {
        let binding: Binding
        private let fd: Int32
        private let queue: DispatchQueue
        private var source: DispatchSourceRead?
        private var sessions: [UUID: Session] = [:]
        private let sessionLock = NSLock()

        init(binding: Binding) throws {
            self.binding = binding
            let sock = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
            guard sock >= 0 else {
                throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, errno)
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
            addr.sin_port = binding.port.bigEndian
            guard inet_pton(AF_INET, binding.gatewayIP, &addr.sin_addr) == 1 else {
                Darwin.close(sock)
                throw PortProxyError.invalidAddress(binding.gatewayIP)
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
                    throw PortProxyError.collision(binding.gatewayIP, binding.port)
                }
                throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, code)
            }
            guard Darwin.listen(sock, 32) == 0 else {
                let code = errno
                Darwin.close(sock)
                throw PortProxyError.bindFailed(binding.gatewayIP, binding.port, code)
            }
            fd = sock
            queue = DispatchQueue(label: "vzctl.ingress.gw.\(binding.key)")
            let source = DispatchSource.makeReadSource(fileDescriptor: sock, queue: queue)
            source.setEventHandler { [weak self] in
                self?.acceptOne()
            }
            source.setCancelHandler {
                Darwin.close(sock)
            }
            self.source = source
            source.resume()
        }

        func matches(_ other: Binding) -> Bool {
            binding == other
        }

        func close() {
            source?.cancel()
            source = nil
            sessionLock.lock()
            let values = Array(sessions.values)
            sessions.removeAll()
            sessionLock.unlock()
            for session in values {
                session.close()
            }
        }

        private func acceptOne() {
            let client = Darwin.accept(fd, nil, nil)
            guard client >= 0 else { return }
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
        private let queue: DispatchQueue
        private var clientSource: DispatchSourceRead?
        private var backendSource: DispatchSourceRead?
        private let onClose: (UUID) -> Void
        private let lock = NSLock()
        private var closed = false

        init(client: Int32, backend: Int32, onClose: @escaping (UUID) -> Void) {
            self.client = client
            self.backend = backend
            self.onClose = onClose
            queue = DispatchQueue(label: "vzctl.ingress.session.\(UUID().uuidString)")
        }

        func start() {
            let clientSource = DispatchSource.makeReadSource(fileDescriptor: client, queue: queue)
            clientSource.setEventHandler { [weak self] in
                self?.pump(from: self?.client ?? -1, to: self?.backend ?? -1)
            }
            clientSource.setCancelHandler { [weak self] in
                if let fd = self?.client { Darwin.close(fd) }
            }
            let backendSource = DispatchSource.makeReadSource(fileDescriptor: backend, queue: queue)
            backendSource.setEventHandler { [weak self] in
                self?.pump(from: self?.backend ?? -1, to: self?.client ?? -1)
            }
            backendSource.setCancelHandler { [weak self] in
                if let fd = self?.backend { Darwin.close(fd) }
            }
            self.clientSource = clientSource
            self.backendSource = backendSource
            clientSource.resume()
            backendSource.resume()
        }

        private func pump(from: Int32, to: Int32) {
            guard from >= 0, to >= 0 else { return }
            var buffer = [UInt8](repeating: 0, count: 65_536)
            let count = Darwin.read(from, &buffer, buffer.count)
            if count <= 0 {
                close()
                return
            }
            var offset = 0
            while offset < count {
                let written = Darwin.write(to, &buffer[offset], count - offset)
                if written <= 0 {
                    close()
                    return
                }
                offset += written
            }
        }

        func close() {
            lock.lock()
            defer { lock.unlock() }
            guard !closed else { return }
            closed = true
            clientSource?.cancel()
            backendSource?.cancel()
            clientSource = nil
            backendSource = nil
            onClose(id)
        }
    }
}
