import Foundation
import Testing
@testable import VzSupervisor

@Test func applyJournalBlocksParallelHolderAndSupportsAbort() throws {
    let database = try temporaryDatabase()
    let holder = "test:\(ProcessInfo.processInfo.processIdentifier)"
    let journal = try database.beginApply(
        stackID: "project:stack",
        holder: holder,
        desiredHash: "abc",
        payload: #"{"desired_hash":"abc"}"#,
        resume: false
    )

    #expect(journal.generation == 1)
    #expect(journal.step == "validate")
    #expect(throws: ReconcileDatabaseError.self) {
        _ = try database.beginApply(
            stackID: "project:stack",
            holder: "other:\(ProcessInfo.processInfo.processIdentifier)",
            desiredHash: "abc",
            payload: #"{"desired_hash":"abc"}"#,
            resume: false
        )
    }

    let aborted = try database.abortApply(stackID: "project:stack", holder: holder)
    #expect(aborted.status == "aborted")
    #expect(try database.stackState(stackID: "project:stack").journal == nil)
    #expect(try database.stackState(stackID: "project:stack").lease == nil)
}

@Test func failedJournalResumesSameGenerationAfterDeadHolder() throws {
    let database = try temporaryDatabase()
    let original = try database.beginApply(
        stackID: "project:stack",
        holder: "dead:2147483646",
        desiredHash: "abc",
        payload: #"{"desired_hash":"abc"}"#,
        resume: false
    )
    _ = try database.advanceApply(
        id: original.id,
        stackID: "project:stack",
        holder: "dead:2147483646",
        step: "ensure_nets",
        status: "failed",
        error: "crash"
    )

    let resumed = try database.beginApply(
        stackID: "project:stack",
        holder: "resume:\(ProcessInfo.processInfo.processIdentifier)",
        desiredHash: "abc",
        payload: #"{"desired_hash":"abc"}"#,
        resume: true
    )
    #expect(resumed.id == original.id)
    #expect(resumed.generation == original.generation)
    #expect(resumed.step == "ensure_nets")
    #expect(resumed.status == "running")
}

@Test func finishCommitsActualResourcesAndMakesNextApplyIdempotent() throws {
    let database = try temporaryDatabase()
    let holder = "test:\(ProcessInfo.processInfo.processIdentifier)"
    let journal = try database.beginApply(
        stackID: "project:stack",
        holder: holder,
        desiredHash: "abc",
        payload: #"{"desired_hash":"abc"}"#,
        resume: false
    )
    let resources = [
        StackResourceRecord(
            kind: "network",
            name: "lan",
            labels: ["managed-by": "vzctl", "spec": #"{"cidr":"10.0.0.0/24"}"#],
            state: "active"
        ),
    ]
    let encoded = String(decoding: try JSONEncoder().encode(resources), as: UTF8.self)
    try database.finishApply(
        id: journal.id,
        stackID: "project:stack",
        holder: holder,
        resourcesJSON: encoded
    )

    let state = try database.stackState(stackID: "project:stack")
    #expect(state.journal == nil)
    #expect(state.lease == nil)
    #expect(state.resources.count == 1)
    #expect(state.resources[0].name == "lan")
}

private func temporaryDatabase() throws -> StateDatabase {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return try StateDatabase(path: directory.appendingPathComponent("state.sqlite").path)
}
