import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

struct RestStatusBundleTests {
    @Test func runningInventoryPreservesDegradedCanonicalHealth() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let cli = dir.appendingPathComponent("fake-vzctl")
        try """
        #!/bin/sh
        printf '%s\\n' "$1" >> "$VZCTL_STATE_DIR/calls"
        if [ "$1" = stack ]; then
          printf '%s' '{"status":"warn","summary":{"health":"degraded"},"stack":{"project":"demo","oidc":{"enabled":false}},"vms":[{"id":"demo/web","name":"web","state":"running"}],"warnings":[{"code":"AGENT_DEGRADED","message":"demo/web agent is degraded"}]}'
          exit 1
        fi
        printf '{"status":"ok"}'
        """.write(to: cli, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: cli.path)
        // Valid YAML in inline form: names must come from the CLI's parsed
        // configuration rather than an indentation-sensitive Swift scanner.
        try "spec: {project: demo, vms: {web: {from: base}}}".write(
            to: dir.appendingPathComponent("hypernetwork.config.yaml"), atomically: true, encoding: .utf8
        )
        let result = try RestStatusBundle.build(
            path: dir.path, jobs: RestJobRunner(stateDirectory: dir, vzctlPath: cli.path),
            rpcVMs: { Issue.record("canonical VM inventory should be used"); return .array([]) }
        )
        guard case let .object(root) = result, case let .object(sections)? = root["sections"],
              case let .object(health)? = sections["health"],
              case let .object(report)? = health["data"],
              case let .object(summary)? = report["summary"],
              case let .object(stack)? = sections["stack"],
              case let .object(inventory)? = stack["data"],
              case let .object(oidc)? = sections["oidc"],
              case let .object(oidcData)? = oidc["data"]
        else { Issue.record("missing sections"); return }
        #expect(summary["health"] == .string("degraded"))
        #expect(health["exit_code"] == .number(1))
        #expect(inventory["phase"] == .string("running"))
        #expect(inventory["project"] == .string("demo"))
        #expect(oidcData["enabled"] == .bool(false))
        let calls = try String(contentsOf: dir.appendingPathComponent("calls"), encoding: .utf8)
        #expect(!calls.contains("vm\n"))
        #expect(!calls.contains("oidc\n"))
    }
}

struct RestApplyJournalProjectionTests {
    @Test func persistedFailureUsesCanonicalStackAndNeverPublishesPayload() throws {
        let journal = JournalRecord(id: "7", stackID: "demo:stack", generation: 7,
            step: "ensure_containers", status: "failed",
            payload: #"{"mode":"apply","desired_hash":"private-hash","config":"PRIVATE-CREDENTIAL"}"#,
            error: "container provision failed", createdAt: "created", updatedAt: "updated")
        var requested: String?
        let section = RestStatusBundle.applySection(stackID: "demo:stack") { id in
            requested = id
            return StackStateRecord(resources: [], journal: journal,
                lease: LeaseRecord(holder: "host:123", expiresAt: Date(), expiresAtText: "expiry"))
        }
        #expect(requested == "demo:stack")
        let raw = String(decoding: try JSONEncoder().encode(section), as: UTF8.self)
        #expect(raw.contains("ensure_containers"))
        #expect(raw.contains("container provision failed"))
        #expect(raw.contains("host:123"))
        #expect(!raw.contains("PRIVATE-CREDENTIAL"))
        #expect(!raw.contains("desired_hash"))
        #expect(!raw.contains("payload"))
    }

