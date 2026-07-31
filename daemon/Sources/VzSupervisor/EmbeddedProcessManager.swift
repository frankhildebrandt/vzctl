import Foundation
import VzDaemonKit

/// Manages an embedded child process (Caddy / Dex) under Application Support.
final class EmbeddedProcessManager: @unchecked Sendable {
    struct Spec: Equatable, Sendable {
        let name: String
        let binary: String
        let arguments: [String]
        let workDir: String
        let pidFile: String
        let env: [String: String]
    }

    private let lock = NSLock()
    private var processes: [String: Process] = [:]
    private var specs: [String: Spec] = [:]

    func ensure(_ spec: Spec) throws -> [String: JSONValue] {
        lock.lock()
        defer { lock.unlock() }

        if let existing = processes[spec.name], existing.isRunning, specs[spec.name] == spec {
            return statusLocked(name: spec.name)
        }
        stopLocked(name: spec.name)

        try FileManager.default.createDirectory(
            atPath: spec.workDir,
            withIntermediateDirectories: true
        )
        let process = Process()
        process.executableURL = URL(fileURLWithPath: spec.binary)
        process.arguments = spec.arguments
        process.currentDirectoryURL = URL(fileURLWithPath: spec.workDir)
        var environment = ProcessInfo.processInfo.environment
        for (key, value) in spec.env {
            environment[key] = value
        }
        process.environment = environment
        let logPath = (spec.workDir as NSString).appendingPathComponent("\(spec.name).log")
        FileManager.default.createFile(atPath: logPath, contents: nil)
        let logHandle = try FileHandle(forWritingTo: URL(fileURLWithPath: logPath))
        process.standardOutput = logHandle
        process.standardError = logHandle
        try process.run()
        processes[spec.name] = process
        specs[spec.name] = spec
        try "\(process.processIdentifier)\n".write(
            toFile: spec.pidFile,
            atomically: true,
            encoding: .utf8
        )
        return statusLocked(name: spec.name)
    }

    func stop(name: String) {
        lock.lock()
        defer { lock.unlock() }
        stopLocked(name: name)
    }

    func status(name: String) -> [String: JSONValue] {
        lock.withLock { statusLocked(name: name) }
    }

    func shutdown() {
        lock.lock()
        defer { lock.unlock() }
        for name in Array(processes.keys) {
            stopLocked(name: name)
        }
    }

    private func stopLocked(name: String) {
        if let process = processes.removeValue(forKey: name) {
            if process.isRunning {
                process.terminate()
                process.waitUntilExit()
            }
        }
        if let spec = specs.removeValue(forKey: name) {
            try? FileManager.default.removeItem(atPath: spec.pidFile)
        }
    }

    private func statusLocked(name: String) -> [String: JSONValue] {
        let process = processes[name]
        let running = process?.isRunning == true
        return [
            "name": .string(name),
            "running": .bool(running),
            "pid": running
                ? .number(Double(process?.processIdentifier ?? 0))
                : .null,
        ]
    }
}
