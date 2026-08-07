import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

@Test func recoveryClassificationSeparatesHostPortalFromGuestEgressFailure() {
    var result = NetworkRecoveryResult(
        internalOK: true,
        hostEgress: NetworkEgressResult(
            classification: "captive", phase: "http", statusCode: 302,
            latencyMS: 5, errorCode: nil
        ),
        networkEgress: [:],
        conflicts: [],
        error: nil
    )
    #expect(result.state(pathSatisfied: true) == .captive)

    result.hostEgress.classification = "online"
    result.networkEgress["dmz"] = NetworkEgressResult(
        classification: "offline", phase: "tcp", statusCode: nil,
        latencyMS: 10, errorCode: "connect"
    )
    #expect(result.state(pathSatisfied: true) == .degraded)
    #expect(result.state(pathSatisfied: false) == .offline)
}

@Test func routeConflictsAreReportedWithoutRenumberingDesiredNetworks() {
    let networks = [NetworkRecord(name: "dmz", cidr: "10.80.0.0/24")]
    let conflicts = HostRouteScanner.conflicts(
        networks: networks,
        routes: [
            HostRoute(cidr: "10.80.0.0/24", interface: "en0"),
            HostRoute(cidr: "10.90.0.0/24", interface: "utun4"),
            HostRoute(cidr: "10.80.0.0/24", interface: "bridge100"),
        ]
    )
    #expect(conflicts.count == 1)
    #expect(conflicts[0].network == "dmz")
    #expect(networks[0].cidr == "10.80.0.0/24")
}

@Test func vpnSplitRoutesAreParsedFromTheMacOSRouteTable() {
    let output = """
    Routing tables
    Internet:
    Destination        Gateway        Flags        Netif Expire
    default            192.0.2.1      UGScg        en0
    10.80/16           10.0.0.1       UGSc         utun4
    10.90.1/24         link#24        UC           bridge100
    192.168.178        link#14        UCS          en0
    203.0.113.1/32     10.0.0.1       UGHS         utun4
    """
    #expect(HostRouteScanner.parseRouteTable(output) == [
        HostRoute(cidr: "10.80.0.0/16", interface: "utun4"),
        HostRoute(cidr: "192.168.178.0/24", interface: "en0"),
    ])
}

@Test func pathFlappingCancelsStaleDebouncedRecovery() async throws {
    let counter = LockedCounter()
    let controller = NetworkResilienceController(
        debounce: 0.02,
        recoveryBudget: 0.1,
        probe: {
            counter.increment()
            return NetworkRecoveryResult(
                internalOK: true,
                hostEgress: NetworkEgressResult(
                    classification: "online", phase: "http", statusCode: 200,
                    latencyMS: 1, errorCode: nil
                ),
                networkEgress: [:], conflicts: [], error: nil
            )
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["ethernet"], event: "path")
    controller.receiveForTest(satisfied: false, interfaces: [], event: "path")
    try await Task.sleep(for: .milliseconds(50))
    #expect(counter.value == 0)

    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(80))
    #expect(counter.value == 1)
    guard case let .object(values) = controller.health() else {
        Issue.record("health must be object")
        return
    }
    #expect(values["state"] == .string("healthy"))
    controller.stop()
}

@Test func wakeWhileOfflineWaitsForAUsablePath() async throws {
    let counter = LockedCounter()
    let controller = NetworkResilienceController(
        debounce: 0.01,
        recoveryBudget: 0.05,
        probe: {
            counter.increment()
            return .healthyForTest
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: false, interfaces: [], event: "path")
    controller.sleepForTest()
    controller.wakeForTest()
    try await Task.sleep(for: .milliseconds(30))
    #expect(counter.value == 0)

    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(40))
    #expect(counter.value == 1)
    controller.stop()
}

@Test func recoveryDeadlineDoesNotStartAPastDeadlineProbeOrDefaultFallback() async throws {
    let metrics = LockedProbeMetrics()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.04,
        probe: {
            metrics.enter()
            defer { metrics.leave() }
            return .degradedForTest
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(100))
    #expect(metrics.calls == 1)
    #expect(metrics.maximumActive == 1)
    guard case let .object(values) = controller.health() else {
        Issue.record("health must be object")
        return
    }
    #expect(values["state"] == .string("degraded"))
    controller.stop()
}

