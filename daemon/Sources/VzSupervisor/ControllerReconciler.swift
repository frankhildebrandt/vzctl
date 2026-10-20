import Darwin
import Foundation
import VzDaemonKit

/// Explicitly authorized stack sources only. One worker globally, no pending-job queue.
final class ControllerReconciler: @unchecked Sendable {
    struct Registration: Codable, Sendable {
        let project: String
        let configPath: String
        let generation: String
    }
    typealias Worker = @Sendable (Registration, @escaping @Sendable () -> Bool) throws -> JSONValue
    typealias Event = @Sendable (String, [String: JSONValue]) -> Void
    private struct Active {
        let registration: Registration
        let completion: DispatchGroup
        let correlation = UUID().uuidString.lowercased()
    }
    private let lock = NSLock()
    private let registryURL: URL
    private let statusURL: URL
    private let interval: TimeInterval
    private let worker: Worker
    private let event: Event
    private var registrations: [String: Registration] = [:]
    private var eligibleAt: [String: Date] = [:]
    private var lastSummary: [String: JSONValue] = [:]
    private var lastAttemptAt: [String: JSONValue] = [:]
    private var active: Active?
    private var stopped = false
    private var lastProject = ""
    private var timer: DispatchSourceTimer?
    private var registryError: String?

    init(stateDirectory: URL, interval: TimeInterval = 30, worker: Worker? = nil, event: @escaping Event) {
        registryURL = stateDirectory.appendingPathComponent("controller-registry.json")
        statusURL = stateDirectory.appendingPathComponent("controller-status.json")
        self.interval = interval
        self.event = event
        self.worker = worker ?? { registration, cancelled in
            let jobs = RestJobRunner(stateDirectory: stateDirectory)
            let result = try JobProcessRunner.run(
                executable: jobs.resolveVzctl(),
                arguments: ["controller", "reconcile", "-C", registration.configPath,
                            "--registered", registration.generation, "--format", "json"],
                environment: RestJobRunner.jobEnvironment(stateDirectory: stateDirectory, overrides: [:]),
                timeout: 300, stdoutLimit: 1_048_576, stderrLimit: 65_536, cancelled: cancelled
            )
            guard let data = result.stdout.data(using: .utf8),
                  let value = try? JSONDecoder().decode(JSONValue.self, from: data),
                  case let .object(envelope) = value,
                  let payload = envelope["data"]
            else { throw ReconcileRPCError.invalid("controller worker returned no valid status") }
            return payload
        }
        do {
            var info = stat()
            if lstat(registryURL.path, &info) == 0 {
                guard info.st_mode & S_IFMT == S_IFREG, info.st_uid == geteuid(), info.st_mode & 0o077 == 0,
                      info.st_size <= 262_144 else { throw ReconcileRPCError.invalid("unsafe controller registry") }
                let values = try JSONDecoder().decode([String: Registration].self, from: Data(contentsOf: registryURL))
                guard values.count <= 128, values.allSatisfy({ $0.key == $0.value.project && Self.validProject($0.key) && $0.value.configPath.hasPrefix("/") && $0.value.configPath.utf8.count <= 1024 && UUID(uuidString: $0.value.generation) != nil }) else {
                    throw ReconcileRPCError.invalid("invalid controller registry")
                }
                registrations = values
                eligibleAt = values.mapValues { _ in Date().addingTimeInterval(interval) }
            } else if errno != ENOENT { throw ReconcileRPCError.invalid("controller registry unavailable") }
        } catch { registryError = "controller registry is unavailable or unsafe; automatic reconcile disabled" }
        var statusInfo = stat()
        if lstat(statusURL.path, &statusInfo) == 0, statusInfo.st_mode & S_IFMT == S_IFREG,
           statusInfo.st_uid == geteuid(), statusInfo.st_mode & 0o077 == 0, statusInfo.st_size <= 1_048_576,
           let data = try? Data(contentsOf: statusURL),
           let statuses = try? JSONDecoder().decode([String: JSONValue].self, from: data) {
            for (project, value) in statuses where registrations[project] != nil {
                if case let .object(record) = value, let summary = record["lastStatus"] {
                    lastSummary[project] = summary
                    lastAttemptAt[project] = record["lastAttemptAt"]
                } else { lastSummary[project] = value } // Previous status-file format.
            }
        }
    }

    func start() {
        let source = DispatchSource.makeTimerSource(queue: DispatchQueue(label: "vzctl.controllers.timer"))
        let shouldStart = lock.withLock { () -> Bool in
            guard timer == nil, !stopped else { return false }
            timer = source
            return true
        }
        guard shouldStart else { return }
        source.schedule(deadline: .now() + interval, repeating: interval)
        source.setEventHandler { [weak self] in self?.tick() }
        source.resume()
        if let registryError { event("controller.registry.failed", ["reason": .string(registryError)]) }
    }

