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

public struct RouterPlan: Equatable, Sendable {
    public let vmID: String
    public let networks: [RouterNetwork]

    public init(vmID: String, networks: [RouterNetwork]) {
        self.vmID = vmID
        self.networks = networks
    }

    public init(
        vmID: String,
        networkRecords: [NetworkRecord],
        attachments: [NetworkAttachmentRecord]
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
    }

    public var json: JSONValue {
        .object([
            "vm_id": .string(vmID),
            "networks": .array(networks.map(\.json)),
        ])
    }
}