@Test func recoveryIsSerialAndRunsFallbackOnlyWhenExplicitlyProvided() async throws {
    let metrics = LockedProbeMetrics()
    let fallback = LockedCounter()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.02,
        probe: {
            metrics.enter()
            defer { metrics.leave() }
            if metrics.calls == 1 { return .degradedForTest }
            return .healthyForTest
        },
        fallback: { _ in
            fallback.increment()
            return true
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["ethernet"], event: "path")
    try await Task.sleep(for: .milliseconds(200))
    #expect(fallback.value == 1)
    #expect(metrics.calls == 2)
    #expect(metrics.maximumActive == 1)
    controller.stop()
}

@Test func captivePortalReleaseRecoversWithoutANewPathEvent() async throws {
    let metrics = LockedProbeMetrics()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.02,
        passiveRetry: 0.02,
        probe: {
            metrics.enter()
            defer { metrics.leave() }
            if metrics.calls == 1 {
                return NetworkRecoveryResult(
                    internalOK: true,
                    hostEgress: NetworkEgressResult(
                        classification: "captive", phase: "http", statusCode: 302,
                        latencyMS: 1, errorCode: nil
                    ),
                    networkEgress: [:], conflicts: [], error: nil
                )
            }
            return .healthyForTest
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(70))
    #expect(metrics.calls == 2)
    guard case let .object(values) = controller.health() else {
        Issue.record("health must be object")
        return
    }
    #expect(values["state"] == .string("healthy"))
    controller.stop()
}

@Test func destructiveFallbackIsAttemptedAtMostOncePerEpoch() async throws {
    let fallback = LockedCounter()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.01,
        passiveRetry: 0.01,
        probe: { .degradedForTest },
        fallback: { _ in
            fallback.increment()
            return false
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(70))
    #expect(fallback.value == 1)
    controller.stop()
}

@Test func restartFallbackRequiresCompleteStackOptIn() {
    let network = NetworkRecord(
        name: "dmz", cidr: "10.80.0.0/24", project: "demo", stack: "dev"
    )
    let owned = NetworkAttachmentRecord(
        vmID: "demo/web", networkName: "dmz", ip: "10.80.0.10",
        project: "demo", stack: "dev"
    )
    let policy = NetworkResiliencePolicyRecord(
        project: "demo", stack: "dev", probeEnabled: true,
        probeURL: "https://captive.apple.com/", restartVMs: true,
        updatedAt: "2026-08-03T00:00:00Z"
    )
    #expect(SupervisorServer.networkFallbackPolicyAllows(
        networks: ["dmz"],
        snapshot: NetworkSnapshot(networks: [network], attachments: [owned]),
        policies: [policy]
    ))

    var foreign = owned
    foreign.vmID = "manual"
    foreign.project = nil
    foreign.stack = nil
    #expect(!SupervisorServer.networkFallbackPolicyAllows(
        networks: ["dmz"],
        snapshot: NetworkSnapshot(networks: [network], attachments: [owned, foreign]),
        policies: [policy]
    ))
    let disabled = NetworkResiliencePolicyRecord(
        project: "demo", stack: "dev", probeEnabled: true,
        probeURL: "https://captive.apple.com/", restartVMs: false,
        updatedAt: "2026-08-03T00:00:00Z"
    )
    #expect(!SupervisorServer.networkFallbackPolicyAllows(
        networks: ["dmz"],
        snapshot: NetworkSnapshot(networks: [network], attachments: [owned]),
        policies: [disabled]
    ))
}