    @Test func unavailableIsDistinctFromNoJournalAndDoesNotGuessStackIdentity() throws {
        let missing = RestStatusBundle.applySection(stackID: nil) { _ in
            Issue.record("must not guess identity"); return StackStateRecord(resources: [], journal: nil, lease: nil)
        }
        #expect(missing == .object(["ok": .bool(false), "data": .null]))
        let failed = RestStatusBundle.applySection(stackID: "demo:stack") { _ in
            throw CocoaError(.fileReadUnknown)
        }
        #expect(failed == missing)
        let empty = RestStatusBundle.applySection(stackID: "demo:stack") { _ in
            StackStateRecord(resources: [], journal: nil, lease: nil)
        }
        #expect(empty == .object(["ok": .bool(true), "data": .object([
            "stack_id": .string("demo:stack"), "journal": .null, "lease": .null,
        ])]))
    }

    @Test func foreignJournalIsRejectedAndDiagnosticsAreBounded() throws {
        func journal(stack: String) -> JournalRecord {
            JournalRecord(id: "7", stackID: stack, generation: 7, step: "ensure_containers", status: "failed",
                payload: "{}", error: String(repeating: "x", count: 6000), createdAt: "a", updatedAt: "b")
        }
        let foreign = RestStatusBundle.applySection(stackID: "demo:stack") { _ in
            StackStateRecord(resources: [], journal: journal(stack: "foreign:stack"), lease: nil)
        }
        #expect(foreign == .object(["ok": .bool(false), "data": .null]))
        let bounded = RestStatusBundle.applySection(stackID: "demo:stack") { _ in
            StackStateRecord(resources: [], journal: journal(stack: "demo:stack"), lease: nil)
        }
        let raw = String(decoding: try JSONEncoder().encode(bounded), as: UTF8.self)
        #expect(!raw.contains(String(repeating: "x", count: 4097)))
    }
}

struct RestPersistedApplyJournalTests {
    @Test func statusRouteShowsFailedCLIJournalAfterReopeningDatabaseWithoutMutatingIt() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let databasePath = dir.appendingPathComponent("state.sqlite").path
        let writer = try StateDatabase(path: databasePath)
        try writer.upsertStack(StackRegistryRecord(id: "ui-entry", path: dir.path, name: "stack", openedAt: "now"))
        let owner = "test:\(ProcessInfo.processInfo.processIdentifier)"
        let journal = try writer.beginApply(stackID: "demo:stack", holder: owner,
            desiredHash: "private-hash", payload: #"{"mode":"apply","config":"PRIVATE-CREDENTIAL"}"#, resume: false)
        _ = try writer.advanceApply(id: journal.id, stackID: "demo:stack", holder: owner,
            step: "ensure_containers", status: "failed", error: "container create failed")
        let before = try writer.stackState(stackID: "demo:stack").json
        let cli = dir.appendingPathComponent("fake-cli")
        try """
        #!/bin/sh
        printf '%s\\n' "$1" >> "$VZCTL_STATE_DIR/calls"
        if [ "$1" = stack ]; then
          printf '%s' '{"status":"ok","summary":{"health":"ok"},"stack":{"project":"demo","oidc":{"enabled":false}},"vms":[]}'
        elif [ "$1" = diff ]; then
          printf '%s' '{"status":"ok","stack_id":"demo:stack","actions":[]}'
        else
          printf '%s' '{"status":"ok"}'
        fi
        """.write(to: cli, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: cli.path)
        try "metadata: {name: stack}\nspec: {project: demo}".write(
            to: dir.appendingPathComponent("hypernetwork.config.yaml"), atomically: true, encoding: .utf8)
        let reopened = try StateDatabase(path: databasePath)
        let router = RestRouter(jobs: RestJobRunner(stateDirectory: dir, vzctlPath: cli.path),
            database: reopened, stateDirectory: dir)
        let response = router.handle(RestHTTPRequest(method: "GET", path: "/v1/stacks/ui-entry/status",
            query: [:], headers: [:], body: Data()))
        #expect(response.status == 200)
        let body = try JSONDecoder().decode(JSONValue.self, from: response.body)
        guard case let .object(root) = body, case let .object(sections)? = root["sections"],
              case let .object(apply)? = sections["apply"], case let .object(data)? = apply["data"],
              case let .object(saved)? = data["journal"]
        else { Issue.record("persisted apply journal missing from REST status"); return }
        #expect(saved["id"] == .string(journal.id))
        #expect(saved["status"] == .string("failed"))
        #expect(saved["step"] == .string("ensure_containers"))
        #expect(saved["error"] == .string("container create failed"))
        #expect(!String(decoding: response.body, as: UTF8.self).contains("PRIVATE-CREDENTIAL"))
        #expect(try reopened.stackState(stackID: "demo:stack").json == before)
        let commands = try String(contentsOf: dir.appendingPathComponent("calls"), encoding: .utf8)
        #expect(!commands.split(separator: "\n").contains("apply"))
    }
}
