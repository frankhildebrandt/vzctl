import Foundation

public enum RouteApplyError: Error, CustomStringConvertible, Equatable {
    case invalid(String)
    case unavailable(String)
    case guest(String)

    public var description: String {
        switch self {
        case let .invalid(message): message
        case let .unavailable(message): message
        case let .guest(message): message
        }
    }

    public var rpcCode: Int {
        switch self {
        case .invalid: -32602
        case .unavailable: -32018
        case .guest: -32019
        }
    }
}

public enum RouterOperation: String, Sendable {
    case apply, plan, status
}

public let internetPolicyTarget = "internet"

/// Non-RFC1918 / non-link-local destinations for `to: internet` nft rules.
private let internetDestinationExpr =
    "!= { 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 127.0.0.0/8, 169.254.0.0/16 }"

public struct RouterNetwork: Equatable, Sendable {
    public let name: String
    public let cidr: String
    public let address: String
    public let natEgress: Bool

    public init(name: String, cidr: String, address: String, natEgress: Bool = true) {
        self.name = name
        self.cidr = cidr
        self.address = address
        self.natEgress = natEgress
    }

    public var json: JSONValue {
        .object([
            "name": .string(name),
            "cidr": .string(cidr),
            "address": .string(address),
            "nat_egress": .bool(natEgress),
            "host_gateway_dns": .string(IPv4CIDR.gateway(for: cidr)),
            "router_gateway": .string(IPv4CIDR.router(for: cidr)),
        ])
    }
}

public struct PolicyAllow: Equatable, Sendable {
    public let to: String
    public let proto: String
    public let ports: [Int]

    public init(to: String, proto: String, ports: [Int] = []) {
        self.to = to
        self.proto = proto
        self.ports = ports
    }

    public var json: JSONValue {
        .object([
            "to": .string(to),
            "proto": .string(proto),
            "ports": .array(ports.map { .number(Double($0)) }),
        ])
    }
}

public struct ForwardPolicy: Equatable, Sendable {
    public let name: String
    public let network: String
    public let forward: String
    public let allow: [PolicyAllow]
    /// Optional router VM id or config basename that must apply this policy.
    public let via: String?

    public init(
        name: String,
        network: String,
        forward: String,
        allow: [PolicyAllow],
        via: String? = nil
    ) {
        self.name = name
        self.network = network
        self.forward = forward
        self.allow = allow
        self.via = via
    }

    public var json: JSONValue {
        var object: [String: JSONValue] = [
            "name": .string(name),
            "network": .string(network),
            "forward": .string(forward),
            "allow": .array(allow.map(\.json)),
        ]
        if let via {
            object["via"] = .string(via)
        }
        return .object(object)
    }

    /// Match `policies.*.via` (config key or full runtime id) to a running router.
    public static func matchesVia(vmID: String, via: String) -> Bool {
        if vmID == via { return true }
        if let slash = vmID.lastIndex(of: "/") {
            return String(vmID[vmID.index(after: slash)...]) == via
        }
        return false
    }
}

public enum DockerBackendRoutes {
    /// Static routes so peer routers can reach docker-backend CIDRs via the
    /// docker owner's parent (vmnet) IP.
    public static func staticRoutes(
        forRouter vmID: String,
        networks: [NetworkRecord],
        attachments: [NetworkAttachmentRecord]
    ) -> [StaticRoute] {
        let byName = Dictionary(uniqueKeysWithValues: networks.map { ($0.name, $0) })
        let byVM = Dictionary(grouping: attachments, by: \.vmID)
        var routes: [StaticRoute] = []
        for network in networks where network.isDockerBackend {
            guard let owner = attachments.first(where: { $0.networkName == network.name }) else {
                continue
            }
            if owner.vmID == vmID {
                continue
            }
            guard let ownerAttachments = byVM[owner.vmID] else { continue }
            guard let routerAttachments = byVM[vmID] else { continue }
            let ownerParents = ownerAttachments.filter {
                byName[$0.networkName]?.isDockerBackend != true
            }
            let routerParents = Set(
                routerAttachments
                    .filter { byName[$0.networkName]?.isDockerBackend != true }
                    .map(\.networkName)
            )
            for parent in ownerParents where routerParents.contains(parent.networkName) {
                routes.append(StaticRoute(destination: network.cidr, via: parent.ip))
            }
        }
        var seen = Set<String>()
        return routes.filter { route in
            let key = "\(route.destination)->\(route.via)"
            return seen.insert(key).inserted
        }
    }
}

public struct StaticRoute: Equatable, Sendable {
    public let destination: String
    public let via: String

