import Darwin
import Foundation

public struct IPv4CIDR: Equatable, Sendable {
    public let canonical: String
    public let network: UInt32
    public let mask: UInt32
    public let prefixLength: Int

    public init(_ value: String) throws {
        let parts = value.split(separator: "/", omittingEmptySubsequences: false)
        guard parts.count == 2,
              let prefix = Int(parts[1]),
              (8 ... 30).contains(prefix)
        else {
            throw NetworkValidationError.invalidCIDR(value)
        }

        var address = in_addr()
        guard inet_pton(AF_INET, String(parts[0]), &address) == 1 else {
            throw NetworkValidationError.invalidCIDR(value)
        }
        let hostAddress = UInt32(bigEndian: address.s_addr)
        let hostMask = UInt32.max << UInt32(32 - prefix)
        let network = hostAddress & hostMask
        guard hostAddress == network else {
            throw NetworkValidationError.nonCanonicalCIDR(value)
        }

        self.network = network
        mask = hostMask
        prefixLength = prefix
        canonical = "\(Self.string(network))/\(prefix)"
    }

    public func containsGuest(_ value: String) -> Bool {
        guard let address = Self.parse(value) else { return false }
        let broadcast = network | ~mask
        return address >= network + 10 && address < broadcast
    }

    public func containsAttachment(_ value: String) -> Bool {
        guard let address = Self.parse(value) else { return false }
        return address == network + 2 || containsGuest(value)
    }

    public func guestAddress(offset: UInt32) -> String? {
        let broadcast = network | ~mask
        let address = network + offset
        guard offset >= 10, address < broadcast else { return nil }
        return Self.string(address)
    }

    public var prefix: Int { prefixLength }

    public var subnetAddress: in_addr {
        in_addr(s_addr: network.bigEndian)
    }

    public var maskAddress: in_addr {
        in_addr(s_addr: mask.bigEndian)
    }

    private static func parse(_ value: String) -> UInt32? {
        var address = in_addr()
        guard inet_pton(AF_INET, value, &address) == 1 else { return nil }
        return UInt32(bigEndian: address.s_addr)
    }

    private static func string(_ value: UInt32) -> String {
        var address = in_addr(s_addr: value.bigEndian)
        var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &address, &buffer, socklen_t(INET_ADDRSTRLEN)) != nil else {
            return ""
        }
        return String(
            decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) },
            as: UTF8.self
        )
    }
}

public enum NetworkValidationError: Error, CustomStringConvertible, Equatable {
    case invalidCIDR(String)
    case nonCanonicalCIDR(String)
    case invalidIP(String, cidr: String)

    public var description: String {
        switch self {
        case let .invalidCIDR(value):
            return "invalid IPv4 CIDR \(value); prefix must be /8 through /30"
        case let .nonCanonicalCIDR(value):
            return "CIDR must use its network address: \(value)"
        case let .invalidIP(value, cidr):
            return "IP \(value) must be router offset .2 or a guest offset .10 or later in \(cidr)"
        }
    }
}

public struct NetworkRecord: Equatable, Sendable {
    public var name: String
    public var cidr: String
    public var mode: String
    public var labels: [String: String]
    public var project: String?
    public var stack: String?
    public var runtimeState: String
    public var lastError: String?
    public var updatedAt: String

    public init(
        name: String,
        cidr: String,
        mode: String = "shared",
        labels: [String: String] = [:],
        project: String? = nil,
        stack: String? = nil,
        runtimeState: String = "active",
        lastError: String? = nil,
        updatedAt: String = ISO8601DateFormatter().string(from: Date())
    ) {
        self.name = name
        self.cidr = cidr
        self.mode = mode
        self.labels = labels
        self.project = project
        self.stack = stack
        self.runtimeState = runtimeState
        self.lastError = lastError
        self.updatedAt = updatedAt
    }

