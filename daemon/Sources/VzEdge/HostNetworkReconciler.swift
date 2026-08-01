import Foundation
import VzDaemonKit

protocol HostNetworkClient: Sendable {
    func reconcileFirewall(_ bindings: [DnsBind.FirewallBinding]) throws
    func ensureAlias(cidr: String) throws
    func removeAlias(cidr: String) throws
}

struct LiveHostNetworkClient: HostNetworkClient {
    func reconcileFirewall(_ bindings: [DnsBind.FirewallBinding]) throws {
        try DnsBindClient.reconcileFirewall(bindings)
    }

    func ensureAlias(cidr: String) throws {
        try DnsBindClient.ensureAlias(cidr: cidr)
    }

    func removeAlias(cidr: String) throws {
        try DnsBindClient.removeAlias(cidr: cidr)
    }
}

final class HostNetworkReconciler: @unchecked Sendable {
    private let client: any HostNetworkClient
    private let lock = NSLock()
    private var aliases = Set<String>()
    private var firewall: [String: DnsBind.FirewallBinding] = [:]

    init(client: any HostNetworkClient = LiveHostNetworkClient()) {
        self.client = client
    }

    var aliasCount: Int { lock.withLock { aliases.count } }

    /// Protect new addresses before adding them. Existing protection remains active
    /// until finish() has removed stale aliases.
    func prepare(
        targetCIDRs: Set<String>,
        targetFirewall: [String: DnsBind.FirewallBinding]
    ) throws {
        let current = lock.withLock { (aliases, firewall) }
        var protected = current.1
        for (cidr, binding) in targetFirewall { protected[cidr] = binding }
        if !protected.isEmpty {
            try client.reconcileFirewall(protected.values.sorted { $0.cidr < $1.cidr })
            lock.withLock { firewall = protected }
        }
        for cidr in targetCIDRs.subtracting(current.0).sorted() {
            try client.ensureAlias(cidr: cidr)
            _ = lock.withLock { aliases.insert(cidr) }
        }
    }

    /// Remove stale aliases while their block rules are still installed, then
    /// atomically publish only the final firewall ruleset.
    func finish(
        targetCIDRs: Set<String>,
        targetFirewall: [String: DnsBind.FirewallBinding]
    ) throws {
        let current = lock.withLock { aliases }
        for cidr in current.subtracting(targetCIDRs).sorted().reversed() {
            try client.removeAlias(cidr: cidr)
            _ = lock.withLock { aliases.remove(cidr) }
        }
        if !targetFirewall.isEmpty || !lock.withLock({ firewall.isEmpty }) {
            try client.reconcileFirewall(targetFirewall.values.sorted { $0.cidr < $1.cidr })
        }
        lock.withLock { firewall = targetFirewall }
    }
}