    public init(destination: String, via: String) {
        self.destination = destination
        self.via = via
    }

    public var json: JSONValue {
        .object([
            "destination": .string(destination),
            "via": .string(via),
        ])
    }
}

public struct ActiveForwardRule: Equatable, Sendable {
    public let policy: String
    public let from: String
    public let to: String
    public let proto: String
    public let ports: [Int]
    public let sourceCIDR: String
    public let destinationCIDR: String

    public var isInternet: Bool { to == internetPolicyTarget }

    public var json: JSONValue {
        .object([
            "policy": .string(policy),
            "from": .string(from),
            "to": .string(to),
            "proto": .string(proto),
            "ports": .array(ports.map { .number(Double($0)) }),
            "source_cidr": .string(sourceCIDR),
            "destination_cidr": .string(destinationCIDR),
            "action": .string("accept"),
        ])
    }
}

public struct RouterPlan: Equatable, Sendable {
    public let vmID: String
    public let networks: [RouterNetwork]
    public let policies: [ForwardPolicy]
    public let staticRoutes: [StaticRoute]

    public init(
        vmID: String,
        networks: [RouterNetwork],
        policies: [ForwardPolicy] = [],
        staticRoutes: [StaticRoute] = []
    ) throws {
        self.vmID = vmID
        self.networks = networks.sorted { $0.name < $1.name }
        self.policies = policies.sorted { $0.name < $1.name }
        self.staticRoutes = staticRoutes.sorted {
            if $0.destination != $1.destination {
                return $0.destination < $1.destination
            }
            return $0.via < $1.via
        }
        try validate()
    }

    public init(
        vmID: String,
        networkRecords: [NetworkRecord],
        attachments: [NetworkAttachmentRecord],
        policies: [ForwardPolicy] = [],
        staticRoutes: [StaticRoute] = []
    ) throws {
        let records = Dictionary(uniqueKeysWithValues: networkRecords.map { ($0.name, $0) })
        let selected = attachments
            .filter { $0.vmID == vmID }
            .sorted { $0.networkName < $1.networkName }
        guard selected.count >= 2 else {
            throw RouteApplyError.invalid(
                "router VM \(vmID) requires at least two network attachments"
            )
        }
        let isDockerRouter = selected.contains {
            records[$0.networkName]?.isDockerBackend == true
        }
        networks = try selected.map { attachment in
            guard let network = records[attachment.networkName] else {
                throw RouteApplyError.invalid(
                    "router attachment references unknown network \(attachment.networkName)"
                )
            }
            let expected = IPv4CIDR.router(for: network.cidr)
            if network.isDockerBackend || !isDockerRouter {
                guard attachment.ip == expected else {
                    throw RouteApplyError.invalid(
                        "router VM \(vmID) must use \(expected) on \(network.name), got \(attachment.ip)"
                    )
                }
            }
            return RouterNetwork(
                name: network.name,
                cidr: network.cidr,
                address: attachment.ip,
                natEgress: network.natEgress
            )
        }
        self.vmID = vmID
        self.policies = policies.sorted { $0.name < $1.name }
        self.staticRoutes = staticRoutes.sorted {
            if $0.destination != $1.destination {
                return $0.destination < $1.destination
            }
            return $0.via < $1.via
        }
        try validate()
    }

    public var rules: [ActiveForwardRule] {
        let cidrs = Dictionary(uniqueKeysWithValues: networks.map { ($0.name, $0.cidr) })
        return policies.flatMap { policy in
            policy.allow.map { allow in
                let destination: String
                if allow.to == internetPolicyTarget {
                    destination = internetPolicyTarget
                } else {
                    destination = cidrs[allow.to]!
                }
                return ActiveForwardRule(
                    policy: policy.name,
                    from: policy.network,
                    to: allow.to,
                    proto: allow.proto,
                    ports: allow.ports,
                    sourceCIDR: cidrs[policy.network]!,
                    destinationCIDR: destination
                )
            }
        }
    }

    public var json: JSONValue {
        .object([
            "apiVersion": .string("vzctl.dev/router/v1"),
            "vm_id": .string(vmID),
            "networks": .array(networks.map(\.json)),
            "forward_policy": .string("drop"),
            "policies": .array(policies.map(\.json)),
            "rules": .array(rules.map(\.json)),
            "static_routes": .array(staticRoutes.map(\.json)),
        ])
    }

