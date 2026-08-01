import Foundation
import Testing
import VzDaemonKit

@testable import VzEdge

@Test func hostNetworkReconcileProtectsBeforeAliasAndRemovesBeforeFinalRules() throws {
    let client = RecordingHostNetworkClient()
    let reconciler = HostNetworkReconciler(client: client)
    let dmz = binding("10.80.0.0/24", ports: [80, 443])
    let lan = binding(
        "10.90.0.0/24",
        sources: ["10.90.0.0/24", "10.95.0.0/24"],
        ports: [80, 443]
    )

    try reconciler.prepare(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    try reconciler.finish(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    try reconciler.prepare(targetCIDRs: [lan.cidr], targetFirewall: [lan.cidr: lan])
    try reconciler.finish(targetCIDRs: [lan.cidr], targetFirewall: [lan.cidr: lan])

    #expect(client.events == [
        "firewall:10.80.0.0/24",
        "ensure:10.80.0.0/24",
        "firewall:10.80.0.0/24",
        "firewall:10.80.0.0/24,10.90.0.0/24",
        "ensure:10.90.0.0/24",
        "remove:10.80.0.0/24",
        "firewall:10.90.0.0/24",
    ])
    #expect(reconciler.aliasCount == 1)
}

@Test func hostNetworkReconcileIsAliasIdempotentAndRecoversAfterFailure() throws {
    let client = RecordingHostNetworkClient()
    let reconciler = HostNetworkReconciler(client: client)
    let dmz = binding("10.80.0.0/24", ports: [443])
    let lan = binding("10.90.0.0/24", ports: [443])

    try reconciler.prepare(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    try reconciler.finish(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    try reconciler.prepare(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    #expect(client.events.filter { $0 == "ensure:10.80.0.0/24" }.count == 1)

    client.failEnsure = lan.cidr
    #expect(throws: RecordingError.injected) {
        try reconciler.prepare(
            targetCIDRs: [dmz.cidr, lan.cidr],
            targetFirewall: [dmz.cidr: dmz, lan.cidr: lan]
        )
    }
    client.failEnsure = nil
    try reconciler.prepare(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    try reconciler.finish(targetCIDRs: [dmz.cidr], targetFirewall: [dmz.cidr: dmz])
    #expect(reconciler.aliasCount == 1)
    #expect(client.events.last == "firewall:10.80.0.0/24")
}

private enum RecordingError: Error {
    case injected
}

private final class RecordingHostNetworkClient: HostNetworkClient, @unchecked Sendable {
    var events: [String] = []
    var failEnsure: String?

    func reconcileFirewall(_ bindings: [DnsBind.FirewallBinding]) throws {
        events.append("firewall:" + bindings.map(\.cidr).sorted().joined(separator: ","))
    }

    func ensureAlias(cidr: String) throws {
        if failEnsure == cidr { throw RecordingError.injected }
        events.append("ensure:\(cidr)")
    }

    func removeAlias(cidr: String) throws {
        events.append("remove:\(cidr)")
    }
}

private func binding(
    _ cidr: String,
    sources: [String]? = nil,
    ports: [UInt16]
) -> DnsBind.FirewallBinding {
    DnsBind.FirewallBinding(
        cidr: cidr,
        allowedSources: sources ?? [cidr],
        tcpPorts: ports
    )
}
