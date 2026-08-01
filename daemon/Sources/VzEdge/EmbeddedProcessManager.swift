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

    func reconcile(_ desired: [Spec]) throws -> [[String: JSONValue]] {
        for spec in desired {
            try spec.validateIdentity()
            try validateConfiguration(spec)
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
        validator.standardError = errorPipe
        try validator.run()
        validator.waitUntilExit()
        guard validator.terminationStatus == 0 else {
            let data = errorPipe.fileHandleForReading.readDataToEndOfFile()
            let hint = String(data: data, encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            throw EmbeddedProcessError.invalid(hint?.isEmpty == false ? hint! : "invalid Caddyfile")
        }
    }

    private func startLocked(name: String) throws {
        guard var item = managed[name] else { return }
        let spec = item.spec
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
        try logHandle.seekToEnd()
        process.standardOutput = logHandle
        process.standardError = logHandle
        process.terminationHandler = { [weak self] child in
            self?.childExited(name: name, pid: child.processIdentifier, status: child.terminationStatus)
        }
        try process.run()
        guard waitUntilReady(process: process, readiness: spec.readiness) else {
            let hint = logTail(path: logPath) ?? "exit \(process.terminationStatus)"
            if process.isRunning { process.terminate() }
            item.lastError = hint
            managed[name] = item
            throw EmbeddedProcessError.exited(name, "readiness failed: \(hint)")
        }
        item.process = process
        item.restart = nil
        managed[name] = item
        try "\(process.processIdentifier)\n".write(
            toFile: spec.pidFile, atomically: true, encoding: .utf8
        )
        _ = chmod(spec.pidFile, 0o600)
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
        if let process = item.process, process.isRunning {
            process.terminate()
            let deadline = Date().addingTimeInterval(3)
            while process.isRunning && Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
            if process.isRunning { _ = Darwin.kill(process.processIdentifier, SIGKILL) }
        }
        try? FileManager.default.removeItem(atPath: item.spec.pidFile)
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
