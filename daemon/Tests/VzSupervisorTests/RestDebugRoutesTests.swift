import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

struct RestDebugRoutesTests {
    // Opt in with VZCTL_TEST_CLI=<built binary>: exercises the real parser, no daemon/VM.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["VZCTL_TEST_CLI"] != nil))
    func debugRoutesAreAcceptedByTheRealCLI() throws {
        let cli = try #require(ProcessInfo.processInfo.environment["VZCTL_TEST_CLI"])
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        try """
        apiVersion: hypernetwork/v1
        kind: Environment
        metadata: { name: debug-test }
        spec:
          project: debug-test
          domain: debug-test.vz.test
          dns: { enabled: false, hostResolver: false, hostListen: "127.0.0.1:15353", forward: { enabled: true, upstream: system } }
          routes: []
          policies: []
          images: {}
          networks: {}
          vms: {}
        """.write(to: dir.appendingPathComponent("hypernetwork.config.yaml"), atomically: true, encoding: .utf8)
        let database = try StateDatabase(path: dir.appendingPathComponent("state.sqlite").path)
        try database.upsertStack(StackRegistryRecord(
            id: "test", path: dir.path, name: "test", openedAt: "2026-10-20T00:00:00Z"
        ))
        let router = RestRouter(jobs: RestJobRunner(stateDirectory: dir, vzctlPath: cli),
                                database: database, stateDirectory: dir)
        let cases: [(String, String, String, String, Int)] = [
            ("GET", "sources", "", "debug.sources.list", 0),
            ("GET", "sources/list", "", "debug.sources.list", 0),
            ("GET", "doctor", "", "debug.doctor", 3),
            ("POST", "postgres/query", #"{"sql":"SELECT 1","source":"missing","maxRows":3,"timeoutMs":100}"#, "debug.postgres.query", 3),
            ("POST", "mysql/query", #"{"sql":"SELECT 1","source":"missing","maxRows":3,"timeoutMs":100}"#, "debug.mysql.query", 3),
            ("GET", "nats/streams", "", "debug.nats.streams", 3),
            ("POST", "nats/sub", #"{"subject":"test","count":1,"timeoutMs":100}"#, "debug.nats.sub", 3),
            ("POST", "nats/kv/get", #"{"bucket":"test","key":"key"}"#, "debug.nats.kv.get", 3),
            ("POST", "nats/kv/watch", #"{"bucket":"test","key":"key","timeoutMs":100}"#, "debug.nats.kv.watch", 3),
        ]
        for (method, path, body, command, exit) in cases {
            let response = router.handle(RestHTTPRequest(method: method, path: "/v1/stacks/test/debug/\(path)",
                query: [:], headers: [:], body: Data(body.utf8)))
            #expect(response.status == 200)
            let envelope = try #require(JSONSerialization.jsonObject(with: response.body) as? [String: Any])
            #expect(envelope["command"] as? String == command, "\(path): \(envelope)")
            #expect(envelope["exit_code"] as? Int == exit, "\(path): \(envelope)")
            if exit == 0 {
                #expect(envelope["status"] as? String == "ok")
                #expect(envelope["sources"] != nil)
            } else {
                // Parsed successfully, then rejected the empty fixture before contacting any guest.
                #expect(envelope["status"] as? String == "fail")
            }
        }
    }

    @Test func clientDebugURLsReachTheCLIThroughTheRouter() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let cli = dir.appendingPathComponent("fake-vzctl")
        try """
        #!/bin/sh
        printf '%s\\n' "$@" > "$VZCTL_STATE_DIR/argv"
        printf '{"status":"ok","data":{"value":1}}'
        """.write(to: cli, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: cli.path)
        let database = try StateDatabase(path: dir.appendingPathComponent("state.sqlite").path)
        try database.upsertStack(StackRegistryRecord(
            id: "my/stack", path: dir.path, name: "test", openedAt: "2026-10-20T00:00:00Z"
        ))
        let router = RestRouter(
            jobs: RestJobRunner(stateDirectory: dir, vzctlPath: cli.path),
            database: database, stateDirectory: dir
        )
        let cases: [(String, String, [String: String], String, [String])] = [
            ("GET", "sources", ["type": "postgres"], "", ["sources", "list", "--type", "postgres"]),
            ("GET", "sources/list", [:], "", ["sources", "list"]),
            ("GET", "doctor", [:], "", ["doctor"]),
            ("POST", "postgres/query", [:], #"{"sql":"SELECT 1","source":"db"}"#,
             ["postgres", "query", "--source", "db", "--", "SELECT 1"]),
            ("POST", "mysql/query", [:], #"{"sql":"SELECT 1"}"#,
             ["mysql", "query", "--", "SELECT 1"]),
            ("GET", "nats/streams", ["source": "bus"], "", ["nats", "streams", "--source", "bus"]),
            ("POST", "nats/sub", [:], #"{"subject":"test","source":"bus","count":2,"timeoutMs":100}"#,
             ["nats", "sub", "--subject", "test", "--source", "bus", "--count", "2", "--timeout-ms", "100"]),
            ("POST", "nats/kv/get", ["source": "bus"], #"{"bucket":"b","key":"k"}"#,
             ["nats", "kv", "get", "--bucket", "b", "--key", "k", "--source", "bus"]),
            ("POST", "nats/kv/watch", [:], #"{"bucket":"b","key":"k","source":"bus","timeoutMs":100}"#,
             ["nats", "kv", "watch", "--bucket", "b", "--key", "k", "--source", "bus", "--timeout-ms", "100"]),
        ]
        for (method, path, query, body, command) in cases {
            let response = router.handle(RestHTTPRequest(
                method: method, path: "/v1/stacks/my%2Fstack/debug/\(path)",
                query: query, headers: [:], body: Data(body.utf8)
            ))
            #expect(response.status == 200, "\(method) \(path)")
            let arguments = try String(contentsOf: dir.appendingPathComponent("argv"), encoding: .utf8)
                .split(separator: "\n").map(String.init)
            let sqlBoundary = command.firstIndex(of: "--") ?? command.endIndex
            #expect(arguments == ["debug"] + Array(command[..<sqlBoundary])
                + ["-C", dir.path, "--format", "json"] + Array(command[sqlBoundary...]))
        }
        for (method, path) in [("GET", "unknown"), ("DELETE", "sources"), ("GET", "doctor/extra")] {
            let response = router.handle(RestHTTPRequest(
                method: method, path: "/v1/stacks/my%2Fstack/debug/\(path)",
                query: [:], headers: [:], body: Data()
            ))
            #expect(response.status == 404)
        }
        let invalidQuery = router.handle(RestHTTPRequest(
            method: "POST", path: "/v1/stacks/my%2Fstack/debug/postgres/query",
            query: [:], headers: [:], body: Data("{}".utf8)
        ))
        #expect(invalidQuery.status == 400)
    }
}
