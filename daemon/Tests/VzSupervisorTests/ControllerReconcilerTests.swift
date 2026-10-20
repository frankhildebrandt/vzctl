import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

struct ControllerReconcilerTests {
    final class Spy: @unchecked Sendable {
        let lock = NSLock()
        var calls: [String] = []
        var events: [JSONValue] = []
        func call(_ project: String) { lock.withLock { calls.append(project) } }
        func event(_ value: JSONValue) { lock.withLock { events.append(value) } }
        var callCount: Int { lock.withLock { calls.count } }
        var eventCount: Int { lock.withLock { events.count } }
    }

    @Test func registrationIsExplicitPrivatePersistentAndGenerationBound() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data("spec: {}".utf8).write(to: config)
        let service = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        #expect(service.registration(project: "p") == .object(["enabled": .bool(false), "generation": .null]))
        let first = try service.register(project: "p", configPath: config.path)
        let attrs = try FileManager.default.attributesOfItem(atPath: root.appendingPathComponent("controller-registry.json").path)
        #expect((attrs[.posixPermissions] as? NSNumber)?.intValue == 0o600)
        let restored = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        #expect(restored.registration(project: "p") == .object(["enabled": .bool(true), "generation": .string(first.generation), "config_path": .string(config.path)]))
        let second = try service.register(project: "p", configPath: config.path)
        #expect(first.generation != second.generation)
        try service.unregister(configPath: config.path)
        let disabled = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        #expect(disabled.registration(project: "p") == .object(["enabled": .bool(false), "generation": .null]))
        #expect(FileManager.default.fileExists(atPath: config.path))
    }

    @Test func activeWorkerIsBoundedAndUnregisterCancelsWithoutRegistryDeadlock() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let spy = Spy()
        let service = ControllerReconciler(stateDirectory: root, worker: { record, cancelled in
            spy.call(record.project)
            let until = Date().addingTimeInterval(3)
            while !cancelled() && Date() < until { Thread.sleep(forTimeInterval: 0.005) }
            #expect(cancelled())
            throw JobProcessError.cancelled
        }, event: { _, data in spy.event(.object(data)) })
        _ = try service.register(project: "p", configPath: config.path)
        service.tick() // Newly authorized apply has a 30-second head start.
        #expect(spy.callCount == 0)
        service.tick(now: Date().addingTimeInterval(60))
        try awaitCondition { spy.callCount == 1 }
        for _ in 0..<100 { service.tick(now: Date().addingTimeInterval(60)) }
        #expect(spy.callCount == 1)
        try service.unregister(project: "p")
        service.tick(now: Date().addingTimeInterval(60))
        #expect(spy.callCount == 1)
        #expect(spy.eventCount == 0, "cancelled stale generations must not publish status")
    }

    @Test func schedulerIsFairAndOnlyEmitsSemanticChanges() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let spy = Spy()
        let service = ControllerReconciler(stateDirectory: root, interval: 0, worker: { record, _ in
            spy.call(record.project)
            return .object(["observedAt": .number(Double(spy.callCount)), "claims": .array([
                .object(["name": .string("db"), "kind": .string("postgres"), "state": .string("Ready"),
                         "lastSuccess": .number(Double(spy.callCount)), "secrets": .string("PRIVATE")]),
            ])])
        }, event: { _, data in spy.event(.object(data)) })
        for project in ["a", "b", "c"] { _ = try service.register(project: project, configPath: config.path) }
        for count in 1...3 {
            service.tick()
            try awaitCondition { spy.eventCount == count }
        }
        #expect(spy.lock.withLock { spy.calls } == ["a", "b", "c"])
        service.tick()
        try awaitCondition { spy.callCount == 4 }
        service.stop()
        #expect(spy.eventCount == 3)
        let data = try JSONEncoder().encode(spy.lock.withLock { spy.events })
        #expect(!String(decoding: data, as: UTF8.self).contains("PRIVATE"))
    }

    @Test func unsafeRegistryFailsClosed() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        try FileManager.default.createSymbolicLink(at: root.appendingPathComponent("controller-registry.json"), withDestinationURL: config)
        let service = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        #expect(throws: (any Error).self) { _ = try service.register(project: "p", configPath: config.path) }
        #expect(service.registration(project: "p") == .object(["enabled": .bool(false), "generation": .null]))
    }

    @Test func statusEventsIdentifyRunGenerationAndClaimSourceWithoutCredentials() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let spy = Spy()
        let service = ControllerReconciler(stateDirectory: root, interval: 0, worker: { _, _ in
            spy.call("p")
            return .object(["claims": .array([.object([
                "name": .string("db"), "kind": .object(["password": .string("PRIVATE")]),
                "state": .string(spy.callCount == 1 ? "Degraded" : "Ready"),
                "observedGeneration": .number(7),
                "source": .object(["type": .string("kubernetes"), "cluster": .string("dev"),
                    "namespace": .string("app"), "name": .string("database"), "uid": .string("claim-1"),
                    "credentials": .string("PRIVATE")]),
                "binding": .object(["password": .string("PRIVATE")]),
                "conditions": .array([.object(["type": .string("Ready"),
                    "status": .string("False"), "reason": .object(["password": .string("PRIVATE")])])]),
            ])])])
        }, event: { _, data in spy.event(.object(data)) })
        let registration = try service.register(project: "p", configPath: config.path)
        for count in 1...2 {
            service.tick()
            try awaitCondition { spy.eventCount == count }
        }
        service.stop()
        var correlations = Set<String>()
        for value in spy.lock.withLock({ spy.events }) {
            guard case let .object(event) = value,
                  case let .string(correlation)? = event["correlation"],
                  case let .object(status)? = event["status"],
                  case let .array(claims)? = status["claims"],
                  case let .object(claim)? = claims.first
            else { Issue.record("missing event identity or claims"); continue }
            #expect(event["project"] == .string("p"))
            #expect(event["generation"] == .string(registration.generation))
            #expect(UUID(uuidString: correlation) != nil)
            correlations.insert(correlation)
            #expect(claim["source"] == .object(["type": .string("kubernetes"), "cluster": .string("dev"),
                "namespace": .string("app"), "name": .string("database"), "uid": .string("claim-1")]))
            #expect(claim["observedGeneration"] == .number(7))
        }
        #expect(correlations.count == 2)
        let encoded = try JSONEncoder().encode(spy.lock.withLock { spy.events })
        #expect(!String(decoding: encoded, as: UTF8.self).contains("PRIVATE"))
    }

    @Test func rpcAndUIUnregisterUseTheSamePersistedAuthorization() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let server = try SupervisorServer(stateDirectory: root)
        let registered = server.dispatchRPC(method: "controller.register", params: .object(["project": .string("p"), "config_path": .string(config.path)]))
        #expect(registered.error == nil)
        guard case let .object(value)? = registered.result else { Issue.record("missing registration"); return }
        #expect(value["enabled"] == .bool(true))
        #expect(value["config_path"] == .string(config.path))
        try server.database.upsertStack(StackRegistryRecord(id: "p", path: root.path, name: "p", openedAt: "2026-10-20T00:00:00Z"))
        let router = RestRouter(jobs: RestJobRunner(stateDirectory: root), database: server.database, stateDirectory: root)
        router.server = server
        let response = router.handle(RestHTTPRequest(method: "DELETE", path: "/v1/stacks/p", query: [:], headers: [:], body: Data()))
        #expect(response.status == 204)
        let disabled = server.dispatchRPC(method: "controller.registration", params: .object(["project": .string("p")]))
        #expect(disabled.result == .object(["enabled": .bool(false), "generation": .null]))
        #expect(FileManager.default.fileExists(atPath: config.path))
    }

    @Test func timeoutIsPersistedAsAStableRedactedWarning() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let spy = Spy()
        let service = ControllerReconciler(stateDirectory: root, interval: 0, worker: { _, _ in
            throw JobProcessError.timedOut
        }, event: { _, data in spy.event(.object(data)) })
        _ = try service.register(project: "p", configPath: config.path)
        service.tick()
        try awaitCondition { spy.eventCount == 1 }
        service.stop()
        let path = root.appendingPathComponent("controller-status.json")
        let value = try JSONDecoder().decode([String: JSONValue].self, from: Data(contentsOf: path))
        guard case let .object(record)? = value["p"] else { Issue.record("missing attempt status"); return }
        #expect(record["lastStatus"] == .object(["state": .string("Degraded"), "reason": .string("ReconcileDeadlineExceeded")]))
        #expect(record["lastAttemptAt"] != nil)
        let restored = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        guard case let .object(registration) = restored.registration(project: "p") else { Issue.record("missing registration"); return }
        #expect(registration["lastStatus"] == record["lastStatus"])
        #expect(registration["lastAttemptAt"] == record["lastAttemptAt"])
        let attrs = try FileManager.default.attributesOfItem(atPath: path.path)
        #expect((attrs[.posixPermissions] as? NSNumber)?.intValue == 0o600)
    }

    @Test func projectNamesMatchConfigAndErrorsUseOnlyKnownCodes() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let service = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        _ = try service.register(project: "prod.demo_1-test", configPath: config.path)
        for invalid in [".hidden", "-dash", "_under", "slash/name", "", String(repeating: "x", count: 64)] {
            #expect(throws: (any Error).self) { _ = try service.register(project: invalid, configPath: config.path) }
        }
        for code in ["LeaseHeld", "RegistrationRevoked", "RegistrationUnavailable", "InvalidConfig"] {
            #expect(ControllerReconciler.publicSummary(.object(["code": .string(code), "message": .string("PRIVATE")])) == .object(["state": .string("Degraded"), "reason": .string(code)]))
        }
        #expect(ControllerReconciler.publicSummary(.object(["code": .string("PRIVATE"), "message": .string("PRIVATE")])) == .object(["state": .string("Degraded"), "reason": .string("ReconcileFailed")]))
    }

    @Test func unchangedStatusRefreshesAttemptTimeWithoutEventAndNewGenerationClearsIt() throws {
        let root = try temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: root) }
        let config = root.appendingPathComponent("hypernetwork.config.yaml")
        try Data().write(to: config)
        let initial = ControllerReconciler(stateDirectory: root, event: { _, _ in })
        _ = try initial.register(project: "p", configPath: config.path)
        let summary: JSONValue = .object(["claims": .array([]), "warnings": .number(0)])
        let status = root.appendingPathComponent("controller-status.json")
        try JSONEncoder().encode(["p": ["lastStatus": summary, "lastAttemptAt": .number(1)]]).write(to: status)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: status.path)
        let spy = Spy()
        let service = ControllerReconciler(stateDirectory: root, worker: { _, _ in .object(["claims": .array([])]) }, event: { _, data in spy.event(.object(data)) })
        service.tick(now: Date().addingTimeInterval(60))
        try awaitCondition {
            guard case let .object(record) = service.registration(project: "p"), case let .number(time)? = record["lastAttemptAt"] else { return false }
            return time > 1
        }
        #expect(spy.eventCount == 0)
        _ = try service.register(project: "p", configPath: config.path)
        guard case let .object(record) = service.registration(project: "p") else { Issue.record("missing registration"); return }
        #expect(record["lastStatus"] == nil)
        #expect(record["lastAttemptAt"] == nil)
        service.stop()
    }

    private func temporaryDirectory() throws -> URL {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("vzc-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
        return root
    }
    private func awaitCondition(_ ready: () -> Bool) throws {
        let until = Date().addingTimeInterval(3)
        while !ready() && Date() < until { Thread.sleep(forTimeInterval: 0.005) }
        #expect(ready())
    }
}