    func stop() {
        let state = lock.withLock { () -> (DispatchSourceTimer?, DispatchGroup?) in
            stopped = true
            let state = (timer, active?.completion)
            timer = nil
            return state
        }
        state.0?.cancel()
        _ = state.1?.wait(timeout: .now() + 5)
    }

    @discardableResult
    func register(project: String, configPath: String) throws -> Registration {
        guard Self.validProject(project) else { throw ReconcileRPCError.invalid("invalid controller project") }
        let path = URL(fileURLWithPath: configPath).standardizedFileURL.resolvingSymlinksInPath().path
        var directory: ObjCBool = false
        guard path.hasPrefix("/"), path.utf8.count <= 1024, FileManager.default.fileExists(atPath: path, isDirectory: &directory), !directory.boolValue else {
            throw ReconcileRPCError.invalid("controller config_path must be an existing file")
        }
        return try lock.withLock {
            guard registryError == nil else { throw ReconcileRPCError.invalid(registryError!) }
            guard registrations[project] != nil || registrations.count < 128 else { throw ReconcileRPCError.invalid("controller registration limit reached") }
            let record = Registration(project: project, configPath: path, generation: UUID().uuidString.lowercased())
            var next = registrations
            next[project] = record
            try persist(next)
            registrations = next // Invalidates an old worker before the new apply proceeds.
            eligibleAt[project] = Date().addingTimeInterval(interval)
            lastSummary.removeValue(forKey: project)
            lastAttemptAt.removeValue(forKey: project)
            persistStatuses()
            return record
        }
    }

    func registration(project: String) -> JSONValue {
        lock.withLock {
            guard let record = registrations[project], registryError == nil, !stopped else {
                return .object(["enabled": .bool(false), "generation": .null])
            }
            var result: [String: JSONValue] = ["enabled": .bool(true), "generation": .string(record.generation), "config_path": .string(record.configPath)]
            result["lastStatus"] = lastSummary[project]
            result["lastAttemptAt"] = lastAttemptAt[project]
            return .object(result)
        }
    }

    func unregister(project: String) throws {
        let completion = try lock.withLock { () throws -> DispatchGroup? in
            guard registryError == nil else { throw ReconcileRPCError.invalid(registryError!) }
            var next = registrations
            next.removeValue(forKey: project)
            try persist(next)
            registrations = next
            eligibleAt.removeValue(forKey: project)
            lastSummary.removeValue(forKey: project)
            lastAttemptAt.removeValue(forKey: project)
            persistStatuses()
            return active?.registration.project == project ? active?.completion : nil
        }
        // Never wait while holding the registry lock: workers make supervisor RPCs.
        if completion?.wait(timeout: .now() + 5) == .timedOut {
            throw ReconcileRPCError.invalid("controller disabled; worker cancellation is still completing")
        }
    }

    func unregister(configPath: String) throws {
        let path = URL(fileURLWithPath: configPath).standardizedFileURL.resolvingSymlinksInPath().path
        let projects = lock.withLock { registrations.values.filter { $0.configPath == path }.map(\.project) }
        for project in projects { try unregister(project: project) }
    }

    func tick(now: Date = Date()) {
        let selected: Active? = lock.withLock {
            guard !stopped, registryError == nil, active == nil else { return nil }
            let candidates = registrations.values.filter { (eligibleAt[$0.project] ?? .distantFuture) <= now }
                .sorted { $0.project < $1.project }
            guard let registration = candidates.first(where: { $0.project > lastProject }) ?? candidates.first else { return nil }
            lastProject = registration.project
            let completion = DispatchGroup()
            completion.enter()
            let value = Active(registration: registration, completion: completion)
            active = value
            return value
        }
        guard let selected else { return }
        DispatchQueue.global(qos: .utility).async { [self] in
            defer { selected.completion.leave() }
            let registration = selected.registration
            let cancelled: @Sendable () -> Bool = { [self] in
                lock.withLock { stopped || registrations[registration.project]?.generation != registration.generation }
            }
            let summary: JSONValue
            do { summary = Self.publicSummary(try worker(registration, cancelled)) }
            catch {
                let reason: String
                switch error {
                case JobProcessError.timedOut: reason = "ReconcileDeadlineExceeded"
                case JobProcessError.cancelled: reason = "ReconcileCancelled"
                default: reason = "ReconcileFailed"
                }
                summary = .object(["state": .string("Degraded"), "reason": .string(reason)])
            }
            let changed = lock.withLock { () -> Bool in
                active = nil
                guard !stopped, registrations[registration.project]?.generation == registration.generation else { return false }
                eligibleAt[registration.project] = Date().addingTimeInterval(interval)
                let changed = lastSummary[registration.project] != summary
                lastSummary[registration.project] = summary
                lastAttemptAt[registration.project] = .number(Date().timeIntervalSince1970.rounded(.down))
                persistStatuses()
                return changed
            }
            if changed {
                event("controller.status.changed", [
                    "project": .string(registration.project),
                    "generation": .string(registration.generation),
                    "correlation": .string(selected.correlation), "status": summary,
                ])
            }
        }
    }

