import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

struct RestStackFilesTests {
    @Test func missingDiagramIs404ButInvalidFileIsNotHidden() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let database = try StateDatabase(path: root.appendingPathComponent("state.sqlite").path)
        try database.upsertStack(StackRegistryRecord(
            id: "test", path: root.path, name: "test", openedAt: "2026-10-20T00:00:00Z"
        ))
        let router = RestRouter(
            jobs: RestJobRunner(stateDirectory: root, vzctlPath: "/usr/bin/false"),
            database: database, stateDirectory: root
        )
        let request = RestHTTPRequest(
            method: "GET", path: "/v1/stacks/test/diagram",
            query: [:], headers: [:], body: Data()
        )
        #expect(router.handle(request).status == 404)
        let diagram = RestStackStore.diagramURL(for: root.path)
        try FileManager.default.createDirectory(at: diagram.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data([0xff, 0xfe, 0xff]).write(to: diagram)
        #expect(router.handle(request).status == 500)
        try "{}".write(to: diagram, atomically: true, encoding: .utf8)
        #expect(router.handle(request).status == 200)
    }
}
