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
    var progressPercent: Int?
    var progressLabel: String?
}

final class RestJobRunner: @unchecked Sendable {
    static let maxLogLines = 500
    static let maxLineBytes = 4096
    struct Limits: Sendable {
        var retainedJobs = 32
        var activeJobs = 4
        var syncTimeout: TimeInterval = 60
        var jobTimeout: TimeInterval = 7200
    }

    private let lock = NSLock()
    private var jobs: [String: RestJob] = [:]
    private var order: [String] = []
    private var cancelledJobs = Set<String>()
    private let stateDirectory: URL
    private var vzctlPathOverride: String?
    private let limits: Limits

    init(stateDirectory: URL, vzctlPath: String? = nil, limits: Limits = Limits()) {
        self.stateDirectory = stateDirectory
        self.vzctlPathOverride = vzctlPath
        self.limits = limits
    }

    func get(_ id: String) -> RestJob? {
        lock.withLock { jobs[id] }
    }

    func logLines(_ id: String) -> [String] {
        lock.withLock { jobs[id]?.log ?? [] }
    }

    @discardableResult
    func start(kind: String, arguments: [String], environment: [String: String] = [:]) throws -> String {
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
            log: [],
            progressPercent: nil,
            progressLabel: nil
        )
        try lock.withLock {
            let active = jobs.values.filter { $0.status == .queued || $0.status == .running }.count
            guard active < max(1, min(limits.activeJobs, limits.retainedJobs)) else {
                throw RestRouteError(429, .conflict, "too many active jobs; retry after a running job finishes")
            }
            jobs[id] = job
            order.append(id)
            pruneLocked()
        }

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            self?.run(jobId: id, arguments: arguments, environment: environment)
        }
        return id
    }

    func runSync(arguments: [String], environment: [String: String] = [:]) throws -> (exitCode: Int32, stdout: String, stderr: String) {
        try JobProcessRunner.run(
            executable: resolveVzctl(), arguments: arguments,
            environment: Self.jobEnvironment(stateDirectory: stateDirectory, overrides: environment),
            timeout: limits.syncTimeout
        )
    }

    /// Cancellation stops the CLI process, not already applied stack resources.
    @discardableResult
    func cancel(_ id: String) -> Bool {
        lock.withLock {
            guard let job = jobs[id], job.status == .queued || job.status == .running else { return false }
            cancelledJobs.insert(id)
            return true
        }
    }

    /// LaunchAgents inherit a minimal PATH (`/usr/bin:/bin:…`). Homebrew tools
    /// such as `docker` live under `/opt/homebrew/bin` and must be visible to
    /// spawned `vzctl` apply jobs (e.g. `ensure_docker_context`).
    /// Image bake/seal/pull jobs also get `VZCTL_PROGRESS=1` so phase lines reach job logs.
    static func jobEnvironment(
        stateDirectory: URL,
        overrides: [String: String] = [:],
        processEnvironment: [String: String] = ProcessInfo.processInfo.environment
    ) -> [String: String] {
        var env = processEnvironment
        for (key, value) in overrides {
            env[key] = value
        }
        env["VZCTL_STATE_DIR"] = stateDirectory.path
        env["VZCTL_PROGRESS"] = "1"
        env["PATH"] = userToolPath(existing: env["PATH"])
        return env
    }

    static func userToolPath(existing: String?) -> String {
        var parts: [String] = []
        let preferred = [
            "\(NSHomeDirectory())/.local/bin",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ]
        let fallback = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"]
        let existingParts = (existing ?? "")
            .split(separator: ":", omittingEmptySubsequences: true)
            .map(String.init)
        for candidate in preferred + existingParts + fallback where !parts.contains(candidate) {
            parts.append(candidate)
        }
        return parts.joined(separator: ":")
    }

    /// Keep the newest `maxLogLines` entries (FIFO trim from the front).
    static func cappedLogAppending(existing: [String], lines: [String], limit: Int = maxLogLines) -> [String] {
        guard !lines.isEmpty else { return existing }
        var next = existing
        next.append(contentsOf: lines)
        if next.count > limit {
            next = Array(next.suffix(limit))
        }
        return next
    }

    private func run(jobId: String, arguments: [String], environment: [String: String]) {
        update(jobId) { job in
            job.status = .running
            job.updatedAt = isoNow()
        }
        do {
            let result = try runStreaming(jobId: jobId, arguments: arguments, environment: environment)
            let parsed = parseJSONValue(result.stdout)
            update(jobId) { job in
                job.updatedAt = isoNow()
                if cancelledJobs.contains(jobId) {
                    job.status = .failed
                    job.error = JobProcessError.cancelled.description
                } else if result.exitCode == 0 {
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

    private func runStreaming(
        jobId: String,
        arguments: [String],
        environment: [String: String]
    ) throws -> (exitCode: Int32, stdout: String, stderr: String) {
        let accumulator = LineAccumulator { [weak self] line in
            self?.appendLog(jobId, lines: [line])
        }
        defer { accumulator.flush() }
        return try JobProcessRunner.run(
            executable: resolveVzctl(), arguments: arguments,
            environment: Self.jobEnvironment(stateDirectory: stateDirectory, overrides: environment),
            timeout: limits.jobTimeout,
            cancelled: { self.lock.withLock { self.cancelledJobs.contains(jobId) } },
            onStderr: { accumulator.append($0) }
        )
    }

    private func appendLog(_ id: String, lines: [String]) {
        let filtered = lines.filter { !$0.isEmpty }
        guard !filtered.isEmpty else { return }
        update(id) { job in
            for line in filtered {
                Self.ingestLogLine(&job, line: line)
            }
            job.updatedAt = isoNow()
        }
    }

    static func ingestLogLine(_ job: inout RestJob, line rawLine: String) {
        let line = String(decoding: rawLine.utf8.prefix(maxLineBytes), as: UTF8.self)
        if let percent = progressPercent(in: line) {
            job.progressPercent = percent
            job.progressLabel = progressLabel(in: line)
            if let last = job.log.last, isProgressLine(last) {
                job.log[job.log.count - 1] = line
            } else {
                job.log.append(line)
            }
        } else {
            job.log.append(line)
        }
        if job.log.count > maxLogLines {
            job.log = Array(job.log.suffix(maxLogLines))
        }
    }

    static func isProgressLine(_ line: String) -> Bool {
        progressPercent(in: line) != nil
    }

    static func progressPercent(in line: String) -> Int? {
        guard let regex = try? NSRegularExpression(pattern: "(\\d{1,3})%\\s*$") else {
            return nil
        }
        let range = NSRange(line.startIndex..<line.endIndex, in: line)
        guard let match = regex.firstMatch(in: line, range: range),
              let inner = Range(match.range(at: 1), in: line),
              let value = Int(line[inner]),
              value <= 100
        else {
            return nil
        }
        return value
    }

    static func progressLabel(in line: String) -> String {
        guard let regex = try? NSRegularExpression(pattern: "\\s*\\d{1,3}%\\s*$") else {
            return line
        }
        let range = NSRange(line.startIndex..<line.endIndex, in: line)
        return regex.stringByReplacingMatches(in: line, options: [], range: range, withTemplate: "")
            .trimmingCharacters(in: .whitespaces)
    }

    private func update(_ id: String, mutate: (inout RestJob) -> Void) {
        lock.withLock {
            guard var job = jobs[id] else { return }
            mutate(&job)
            jobs[id] = job
            if job.status == .succeeded || job.status == .failed { cancelledJobs.remove(id) }
            pruneLocked()
        }
    }

    private func pruneLocked() {
        while jobs.count > max(1, limits.retainedJobs) {
            guard let index = order.firstIndex(where: {
                jobs[$0]?.status == .succeeded || jobs[$0]?.status == .failed
            }) else { return }
            let id = order.remove(at: index)
            jobs.removeValue(forKey: id)
            cancelledJobs.remove(id)
        }
    }

    func resolveVzctl() throws -> String {
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
        // PATH lookup (include Homebrew / ~/.local even under LaunchAgent PATH).
        let pathEnv = Self.userToolPath(existing: ProcessInfo.processInfo.environment["PATH"])
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

/// Accumulates pipe chunks into newline-delimited log lines.
///
/// curl `--progress-bar` and `qemu-img -p` rewrite a TTY meter with CR.
/// Treat CR as a line break and throttle those updates so the job log
/// shows live percent without overflowing the 500-line cap.
final class LineAccumulator: @unchecked Sendable {
    private let lock = NSLock()
    private var partial = Data()
    private var lineTruncated = false
    private var lastProgressAt: Date?
    private var pendingProgress: String?
    private let progressMinInterval: TimeInterval
    private let onLine: @Sendable (String) -> Void

    init(
        progressMinInterval: TimeInterval = 0.4,
        onLine: @escaping @Sendable (String) -> Void
    ) {
        self.progressMinInterval = progressMinInterval
        self.onLine = onLine
    }

    func append(_ data: Data) {
        lock.withLock {
            for byte in data {
                if byte == 10 || byte == 13 {
                    let line = takeLineLocked()
                    if byte == 13 {
                        if !line.isEmpty { emitProgressLocked(line) }
                    } else {
                        emitPendingProgressLocked()
                        if !line.isEmpty { onLine(line) }
                    }
                } else if partial.count < RestJobRunner.maxLineBytes {
                    partial.append(byte)
                } else {
                    lineTruncated = true
                }
            }
        }
    }

    func flush() {
        lock.withLock {
            emitPendingProgressLocked()
            let line = takeLineLocked()
            if !line.isEmpty { onLine(line) }
        }
    }

    private func takeLineLocked() -> String {
        let marker = lineTruncated ? " [line truncated]" : ""
        let line = String(
            decoding: partial.prefix(RestJobRunner.maxLineBytes - marker.utf8.count), as: UTF8.self
        ) + marker
        partial.removeAll(keepingCapacity: true)
        lineTruncated = false
        return line
    }

    private func emitProgressLocked(_ line: String) {
        let now = Date()
        if let last = lastProgressAt, now.timeIntervalSince(last) < progressMinInterval {
            pendingProgress = line
            return
        }
        pendingProgress = nil
        lastProgressAt = now
        onLine(line)
    }

    private func emitPendingProgressLocked() {
        guard let pending = pendingProgress else { return }
        pendingProgress = nil
        lastProgressAt = Date()
        onLine(pending)
    }
}

final class StringBox: @unchecked Sendable {
    private let lock = NSLock()
    private var lines: [String] = []

    func append(_ line: String) {
        lock.withLock {
            lines.append(String(decoding: line.utf8.prefix(RestJobRunner.maxLineBytes), as: UTF8.self))
            if lines.count > RestJobRunner.maxLogLines {
                lines = Array(lines.suffix(RestJobRunner.maxLogLines))
            }
        }
    }

    func joined(separator: String) -> String {
        lock.withLock { lines.joined(separator: separator) }
    }

    func snapshot() -> [String] {
        lock.withLock { lines }
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
            "log": .array(log.map { .string($0) }),
        ]
        if let result { obj["result"] = result }
        if let error { obj["error"] = .string(error) }
        if let percent = progressPercent {
            var progress: [String: JSONValue] = ["percent": .number(Double(percent))]
            if let progressLabel { progress["label"] = .string(progressLabel) }
            obj["progress"] = .object(progress)
        }
        return .object(obj)
    }
}
