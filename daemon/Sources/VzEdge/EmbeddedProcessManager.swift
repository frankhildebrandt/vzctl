import Darwin
import Foundation
import VzDaemonKit

enum EmbeddedProcessError: Error, CustomStringConvertible {
    case invalid(String)
    case exited(String, String)
    var description: String {
        switch self {
        case let .invalid(message): return message
        case let .exited(name, hint): return "\(name) exited: \(hint)"
        }
    }
}

final class EmbeddedProcessManager: @unchecked Sendable {
    struct Readiness: Equatable, Sendable {
        let host: String
        let port: UInt16
    }

    struct Spec: Equatable, Sendable {
        let kind: String
        let project: String
        let name: String
        let binary: String
        let arguments: [String]
        let workDir: String
        let pidFile: String
        let env: [String: String]
        let readiness: Readiness?

        func validateIdentity() throws {
            guard ["caddy", "dex", "oidc-simple"].contains(kind) else {
                throw EmbeddedProcessError.invalid("unsupported edge service kind \(kind)")
            }
            let expected = kind == "caddy" ? "caddy-\(project)" : "\(kind)-\(project)"
            guard name == expected else {
                throw EmbeddedProcessError.invalid("service name must be \(expected)")
            }
            guard !project.isEmpty, !binary.isEmpty, !arguments.isEmpty else {
                throw EmbeddedProcessError.invalid("incomplete \(kind) service spec")
            }
        }
    }

    private struct Managed {
        var spec: Spec
        var process: Process?
        var failures: Int
        var lastError: String?
        var restart: DispatchWorkItem?
    }

    private let lock = NSLock()
    private var managed: [String: Managed] = [:]
    private var stopped = false
    private let validatorTimeout: TimeInterval

    init(validatorTimeout: TimeInterval = 10) {
        self.validatorTimeout = validatorTimeout
    }

    func reconcile(_ desired: [Spec]) throws -> [[String: JSONValue]] {
        for spec in desired {
            try spec.validateIdentity()
            try validateConfiguration(spec)
            EmbeddedProcessReaper.reapStale(
                binary: spec.binary,
                workDir: spec.workDir,
                pidFile: spec.pidFile,
                exceptPID: lock.withLock { managed[spec.name]?.process?.processIdentifier }
            )
        }
        let desiredNames = Set(desired.map(\.name))
        let obsolete = lock.withLock { Array(managed.keys).filter { !desiredNames.contains($0) } }
        for name in obsolete { stop(name: name) }
        return try desired.map { try ensure($0) }
    }

    func ensure(_ spec: Spec) throws -> [String: JSONValue] {
        try spec.validateIdentity()
        try validateConfiguration(spec)
        return try lock.withLock {
            guard !stopped else { throw EmbeddedProcessError.invalid("edge process manager stopped") }
            if let current = managed[spec.name], current.spec == spec,
               current.process?.isRunning == true {
                return statusLocked(name: spec.name)
            }
            stopLocked(name: spec.name)
            managed[spec.name] = Managed(
                spec: spec, process: nil, failures: 0, lastError: nil, restart: nil
            )
            try startLocked(name: spec.name)
            return statusLocked(name: spec.name)
        }
    }

    func stop(name: String) { lock.withLock { stopLocked(name: name) } }

    func orphanCount(for spec: Spec) -> Int {
        EmbeddedProcessReaper.countOrphans(
            binary: spec.binary,
            workDir: spec.workDir,
            exceptPID: lock.withLock { managed[spec.name]?.process?.processIdentifier }
        )
    }

    func statusAll() -> [[String: JSONValue]] {
        lock.withLock { managed.keys.sorted().map(statusLocked) }
    }

    func shutdown() {
        lock.withLock {
            stopped = true
            for name in Array(managed.keys) { stopLocked(name: name) }
        }
    }

