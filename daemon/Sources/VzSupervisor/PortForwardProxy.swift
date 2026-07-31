import Darwin
import Dispatch
import Foundation
import VzDaemonKit

enum PortProxyError: Error, CustomStringConvertible {
    case bindFailed(String, UInt16, Int32)
    case invalidAddress(String)
    case collision(String, UInt16)

    var description: String {
        switch self {
        case let .bindFailed(bind, port, code):
            return "cannot bind \(bind):\(port): \(String(cString: strerror(code)))"
        case let .invalidAddress(value):
            return "invalid IP address \(value)"
        case let .collision(bind, port):
            return "host bind \(bind):\(port) is already in use"
        }
    }
}

/// Userspace TCP proxy: host loopback → guest IP (Alpha port forwards).
final class PortForwardProxy: @unchecked Sendable {
    private let lock = NSLock()
    private var listeners: [String: Listener] = [:]

    func ensure(_ desired: [PortForwardRecord]) throws -> [PortForwardRecord] {
        lock.lock()
        defer { lock.unlock() }

        let projects = Set(desired.map(\.project))
        let stacks = Set(desired.map(\.stack))
        let desiredKeys = Set(desired.map(\.key))
        let scopedKeys = listeners.compactMap { key, listener -> String? in
            let record = listener.record
            if desiredKeys.contains(key) { return nil }
            if projects.contains(record.project), stacks.contains(record.stack) {
                return key
            }
            return nil
        }
        for key in scopedKeys {
            listeners.removeValue(forKey: key)?.close()
        }

        var active: [PortForwardRecord] = []
        for record in desired {
            if let existing = listeners[record.key], existing.matches(record) {
                active.append(record)
                continue
            }
            listeners.removeValue(forKey: record.key)?.close()
            let listener = try Listener(record: record)
            listeners[record.key] = listener
            active.append(record)
        }
        return active.sorted { $0.hostPort < $1.hostPort }
    }

    func list() -> [PortForwardRecord] {
        lock.withLock {
            listeners.values.map(\.record).sorted { $0.hostPort < $1.hostPort }
        }
    }

    func purge(project: String, stack: String) {
        lock.lock()
        defer { lock.unlock() }
        let keys = listeners.compactMap { key, listener -> String? in
            listener.record.project == project && listener.record.stack == stack ? key : nil
        }
        for key in keys {
            listeners.removeValue(forKey: key)?.close()
        }
    }

    func purge(vmID: String) {
        lock.lock()
        defer { lock.unlock() }
        let keys = listeners.compactMap { key, listener -> String? in
            listener.record.vmID == vmID ? key : nil
        }
        for key in keys {
            listeners.removeValue(forKey: key)?.close()
        }
    }

    func shutdown() {
        lock.lock()
        defer { lock.unlock() }
        for key in Array(listeners.keys) {
            listeners.removeValue(forKey: key)?.close()
        }
    }

    private final class Listener: @unchecked Sendable {
        let record: PortForwardRecord
        private let fd: Int32
        private let queue: DispatchQueue
        private var source: DispatchSourceRead?
        private var sessions: [UUID: Session] = [:]
        private let sessionLock = NSLock()

        init(record: PortForwardRecord) throws {
            self.record = record
            let sock = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
            guard sock >= 0 else {
                throw PortProxyError.bindFailed(record.bind, record.hostPort, errno)
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
            addr.sin_port = record.hostPort.bigEndian
            guard inet_pton(AF_INET, record.bind, &addr.sin_addr) == 1 else {
                Darwin.close(sock)
                throw PortProxyError.invalidAddress(record.bind)
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
                    throw PortProxyError.collision(record.bind, record.hostPort)
                }
                throw PortProxyError.bindFailed(record.bind, record.hostPort, code)
            }
            guard Darwin.listen(sock, 32) == 0 else {
                let code = errno
                Darwin.close(sock)
                throw PortProxyError.bindFailed(record.bind, record.hostPort, code)
            }
            fd = sock
            queue = DispatchQueue(label: "vzctl.port.\(record.key)")
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

        func matches(_ other: PortForwardRecord) -> Bool {
            record.bind == other.bind
                && record.hostPort == other.hostPort
                && record.guestIP == other.guestIP
                && record.guestPort == other.guestPort
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
            guard sock >= 0 else {
                throw PortProxyError.bindFailed(record.guestIP, record.guestPort, errno)
            }
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
            addr.sin_port = record.guestPort.bigEndian
            guard inet_pton(AF_INET, record.guestIP, &addr.sin_addr) == 1 else {
                Darwin.close(sock)
                throw PortProxyError.invalidAddress(record.guestIP)
            }
            let result = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard result == 0 else {
                let code = errno
                Darwin.close(sock)
                throw PortProxyError.bindFailed(record.guestIP, record.guestPort, code)
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
            queue = DispatchQueue(label: "vzctl.port.session.\(id.uuidString)")
        }

        func start() {
            let clientSource = DispatchSource.makeReadSource(fileDescriptor: client, queue: queue)
            clientSource.setEventHandler { [weak self] in
                self?.relay(from: self?.client ?? -1, to: self?.backend ?? -1)
            }
            clientSource.setCancelHandler { [weak self] in
                if let fd = self?.client { Darwin.close(fd) }
            }
            let backendSource = DispatchSource.makeReadSource(fileDescriptor: backend, queue: queue)
            backendSource.setEventHandler { [weak self] in
                self?.relay(from: self?.backend ?? -1, to: self?.client ?? -1)
            }
            backendSource.setCancelHandler { [weak self] in
                if let fd = self?.backend { Darwin.close(fd) }
            }
            self.clientSource = clientSource
            self.backendSource = backendSource
            clientSource.resume()
            backendSource.resume()
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

        private func relay(from sourceFD: Int32, to destinationFD: Int32) {
            guard sourceFD >= 0, destinationFD >= 0 else {
                close()
                return
            }
            var buffer = [UInt8](repeating: 0, count: 16 * 1024)
            let readCount = Darwin.read(sourceFD, &buffer, buffer.count)
            if readCount <= 0 {
                close()
                return
            }
            var written = 0
            while written < readCount {
                let result = Darwin.write(destinationFD, &buffer[written], readCount - written)
                if result <= 0 {
                    close()
                    return
                }
                written += result
            }
        }
    }
}
