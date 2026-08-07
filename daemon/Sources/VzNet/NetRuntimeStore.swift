import Darwin
import Foundation
import VzDaemonKit

struct RuntimeNetwork: Sendable {
    var name: String
    var cidr: String
    var mode: String
    var natEgress: Bool

    var gateway: String { IPv4CIDR.gateway(for: cidr) }

    var info: VzNetNetworkInfo {
        VzNetNetworkInfo(
            name: name,
            cidr: cidr,
            mode: mode,
            natEgress: natEgress,
            gateway: gateway
        )
    }

    func matches(cidr: String, mode: String, natEgress: Bool) -> Bool {
        self.cidr == cidr && self.mode == mode && self.natEgress == natEgress
    }
}

enum NetRuntimeError: Error, CustomStringConvertible {
    case invalid(String)
    case conflict(String)
    case notFound(String)
    case runtime(String)

    var description: String {
        switch self {
        case let .invalid(message), let .conflict(message),
             let .notFound(message), let .runtime(message):
            return message
        }
    }

    var rpcCode: Int {
        switch self {
        case .invalid:
            return -32602
        case .conflict, .notFound:
            return -32031
        case .runtime:
            return -32032
        }
    }
}

final class NetRuntimeStore: @unchecked Sendable {
    private let lock = NSLock()
    private var records: [String: RuntimeNetwork] = [:]
    private var handles: [String: NativeVmnetHandle] = [:]
    private var stopped = false

    func acquire(
        name: String,
        cidr rawCIDR: String,
        mode: String,
        natEgress: Bool
    ) throws -> VzNetNetworkInfo {
        try lock.withLock {
            try requireRunning()
            let cidr: IPv4CIDR
            do {
                cidr = try IPv4CIDR(rawCIDR)
            } catch {
                throw NetRuntimeError.invalid(String(describing: error))
            }
            guard mode == "shared" else {
                throw NetRuntimeError.invalid(
                    "bridged mode is unsupported in v0.1; use --mode shared"
                )
            }
            if let existing = records[name] {
                guard existing.matches(
                    cidr: cidr.canonical,
                    mode: mode,
                    natEgress: natEgress
                ) else {
                    throw NetRuntimeError.conflict(
                        "network \(name) exists with different config"
                    )
                }
                guard handles[name] != nil else {
                    throw NetRuntimeError.runtime(
                        "network \(name) is recorded but has no live handle"
                    )
                }
                return existing.info
            }
            if let conflict = records.values.first(where: { $0.cidr == cidr.canonical }) {
                throw NetRuntimeError.conflict(
                    "CIDR already reserved: \(cidr.canonical) by \(conflict.name)"
                )
            }
            let handle: NativeVmnetHandle
            do {
                handle = try NativeVmnetRuntime.reserve(
                    name: name,
                    cidr: cidr.canonical,
                    mode: mode,
                    natEgress: natEgress
                )
            } catch {
                throw NetRuntimeError.runtime(String(describing: error))
            }
            let record = RuntimeNetwork(
                name: name,
                cidr: cidr.canonical,
                mode: mode,
                natEgress: natEgress
            )
            records[name] = record
            handles[name] = handle
            return record.info
        }
    }

    func release(name: String) throws -> VzNetNetworkInfo {
        try lock.withLock {
            try requireRunning()
            guard let record = records.removeValue(forKey: name) else {
                throw NetRuntimeError.notFound("network not found: \(name)")
            }
            handles.removeValue(forKey: name)
            return record.info
        }
    }

    func list() throws -> [VzNetNetworkInfo] {
        try lock.withLock {
            try requireRunning()
            return records.values.sorted { $0.name < $1.name }.map(\.info)
        }
    }

    func serialize(name: String) throws -> Data {
        try lock.withLock {
            try requireRunning()
            guard let handle = handles[name] else {
                throw NetRuntimeError.notFound("network not found: \(name)")
            }
            do {
                return try VmnetSerialization.blob(from: handle.network)
            } catch {
                throw NetRuntimeError.runtime(
                    "cannot serialize network \(name): \(error)"
                )
            }
        }
    }

    func networkCount() -> Int {
        lock.withLock { records.count }
    }

    func verify() throws -> [JSONValue] {
        try lock.withLock {
            try requireRunning()
            let addresses = hostIPv4Addresses()
            return records.values.sorted { $0.name < $1.name }.map { record in
                let refOK = handles[record.name] != nil
                var serializationOK = false
                var verificationError: String?
                if let handle = handles[record.name] {
                    do {
                        _ = try VmnetSerialization.blob(from: handle.network)
                        serializationOK = true
                    } catch {
                        verificationError = String(describing: error)
                    }
                } else {
                    verificationError = "missing live vmnet handle"
                }
                let bridgeOK = addresses.contains(record.gateway)
                if !bridgeOK, verificationError == nil {
                    verificationError = "host bridge gateway \(record.gateway) is missing"
                }
                return .object([
                    "name": .string(record.name),
                    "cidr": .string(record.cidr),
                    "ref_ok": .bool(refOK),
                    "serialization_ok": .bool(serializationOK),
                    "bridge_ok": .bool(bridgeOK),
                    "error": verificationError.map(JSONValue.string) ?? .null,
                ])
            }
        }
    }

    func shutdown() {
        lock.withLock {
            stopped = true
            handles.removeAll()
            records.removeAll()
        }
    }

    private func requireRunning() throws {
        if stopped {
            throw NetRuntimeError.runtime("vz-net is shutting down")
        }
    }
}

private func hostIPv4Addresses() -> Set<String> {
    var result = Set<String>()
    var first: UnsafeMutablePointer<ifaddrs>?
    guard getifaddrs(&first) == 0, let first else { return result }
    defer { freeifaddrs(first) }
    var current: UnsafeMutablePointer<ifaddrs>? = first
    while let item = current?.pointee {
        defer { current = item.ifa_next }
        guard let address = item.ifa_addr,
              address.pointee.sa_family == UInt8(AF_INET)
        else { continue }
        var value = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
            $0.pointee.sin_addr
        }
        var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &value, &buffer, socklen_t(INET_ADDRSTRLEN)) != nil else {
            continue
        }
        result.insert(String(
            decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) },
            as: UTF8.self
        ))
    }
    return result
}