@Test func oldGuestAgentsWithoutNetworkProbeRemainCompatible() {
    let oldVersion: JSONValue = .object([
        "agent_version": .string("0.1.0"),
        "capabilities": .array([.string("ping"), .string("report_ip")]),
    ])
    #expect(!SupervisorServer.agentSupportsNetworkProbe(oldVersion))
    let currentVersion: JSONValue = .object([
        "agent_version": .string("0.2.0"),
        "capabilities": .array([.string("ping"), .string("network_probe")]),
    ])
    #expect(SupervisorServer.agentSupportsNetworkProbe(currentVersion))

    let result = NetworkRecoveryResult(
        internalOK: true,
        hostEgress: .unknown,
        networkEgress: ["dmz": .unknown],
        conflicts: [], error: nil
    )
    #expect(result.state(pathSatisfied: true) == .healthy)
}

@Test func localeTimezoneAndWallClockSignalsDoNotCreateANetworkEpoch() async throws {
    let counter = LockedCounter()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.02,
        probe: {
            counter.increment()
            return .healthyForTest
        },
        event: { _, _ in }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["wifi"], event: "path")
    try await Task.sleep(for: .milliseconds(20))
    controller.receiveForTest(
        satisfied: true, interfaces: ["wifi"], event: "locale_changed"
    )
    controller.receiveForTest(
        satisfied: true, interfaces: ["wifi"], event: "timezone_changed"
    )
    try await Task.sleep(for: .milliseconds(20))
    #expect(counter.value == 1)
    guard case let .object(values) = controller.health() else {
        Issue.record("health must be object")
        return
    }
    #expect(values["epoch"] == .number(1))
    controller.stop()
}

@Test func healthAndRecoveryEventContractRemainStable() async throws {
    let events = LockedEvents()
    let controller = NetworkResilienceController(
        debounce: 0.001,
        recoveryBudget: 0.02,
        probe: { .healthyForTest },
        event: { type, data in events.append(type: type, data: data) }
    )
    controller.receiveForTest(satisfied: true, interfaces: ["ethernet"], event: "path")
    try await Task.sleep(for: .milliseconds(30))
    #expect(events.types == [
        "host.network_changed", "network.recovering", "network.recovered",
    ])
    #expect(events.allHaveEpoch)
    guard case let .object(values) = controller.health() else {
        Issue.record("health must be object")
        return
    }
    #expect(values["state"] == .string("healthy"))
    #expect(values["internal_ok"] == .bool(true))
    #expect(values["host_egress"] != nil)
    #expect(values["network_egress"] != nil)
    #expect(values["cidr_conflicts"] != nil)
    controller.stop()
}

private extension NetworkRecoveryResult {
    static var healthyForTest: Self {
        NetworkRecoveryResult(
            internalOK: true,
            hostEgress: NetworkEgressResult(
                classification: "online", phase: "http", statusCode: 200,
                latencyMS: 1, errorCode: nil
            ),
            networkEgress: [:], conflicts: [], error: nil
        )
    }

    static var degradedForTest: Self {
        NetworkRecoveryResult(
            internalOK: false,
            hostEgress: NetworkEgressResult(
                classification: "online", phase: "http", statusCode: 200,
                latencyMS: 1, errorCode: nil
            ),
            networkEgress: [:], conflicts: [], error: "internal"
        )
    }
}

private final class LockedCounter: @unchecked Sendable {
    private let lock = NSLock()
    private var count = 0
    var value: Int { lock.withLock { count } }
    func increment() { lock.withLock { count += 1 } }
}

private final class LockedProbeMetrics: @unchecked Sendable {
    private let lock = NSLock()
    private var callCount = 0
    private var active = 0
    private var maxActive = 0
    var calls: Int { lock.withLock { callCount } }
    var maximumActive: Int { lock.withLock { maxActive } }

    func enter() {
        lock.withLock {
            callCount += 1
            active += 1
            maxActive = max(maxActive, active)
        }
    }

    func leave() { lock.withLock { active -= 1 } }
}

private final class LockedEvents: @unchecked Sendable {
    private let lock = NSLock()
    private var values: [(String, [String: JSONValue])] = []
    var types: [String] { lock.withLock { values.map(\.0) } }
    var allHaveEpoch: Bool {
        lock.withLock { values.allSatisfy { $0.1["epoch"] != nil } }
    }
    func append(type: String, data: [String: JSONValue]) {
        lock.withLock { values.append((type, data)) }
    }
}