    /// Called under lock. Time changes are persisted without producing semantic events.
    private func persistStatuses() {
        let values = lastSummary.mapValues { _ in [String: JSONValue]() }
        var records = values
        for (project, summary) in lastSummary {
            records[project] = ["lastStatus": summary, "lastAttemptAt": lastAttemptAt[project] ?? .null]
        }
        if let data = try? JSONEncoder().encode(records) {
            try? data.write(to: statusURL, options: [.atomic])
            _ = chmod(statusURL.path, 0o600)
        }
    }

    private func persist(_ values: [String: Registration]) throws {
        let data = try JSONEncoder().encode(values)
        try data.write(to: registryURL, options: [.atomic])
        guard chmod(registryURL.path, 0o600) == 0 else { throw ReconcileRPCError.invalid("cannot protect controller registry") }
    }

    private static func validProject(_ value: String) -> Bool {
        let alphanumeric: (UInt8) -> Bool = { ($0 >= 48 && $0 <= 57) || ($0 >= 65 && $0 <= 90) || ($0 >= 97 && $0 <= 122) }
        return !value.isEmpty && value.utf8.count <= 63 && value.utf8.first.map(alphanumeric) == true && value.utf8.allSatisfy {
            alphanumeric($0) || $0 == 45 || $0 == 95 || $0 == 46
        }
    }

    static func publicSummary(_ value: JSONValue) -> JSONValue {
        guard case let .object(payload) = value else {
            return .object(["state": .string("Degraded"), "reason": .string("InvalidStatus")])
        }
        if case let .string(code)? = payload["code"] {
            let known: Set<String> = ["LeaseHeld", "RegistrationRevoked", "RegistrationUnavailable", "InvalidConfig", "InvalidState", "SecretStorage", "GuestUnavailable", "KubernetesUnavailable", "InvalidOperation", "Usage"]
            return .object(["state": .string("Degraded"), "reason": .string(known.contains(code) ? code : "ReconcileFailed")])
        }
        guard case let .array(claims)? = payload["claims"] else {
            return .object(["state": .string("Degraded"), "reason": .string("InvalidStatus")])
        }
        let output = claims.prefix(256).map { claim -> JSONValue in
            guard case let .object(item) = claim else { return .object(["state": .string("Unknown")]) }
            var result: [String: JSONValue] = [:]
            for key in ["name", "kind", "state", "observedGeneration"] { result[key] = publicScalar(item[key]) }
            if case let .object(source)? = item["source"] {
                var reference: [String: JSONValue] = [:]
                for key in ["type", "cluster", "namespace", "name", "uid"] {
                    if case let .string(value)? = source[key] {
                        reference[key] = publicScalar(.string(value))
                    }
                }
                result["source"] = .object(reference)
            }
            if case let .array(conditions)? = item["conditions"] {
                result["conditions"] = .array(conditions.prefix(8).compactMap { condition in
                    guard case let .object(data) = condition else { return nil }
                    return .object(Dictionary(uniqueKeysWithValues: ["type", "status", "reason"].map {
                        ($0, publicScalar(data[$0]))
                    }))
                })
            }
            return .object(result)
        }
        let warningCount: Int
        if case let .array(warnings)? = payload["warnings"] { warningCount = warnings.count } else { warningCount = 0 }
        return .object(["claims": .array(output), "warnings": .number(Double(warningCount))])
    }

    private static func publicScalar(_ value: JSONValue?) -> JSONValue {
        switch value {
        case let .string(text):
            let lower = text.lowercased()
            if ["password", "secret", "token", "authorization", "private key", "://", "@"].contains(where: lower.contains) {
                return .string("[redacted]")
            }
            return .string(String(text.filter {
                $0.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
            }.prefix(160)))
        case .number, .bool: return value ?? .null
        default: return .null
        }
    }
}
