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

public struct RouterNetwork: Equatable, Sendable {
    public let name: String
    public let cidr: String
    public let address: String

    public init(name: String, cidr: String, address: String) {
        self.name = name
        self.cidr = cidr
        self.address = address
    }

    public var json: JSONValue {
        .object([
            "name": .string(name),
            "cidr": .string(cidr),
            "address": .string(address),
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

    public init(name: String, network: String, forward: String, allow: [PolicyAllow]) {
        self.name = name
        self.network = network
        self.forward = forward
        self.allow = allow
    }

    public var json: JSONValue {
        .object([
            "name": .string(name),
            "network": .string(network),
            "forward": .string(forward),
            "allow": .array(allow.map(\.json)),
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

    public init(
        vmID: String,
        networks: [RouterNetwork],
        policies: [ForwardPolicy] = []
    ) throws {
        self.vmID = vmID
        self.networks = networks.sorted { $0.name < $1.name }
        self.policies = policies.sorted { $0.name < $1.name }
        try validate()
    }

    public init(
        vmID: String,
        networkRecords: [NetworkRecord],
        attachments: [NetworkAttachmentRecord],
        policies: [ForwardPolicy] = []
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
        networks = try selected.map { attachment in
            guard let network = records[attachment.networkName] else {
                throw RouteApplyError.invalid(
                    "router attachment references unknown network \(attachment.networkName)"
                )
            }
            let expected = IPv4CIDR.router(for: network.cidr)
            guard attachment.ip == expected else {
                throw RouteApplyError.invalid(
                    "router VM \(vmID) must use \(expected) on \(network.name), got \(attachment.ip)"
                )
            }
            return RouterNetwork(name: network.name, cidr: network.cidr, address: attachment.ip)
        }
        self.vmID = vmID
        self.policies = policies.sorted { $0.name < $1.name }
        try validate()
    }

    public var rules: [ActiveForwardRule] {
        let cidrs = Dictionary(uniqueKeysWithValues: networks.map { ($0.name, $0.cidr) })
        return policies.flatMap { policy in
            policy.allow.map { allow in
                ActiveForwardRule(
                    policy: policy.name,
                    from: policy.network,
                    to: allow.to,
                    proto: allow.proto,
                    ports: allow.ports,
                    sourceCIDR: cidrs[policy.network]!,
                    destinationCIDR: cidrs[allow.to]!
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
            var expression =
                "    ip saddr \(rule.sourceCIDR) ip daddr \(rule.destinationCIDR)"
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
        lines.append(contentsOf: ["  }", "}", ""])
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
            guard network.address == IPv4CIDR.router(for: network.cidr) else {
                throw RouteApplyError.invalid(
                    "router address for \(network.name) must be \(IPv4CIDR.router(for: network.cidr))"
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
                guard networkNames.contains(allow.to) else {
                    throw RouteApplyError.invalid(
                        "policy \(policy.name) references unattached destination network \(allow.to)"
                    )
                }
                guard ["tcp", "udp", "icmp"].contains(allow.proto) else {
                    throw RouteApplyError.invalid(
                        "policy \(policy.name) proto must be tcp, udp, or icmp"
                    )
                }
                if allow.proto == "icmp" {
                    guard allow.ports.isEmpty else {
                        throw RouteApplyError.invalid(
                            "policy \(policy.name) ICMP allow must not declare ports"
                        )
                    }
                } else {
                    guard !allow.ports.isEmpty,
                          allow.ports.allSatisfy({ (1 ... 65_535).contains($0) }),
                          Set(allow.ports).count == allow.ports.count
                    else {
                        throw RouteApplyError.invalid(
                            "policy \(policy.name) TCP/UDP ports must be unique values 1...65535"
                        )
                    }
                }
            }
        }
    }
}