    private func validateConfiguration(_ spec: Spec) throws {
        guard spec.kind == "caddy" else { return }
        guard let configIndex = spec.arguments.firstIndex(of: "--config"),
              spec.arguments.indices.contains(configIndex + 1) else {
            throw EmbeddedProcessError.invalid("caddy --config is required")
        }
        let validator = Process()
        validator.executableURL = URL(fileURLWithPath: spec.binary)
        validator.arguments = [
            "validate", "--config", spec.arguments[configIndex + 1], "--adapter", "caddyfile",
        ]
        validator.currentDirectoryURL = URL(fileURLWithPath: spec.workDir)
        validator.standardOutput = FileHandle.nullDevice
        let errorPipe = Pipe()
        defer {
            try? errorPipe.fileHandleForReading.close()
            try? errorPipe.fileHandleForWriting.close()
        }
        validator.standardError = errorPipe
        try validator.run()
        try? errorPipe.fileHandleForWriting.close()
        let data = try validatorOutput(
            validator, descriptor: errorPipe.fileHandleForReading.fileDescriptor
        )
        validator.waitUntilExit()
        guard validator.terminationStatus == 0 else {
            let hint = String(data: data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw EmbeddedProcessError.invalid(hint?.isEmpty == false ? hint! : "invalid Caddyfile")
        }
    }

    private func validatorOutput(_ process: Process, descriptor: Int32) throws -> Data {
        let flags = fcntl(descriptor, F_GETFL)
        guard flags >= 0, fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
            terminate(process)
            throw EmbeddedProcessError.invalid("cannot read caddy validation output")
        }
        let deadline = ProcessInfo.processInfo.systemUptime + validatorTimeout
        var tail = Data()
        var buffer = [UInt8](repeating: 0, count: 8192)
        var eof = false
        while true {
            // Bound memory and each drain pass so continuous output cannot starve the deadline.
            for _ in 0..<16 where !eof {
                let count = Darwin.read(descriptor, &buffer, buffer.count)
                if count > 0 {
                    tail.append(contentsOf: buffer.prefix(count))
                    if tail.count > 65536 { tail.removeFirst(tail.count - 65536) }
                } else if count == 0 {
                    eof = true
                } else if errno == EAGAIN || errno == EWOULDBLOCK {
                    break
                } else if errno != EINTR {
                    terminate(process)
                    throw EmbeddedProcessError.invalid("cannot read caddy validation output")
                }
            }
            if eof && !process.isRunning { return tail }
            if ProcessInfo.processInfo.systemUptime >= deadline {
                terminate(process)
                throw EmbeddedProcessError.invalid("caddy validation timed out")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }

    private func startLocked(name: String) throws {
        guard var item = managed[name] else { return }
        let spec = item.spec
        EmbeddedProcessReaper.reapStale(
            binary: spec.binary,
            workDir: spec.workDir,
            pidFile: spec.pidFile
        )
        try FileManager.default.createDirectory(atPath: spec.workDir, withIntermediateDirectories: true)
        let process = Process()
        process.executableURL = URL(fileURLWithPath: spec.binary)
        process.arguments = spec.arguments
        process.currentDirectoryURL = URL(fileURLWithPath: spec.workDir)
        var environment = ProcessInfo.processInfo.environment
        for (key, value) in spec.env { environment[key] = value }
        process.environment = environment
        let logPath = (spec.workDir as NSString).appendingPathComponent("\(spec.name).log")
        FileManager.default.createFile(atPath: logPath, contents: nil)
        let logHandle = try FileHandle(forWritingTo: URL(fileURLWithPath: logPath))
        defer { try? logHandle.close() }
        try logHandle.seekToEnd()
        process.standardOutput = logHandle
        process.standardError = logHandle
        process.terminationHandler = { [weak self] child in
            self?.childExited(name: name, pid: child.processIdentifier, status: child.terminationStatus)
        }
        try process.run()
        // The child owns its duplicated descriptors; the parent must release its copy.
        try? logHandle.close()
        guard waitUntilReady(process: process, readiness: spec.readiness) else {
            let hint = logTail(path: logPath)
                ?? (process.isRunning ? "readiness timed out" : "exit \(process.terminationStatus)")
            terminate(process)
            item.lastError = hint
            managed[name] = item
            throw EmbeddedProcessError.exited(name, "readiness failed: \(hint)")
        }
        do {
            try "\(process.processIdentifier)\n".write(
                toFile: spec.pidFile, atomically: true, encoding: .utf8
            )
        } catch {
            terminate(process)
            throw error
        }
        _ = chmod(spec.pidFile, 0o600)
        item.process = process
        item.restart = nil
        managed[name] = item
    }

    private func childExited(name: String, pid: Int32, status: Int32) {
        lock.withLock {
            guard !stopped, var item = managed[name],
                  item.process?.processIdentifier == pid else { return }
            item.process = nil
            item.failures += 1
            item.lastError = "exit \(status)"
            scheduleRestartLocked(name: name, item: &item)
            managed[name] = item
        }
    }

    private func restart(name: String) {
        lock.withLock {
            guard !stopped, managed[name] != nil else { return }
            do { try startLocked(name: name) }
            catch {
                guard var item = managed[name] else { return }
                item.lastError = String(describing: error)
                item.failures += 1
                scheduleRestartLocked(name: name, item: &item)
                managed[name] = item
            }
        }
    }

    private func scheduleRestartLocked(name: String, item: inout Managed) {
        let delay = min(30, 1 << min(max(0, item.failures - 1), 5))
        let work = DispatchWorkItem { [weak self] in self?.restart(name: name) }
        item.restart = work
        DispatchQueue.global(qos: .utility).asyncAfter(
            deadline: .now() + .seconds(delay), execute: work
        )
    }

    private func stopLocked(name: String) {
        guard let item = managed.removeValue(forKey: name) else { return }
        item.restart?.cancel()
        if let process = item.process { terminate(process) }
        try? FileManager.default.removeItem(atPath: item.spec.pidFile)
    }

    private func terminate(_ process: Process) {
        // A deliberate stop must not enqueue an automatic restart.
        process.terminationHandler = nil
        if process.isRunning {
            process.terminate()
            let deadline = Date().addingTimeInterval(3)
            while process.isRunning && Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
        }
        process.waitUntilExit()
    }

    private func statusLocked(name: String) -> [String: JSONValue] {
        guard let item = managed[name] else {
            return ["name": .string(name), "running": .bool(false)]
        }
        let running = item.process?.isRunning == true
        return [
            "name": .string(name), "kind": .string(item.spec.kind),
            "project": .string(item.spec.project), "running": .bool(running),
            "pid": running ? .number(Double(item.process?.processIdentifier ?? 0)) : .null,
            "restarts": .number(Double(item.failures)),
            "last_error": item.lastError.map(JSONValue.string) ?? .null,
        ]
    }

    private func logTail(path: String) -> String? {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: path)),
              let text = String(data: data, encoding: .utf8) else { return nil }
        let tail = text.split(separator: "\n").suffix(8).joined(separator: "\n")
        return tail.isEmpty ? nil : tail
    }

    private func waitUntilReady(process: Process, readiness: Readiness?) -> Bool {
        let deadline = Date().addingTimeInterval(3)
        repeat {
            guard process.isRunning else { return false }
            if let readiness {
                if canConnect(host: readiness.host, port: readiness.port) { return true }
            } else {
                Thread.sleep(forTimeInterval: 0.35)
                return process.isRunning
            }
            Thread.sleep(forTimeInterval: 0.05)
        } while Date() < deadline
        return false
    }

    private func canConnect(host: String, port: UInt16) -> Bool {
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
}
