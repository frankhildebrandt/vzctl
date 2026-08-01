import Foundation
import VzDaemonKit

struct RestJob: Sendable {
    enum Status: String, Codable, Sendable {
        case queued
        case running
        case succeeded
        case failed
    }

    var id: String
    var kind: String
    var status: Status
    var createdAt: String
    var updatedAt: String
    var result: JSONValue?
    var error: String?
    var log: [String]
}

final class RestJobRunner: @unchecked Sendable {
    private let lock = NSLock()
    private var jobs: [String: RestJob] = [:]
    private let stateDirectory: URL
    private var vzctlPathOverride: String?

    init(stateDirectory: URL, vzctlPath: String? = nil) {
        self.stateDirectory = stateDirectory
        self.vzctlPathOverride = vzctlPath
    }

    func get(_ id: String) -> RestJob? {
        lock.withLock { jobs[id] }
    }

    func logLines(_ id: String) -> [String] {
        lock.withLock { jobs[id]?.log ?? [] }
    }

    @discardableResult
    func start(kind: String, arguments: [String], environment: [String: String] = [:]) -> String {
        let id = UUID().uuidString.lowercased()
        let now = isoNow()
        let job = RestJob(
            id: id,
            kind: kind,
            status: .queued,
            createdAt: now,
            updatedAt: now,
            result: nil,
            error: nil,
            log: []
        )
        lock.withLock { jobs[id] = job }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.run(jobId: id, arguments: arguments, environment: environment)
        }
        return id
    }

    func runSync(arguments: [String], environment: [String: String] = [:]) throws -> (exitCode: Int32, stdout: String, stderr: String) {
        let vzctl = try resolveVzctl()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: vzctl)
        process.arguments = arguments
        var env = ProcessInfo.processInfo.environment
        for (k, v) in environment {
            env[k] = v
        }
        env["VZCTL_STATE_DIR"] = stateDirectory.path
        process.environment = env

        let outPipe = Pipe()
        let errPipe = Pipe()
        process.standardOutput = outPipe
        process.standardError = errPipe
        try process.run()
        process.waitUntilExit()
        let stdout = String(data: outPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        let stderr = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        return (process.terminationStatus, stdout, stderr)
    }

    private func run(jobId: String, arguments: [String], environment: [String: String]) {
        update(jobId) { job in
            job.status = .running
            job.updatedAt = isoNow()
        }
        do {
            let result = try runSync(arguments: arguments, environment: environment)
            let combined = [result.stdout, result.stderr]
                .flatMap { $0.split(separator: "\n", omittingEmptySubsequences: false).map(String.init) }
                .filter { !$0.isEmpty }
            let parsed = parseJSONValue(result.stdout)
            update(jobId) { job in
                job.log.append(contentsOf: combined.suffix(500))
                job.updatedAt = isoNow()
                if result.exitCode == 0 {
                    job.status = .succeeded
                    job.result = parsed ?? .object([
                        "stdout": .string(result.stdout),
                        "exit_code": .number(0),
                    ])
                } else {
                    job.status = .failed
                    job.error = parsed.flatMap(messageFromEnvelope)
                        ?? (result.stderr.isEmpty ? "exit \(result.exitCode)" : result.stderr)
                    job.result = parsed ?? .object([
                        "stdout": .string(result.stdout),
                        "stderr": .string(result.stderr),
                        "exit_code": .number(Double(result.exitCode)),
                    ])
                }
            }
        } catch {
            update(jobId) { job in
                job.status = .failed
                job.error = String(describing: error)
                job.updatedAt = isoNow()
            }
        }
    }

    private func update(_ id: String, mutate: (inout RestJob) -> Void) {
        lock.withLock {
            guard var job = jobs[id] else { return }
            mutate(&job)
            jobs[id] = job
        }
    }

    private func resolveVzctl() throws -> String {
        if let override = vzctlPathOverride, FileManager.default.isExecutableFile(atPath: override) {
            return override
        }
        if let env = ProcessInfo.processInfo.environment["VZCTL_BIN"],
           FileManager.default.isExecutableFile(atPath: env)
        {
            return env
        }
        let sibling = URL(fileURLWithPath: CommandLine.arguments[0])
            .deletingLastPathComponent()
            .appendingPathComponent("vzctl")
            .path
        if FileManager.default.isExecutableFile(atPath: sibling) {
            return sibling
        }
        // PATH lookup
        let pathEnv = ProcessInfo.processInfo.environment["PATH"] ?? "/usr/local/bin:/usr/bin:/bin"
        for dir in pathEnv.split(separator: ":") {
            let candidate = "\(dir)/vzctl"
            if FileManager.default.isExecutableFile(atPath: candidate) {
                return candidate
            }
        }
        throw RestConfigError.invalidListen("vzctl binary not found (set VZCTL_BIN)")
    }

    private func parseJSONValue(_ text: String) -> JSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let data = trimmed.data(using: .utf8) else { return nil }
        if let value = try? JSONDecoder().decode(JSONValue.self, from: data) {
            return value
        }
        // Recover trailing JSON object (CLI may leak diagnostics).
        guard let start = trimmed.firstIndex(of: "{"),
              let end = trimmed.lastIndex(of: "}"),
              start < end
        else { return nil }
        let slice = String(trimmed[start ... end])
        guard let sliceData = slice.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONValue.self, from: sliceData)
    }

    private func messageFromEnvelope(_ value: JSONValue) -> String? {
        guard case let .object(obj) = value else { return nil }
        if case let .string(message)? = obj["message"] { return message }
        if case let .object(summary)? = obj["summary"],
           case let .string(message)? = summary["message"]
        {
            return message
        }
        return nil
    }

    private func isoNow() -> String {
        ISO8601DateFormatter().string(from: Date())
    }
}

extension RestJob {
    var json: JSONValue {
        var obj: [String: JSONValue] = [
            "jobId": .string(id),
            "kind": .string(kind),
            "status": .string(status.rawValue),
            "createdAt": .string(createdAt),
            "updatedAt": .string(updatedAt),
        ]
        if let result { obj["result"] = result }
        if let error { obj["error"] = .string(error) }
        return .object(obj)
    }
}
