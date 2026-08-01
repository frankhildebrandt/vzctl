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