    public var json: JSONValue {
        .object([
            "name": .string(name),
            "cidr": .string(cidr),
            "mode": .string(mode),
            "gateway": .string(IPv4CIDR.gateway(for: cidr)),
            "dns": .string(IPv4CIDR.gateway(for: cidr)),
            "router": .string(IPv4CIDR.router(for: cidr)),
            "guest_range": .string(".10+"),
            "labels": .object(labels.mapValues(JSONValue.string)),
            "project": project.map(JSONValue.string) ?? .null,
            "stack": stack.map(JSONValue.string) ?? .null,
            "runtime_state": .string(runtimeState),
            "last_error": lastError.map(JSONValue.string) ?? .null,
            "updated_at": .string(updatedAt),
        ])
    }
}

public struct NetworkAttachmentRecord: Equatable, Sendable {
    public var vmID: String
    public var networkName: String
    public var ip: String
    public var labels: [String: String]
    public var project: String?
    public var stack: String?
    public var updatedAt: String

    public init(
        vmID: String,
        networkName: String,
        ip: String,
        labels: [String: String] = [:],
        project: String? = nil,
        stack: String? = nil,
        updatedAt: String = ISO8601DateFormatter().string(from: Date())
    ) {
        self.vmID = vmID
        self.networkName = networkName
        self.ip = ip
        self.labels = labels
        self.project = project
        self.stack = stack
        self.updatedAt = updatedAt
    }

    public var json: JSONValue {
        .object([
            "vm_id": .string(vmID),
            "network": .string(networkName),
            "ip": .string(ip),
            "labels": .object(labels.mapValues(JSONValue.string)),
            "project": project.map(JSONValue.string) ?? .null,
            "stack": stack.map(JSONValue.string) ?? .null,
            "updated_at": .string(updatedAt),
        ])
    }
}

public struct DefaultNetworkRecord: Equatable, Sendable {
    public var name: String
    public var cidr: String
    public var updatedAt: String

    public init(
        name: String,
        cidr: String,
        updatedAt: String = ISO8601DateFormatter().string(from: Date())
    ) {
        self.name = name
        self.cidr = cidr
        self.updatedAt = updatedAt
    }

    public func json(network: NetworkRecord?) -> JSONValue {
        .object([
            "name": .string(name),
            "cidr": .string(cidr),
            "mode": .string("shared"),
            "access": .string("full"),
            "nat_egress": .bool(true),
            "network_exists": .bool(network != nil),
            "network": network.map(\.json) ?? .null,
            "updated_at": .string(updatedAt),
        ])
    }
}

public struct PortForwardRecord: Equatable, Sendable {
    public var bind: String
    public var hostPort: UInt16
    public var guestIP: String
    public var guestPort: UInt16
    public var vmID: String
    public var source: String
    public var project: String
    public var stack: String
    public var state: String
    public var updatedAt: String

    public init(
        bind: String,
        hostPort: UInt16,
        guestIP: String,
        guestPort: UInt16,
        vmID: String,
        source: String,
        project: String,
        stack: String,
        state: String = "active",
        updatedAt: String = ISO8601DateFormatter().string(from: Date())
    ) {
        self.bind = bind
        self.hostPort = hostPort
        self.guestIP = guestIP
        self.guestPort = guestPort
        self.vmID = vmID
        self.source = source
        self.project = project
        self.stack = stack
        self.state = state
        self.updatedAt = updatedAt
    }

    public var key: String {
        "\(bind):\(hostPort)"
    }

    public var json: JSONValue {
        .object([
            "bind": .string(bind),
            "host_port": .number(Double(hostPort)),
            "guest_ip": .string(guestIP),
            "guest_port": .number(Double(guestPort)),
            "vm_id": .string(vmID),
            "source": .string(source),
            "project": .string(project),
            "stack": .string(stack),
            "state": .string(state),
            "updated_at": .string(updatedAt),
        ])
    }
}

public extension IPv4CIDR {
    static func gateway(for cidr: String) -> String {
        (try? IPv4CIDR(cidr)).map { string($0.network) } ?? ""
    }

    static func router(for cidr: String) -> String {
        (try? IPv4CIDR(cidr)).map { string($0.network + 2) } ?? ""
    }
}