    public var nftables: String {
        var lines = [
            "table inet vzctl {",
            "  chain forward {",
            "    type filter hook forward priority 0; policy drop;",
            "    ct state established,related accept comment \"vzctl:return\"",
        ]
        for rule in rules {
            var expression = "    ip saddr \(rule.sourceCIDR) "
            if rule.isInternet {
                expression += "ip daddr \(internetDestinationExpr)"
            } else {
                expression += "ip daddr \(rule.destinationCIDR)"
            }
            if rule.proto == "icmp" {
                expression += " ip protocol icmp"
            } else {
                expression += " \(rule.proto)"
                if rule.ports.count == 1 {
                    expression += " dport \(rule.ports[0])"
                } else {
                    expression += " dport { \(rule.ports.map(String.init).joined(separator: ", ")) }"
                }
            }
            expression += " accept comment \"vzctl:\(rule.policy)\""
            lines.append(expression)
        }
        lines.append("  }")

        let internetSources = Array(
            Set(rules.filter(\.isInternet).map(\.sourceCIDR))
        ).sorted()
        let hasNatEgress = networks.contains(where: \.natEgress)
        if !internetSources.isEmpty, hasNatEgress {
            lines.append("  chain postrouting {")
            lines.append("    type nat hook postrouting priority srcnat; policy accept;")
            for cidr in internetSources {
                lines.append(
                    "    ip saddr \(cidr) masquerade comment \"vzctl:internet\""
                )
            }
            lines.append("  }")
        }

        lines.append(contentsOf: ["}", ""])
        return lines.joined(separator: "\n")
    }

    private func validate() throws {
        guard networks.count >= 2 else {
            throw RouteApplyError.invalid("router plan requires at least two networks")
        }
        guard Set(networks.map(\.name)).count == networks.count else {
            throw RouteApplyError.invalid("router plan contains duplicate networks")
        }
        for network in networks {
            _ = try IPv4CIDR(network.cidr)
        }
        let routerOwned = networks.filter {
            $0.address == IPv4CIDR.router(for: $0.cidr)
        }
        let parentOwned = networks.filter {
            $0.address != IPv4CIDR.router(for: $0.cidr)
        }
        if parentOwned.isEmpty {
            // Classic dual-homed router: every attachment is .2.
            guard routerOwned.count == networks.count else {
                throw RouteApplyError.invalid("router plan requires .2 on every network")
            }
        } else {
            // Docker+router: docker bip is .2; parent NIC keeps the guest IP.
            guard !routerOwned.isEmpty else {
                throw RouteApplyError.invalid(
                    "docker router plan requires at least one .2 attachment"
                )
            }
        }
        guard Set(policies.map(\.name)).count == policies.count else {
            throw RouteApplyError.invalid("policy names must be unique")
        }
        let networkNames = Set(networks.map(\.name))
        for policy in policies {
            guard !policy.name.isEmpty,
                  policy.name.utf8.allSatisfy({
                      ($0 >= 48 && $0 <= 57) ||
                          ($0 >= 65 && $0 <= 90) ||
                          ($0 >= 97 && $0 <= 122) ||
                          [45, 46, 95].contains($0)
                  })
            else {
                throw RouteApplyError.invalid(
                    "policy name may only contain letters, digits, dot, dash, and underscore"
                )
            }
            guard networkNames.contains(policy.network) else {
                throw RouteApplyError.invalid(
                    "policy \(policy.name) references unattached source network \(policy.network)"
                )
            }
            guard policy.forward == "deny-all" else {
                throw RouteApplyError.invalid(
                    "policy \(policy.name) forward must be deny-all"
                )
            }
            for allow in policy.allow {
                if allow.to == internetPolicyTarget {
                    // With natEgress: MASQUERADE. Without: forward-only (e.g. docker
                    // router sending container traffic toward a peer router).
                } else {
                    guard networkNames.contains(allow.to) else {
                        throw RouteApplyError.invalid(
                            "policy \(policy.name) references unattached destination network \(allow.to)"
                        )
                    }
                }
                switch allow.proto {
                case "icmp":
                    guard allow.ports.isEmpty else {
                        throw RouteApplyError.invalid(
                            "policy \(policy.name) ICMP allow must not declare ports"
                        )
                    }
                case "tcp", "udp":
                    guard !allow.ports.isEmpty,
                          allow.ports.allSatisfy({ (1 ... 65535).contains($0) })
                    else {
                        throw RouteApplyError.invalid(
                            "policy \(policy.name) TCP/UDP allow requires ports 1...65535"
                        )
                    }
                default:
                    throw RouteApplyError.invalid(
                        "policy \(policy.name) proto must be tcp, udp, or icmp"
                    )
                }
            }
        }
    }
}
