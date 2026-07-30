import Foundation
import Testing
@testable import VzDaemonKit
@testable import VzSupervisor

@Test func networkCRUDPersistsMetadataAndBlocksAttachedDelete() throws {
    let fixture = try RegistryFixture()
    defer { fixture.cleanup() }

    let network = try fixture.registry.create(
        name: "dmz",
        cidr: "10.80.0.0/24",
        mode: "shared",
        labels: ["tier": "edge"],
        project: "demo",
        stack: "dev"
    )
    #expect(network.labels == ["tier": "edge"])
    let attachment = try fixture.registry.attach(
        vmID: "web",
        networkName: "dmz",
        ip: "10.80.0.10",
        labels: ["role": "frontend"],
        project: nil,
        stack: nil,
        vmIsStopped: true
    )
    #expect(attachment.project == "demo")
    #expect(attachment.stack == "dev")
    #expect(throws: NetworkRegistryError.self) {
        try fixture.registry.delete(name: "dmz")
    }

    try fixture.registry.detach(vmID: "web", networkName: "dmz", vmIsStopped: true)
    try fixture.registry.delete(name: "dmz")
    #expect(try fixture.registry.snapshot().networks.isEmpty)
    #expect(fixture.backendState.releases == 1)
}

@Test func stoppedVMRuleAndUniqueIPAreEnforced() throws {
    let fixture = try RegistryFixture()
    defer { fixture.cleanup() }
    _ = try fixture.registry.create(
        name: "dmz",
        cidr: "10.80.0.0/24",
        mode: "shared",
        labels: [:],
        project: nil,
        stack: nil
    )

    #expect(throws: NetworkRegistryError.self) {
        try fixture.registry.attach(
            vmID: "web",
            networkName: "dmz",
            ip: "10.80.0.10",
            labels: [:],
            project: nil,
            stack: nil,
            vmIsStopped: false
        )
    }
    _ = try fixture.registry.attach(
        vmID: "web",
        networkName: "dmz",
        ip: "10.80.0.10",
        labels: [:],
        project: nil,
        stack: nil,
        vmIsStopped: true
    )
    #expect(throws: NetworkRegistryError.self) {
        try fixture.registry.attach(
            vmID: "api",
            networkName: "dmz",
            ip: "10.80.0.10",
            labels: [:],
            project: nil,
            stack: nil,
            vmIsStopped: true
        )
    }
}

@Test func restartRebuildsDesiredNetworksAndCleanShutdownReleasesRefs() throws {
    let directory = temporaryDirectory("restart")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let database = try StateDatabase(path: directory.appendingPathComponent("state.sqlite").path)
    let backendState = BackendState()
    var first: NetworkRegistry? = try NetworkRegistry(
        database: database,
        backend: RecordingBackend(state: backendState)
    )
    _ = try first?.create(
        name: "dmz",
        cidr: "10.80.0.0/24",
        mode: "shared",
        labels: [:],
        project: "demo",
        stack: "dev"
    )
    #expect(backendState.reservations == 1)
    first?.shutdown()
    first = nil
    #expect(backendState.releases == 1)

    let rebuilt = try NetworkRegistry(
        database: database,
        backend: RecordingBackend(state: backendState)
    )
    #expect(backendState.reservations == 2)
    #expect(try rebuilt.snapshot().networks.first?.runtimeState == "active")
    rebuilt.shutdown()
    #expect(backendState.releases == 2)
}

private struct RegistryFixture {
    let directory: URL
    let backendState: BackendState
    let registry: NetworkRegistry

    init() throws {
        directory = temporaryDirectory("fixture")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let database = try StateDatabase(path: directory.appendingPathComponent("state.sqlite").path)
        backendState = BackendState()
        registry = try NetworkRegistry(
            database: database,
            backend: RecordingBackend(state: backendState)
        )
    }

    func cleanup() {
        registry.shutdown()
        try? FileManager.default.removeItem(at: directory)
    }
}

private final class BackendState: @unchecked Sendable {
    private let lock = NSLock()
    private var reservationCount = 0
    private var releaseCount = 0

    var reservations: Int { lock.withLock { reservationCount } }
    var releases: Int { lock.withLock { releaseCount } }

    func reserve() {
        lock.withLock { reservationCount += 1 }
    }

    func release() {
        lock.withLock { releaseCount += 1 }
    }
}

private struct RecordingBackend: NetworkRuntimeBackend {
    let state: BackendState

    func reserve(_ network: NetworkRecord) throws -> any NetworkRuntimeHandle {
        state.reserve()
        return RecordingHandle(state: state)
    }
}

private final class RecordingHandle: NetworkRuntimeHandle, @unchecked Sendable {
    let state: BackendState

    init(state: BackendState) {
        self.state = state
    }

    deinit {
        state.release()
    }
}

private func temporaryDirectory(_ label: String) -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent(
        "vzctl-network-\(label)-\(UUID().uuidString)",
        isDirectory: true
    )
}
