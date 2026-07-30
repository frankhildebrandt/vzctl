import CoreFoundation
import Foundation
import VzDaemonKit
import vmnet

enum NetworkRegistryError: Error, CustomStringConvertible {
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
        case .conflict, .notFound, .runtime:
            return -32031
        }
    }
}

protocol NetworkRuntimeHandle: AnyObject, Sendable {}

protocol NetworkRuntimeBackend: Sendable {
    func reserve(_ network: NetworkRecord) throws -> any NetworkRuntimeHandle
}

final class NativeVmnetHandle: NetworkRuntimeHandle, @unchecked Sendable {
    let network: vmnet_network_ref

    init(network: vmnet_network_ref) {
        self.network = network
    }

    deinit {
        // vmnet_network_ref is an opaque CF_RETURNS_RETAINED C handle; Swift ARC
        // does not release it automatically.
        releaseOpaqueCF(network)
    }
}

struct NativeVmnetBackend: NetworkRuntimeBackend {
    func reserve(_ record: NetworkRecord) throws -> any NetworkRuntimeHandle {
        guard record.mode == "shared" else {
            throw NetworkRegistryError.invalid(
                "bridged mode is unsupported in v0.1; use --mode shared"
            )
        }
        let cidr: IPv4CIDR
        do {
            cidr = try IPv4CIDR(record.cidr)
        } catch {
            throw NetworkRegistryError.invalid(String(describing: error))
        }

        var status: vmnet_return_t = .VMNET_SUCCESS
        guard let configuration = vmnet_network_configuration_create(.VMNET_SHARED_MODE, &status)
        else {
            throw NetworkRegistryError.runtime(
                "vmnet configuration for \(record.name) failed (\(status.rawValue))"
            )
        }
        defer { releaseOpaqueCF(configuration) }
        var subnet = cidr.subnetAddress
        var mask = cidr.maskAddress
        status = vmnet_network_configuration_set_ipv4_subnet(configuration, &subnet, &mask)
        guard status == .VMNET_SUCCESS else {
            throw NetworkRegistryError.runtime(
                "vmnet subnet \(record.cidr) failed (\(status.rawValue))"
            )
        }
        vmnet_network_configuration_disable_dhcp(configuration)
        vmnet_network_configuration_disable_dns_proxy(configuration)

        guard let network = vmnet_network_create(configuration, &status) else {
            throw NetworkRegistryError.runtime(
                "vmnet reserve \(record.cidr) failed (\(status.rawValue)); "
                    + "after an unclean exit this CIDR may remain orphaned until reboot"
            )
        }
        return NativeVmnetHandle(network: network)
    }
}

private func releaseOpaqueCF(_ pointer: OpaquePointer) {
    Unmanaged<AnyObject>.fromOpaque(UnsafeRawPointer(pointer)).release()
}

struct NetworkSnapshot: Sendable {
    let networks: [NetworkRecord]
    let attachments: [NetworkAttachmentRecord]

    var json: JSONValue {
        .object([
            "networks": .array(networks.map(\.json)),
            "attachments": .array(attachments.map(\.json)),
        ])
    }
}

final class NetworkRegistry: @unchecked Sendable {
    private let database: StateDatabase
    private let backend: any NetworkRuntimeBackend
    private let lock = NSLock()
    private var handles: [String: any NetworkRuntimeHandle] = [:]
    private var stopped = false

    init(database: StateDatabase, backend: any NetworkRuntimeBackend = NativeVmnetBackend()) throws {
        self.database = database
        self.backend = backend
        try rebuild()
    }

    func create(
        name: String,
        cidr rawCIDR: String,
        mode: String,
        labels: [String: String],
        project: String?,
        stack: String?
    ) throws -> NetworkRecord {
        try lock.withLock {
            try requireRunning()
            try validateName(name, kind: "network")
            guard mode == "shared" else {
                throw NetworkRegistryError.invalid(
                    "bridged mode is unsupported in v0.1; use --mode shared"
                )
            }
            try validateMetadata(labels: labels, project: project, stack: stack)
            let cidr: IPv4CIDR
            do {
                cidr = try IPv4CIDR(rawCIDR)
            } catch {
                throw NetworkRegistryError.invalid(String(describing: error))
            }
            let existing = try database.networks()
            guard !existing.contains(where: { $0.name == name }) else {
                throw NetworkRegistryError.conflict("network already exists: \(name)")
            }
            guard !existing.contains(where: { $0.cidr == cidr.canonical }) else {
                throw NetworkRegistryError.conflict("CIDR already reserved: \(cidr.canonical)")
            }

            var record = NetworkRecord(
                name: name,
                cidr: cidr.canonical,
                mode: mode,
                labels: labels,
                project: project,
                stack: stack
            )
            let handle = try backend.reserve(record)
            do {
                try database.insertNetwork(record)
            } catch {
                throw NetworkRegistryError.conflict("cannot persist network \(name): \(error)")
            }
            handles[name] = handle
            record.runtimeState = "active"
            return record
        }
    }

    func attach(
        vmID: String,
        networkName: String,
        ip: String,
        labels: [String: String],
        project: String?,
        stack: String?,
        vmIsStopped: Bool
    ) throws -> NetworkAttachmentRecord {
        try lock.withLock {
            try requireRunning()
            try validateName(vmID, kind: "VM")
            try validateName(networkName, kind: "network")
            try validateMetadata(labels: labels, project: project, stack: stack)
            guard vmIsStopped else {
                throw NetworkRegistryError.conflict(
                    "VM \(vmID) must be stopped before changing network attachments"
                )
            }
            guard let network = try database.networks().first(where: { $0.name == networkName })
            else {
                throw NetworkRegistryError.notFound("network not found: \(networkName)")
            }
            guard handles[networkName] != nil else {
                throw NetworkRegistryError.runtime(
                    "network \(networkName) is not active: \(network.lastError ?? "rebuild failed")"
                )
            }
            let cidr = try IPv4CIDR(network.cidr)
            guard cidr.containsGuest(ip) else {
                throw NetworkRegistryError.invalid(
                    NetworkValidationError.invalidIP(ip, cidr: network.cidr).description
                )
            }
            let record = NetworkAttachmentRecord(
                vmID: vmID,
                networkName: networkName,
                ip: ip,
                labels: labels,
                project: project ?? network.project,
                stack: stack ?? network.stack
            )
            do {
                try database.insertAttachment(record)
            } catch {
                throw NetworkRegistryError.conflict(
                    "cannot attach \(vmID) to \(networkName): VM/network or IP already attached"
                )
            }
            return record
        }
    }

    func detach(vmID: String, networkName: String, vmIsStopped: Bool) throws {
        try lock.withLock {
            try requireRunning()
            guard vmIsStopped else {
                throw NetworkRegistryError.conflict(
                    "VM \(vmID) must be stopped before changing network attachments"
                )
            }
            do {
                try database.deleteAttachment(vmID: vmID, networkName: networkName)
            } catch {
                throw NetworkRegistryError.notFound(
                    "attachment not found: \(vmID) on \(networkName)"
                )
            }
        }
    }

    func delete(name: String) throws {
        try lock.withLock {
            try requireRunning()
            let attachments = try database.attachments().filter { $0.networkName == name }
            guard attachments.isEmpty else {
                let VMs = attachments.map(\.vmID).sorted().joined(separator: ", ")
                throw NetworkRegistryError.conflict(
                    "network \(name) still has attached VMs: \(VMs)"
                )
            }
            guard try database.networks().contains(where: { $0.name == name }) else {
                throw NetworkRegistryError.notFound("network not found: \(name)")
            }
            do {
                try database.deleteNetwork(name: name)
            } catch {
                throw NetworkRegistryError.conflict("cannot delete network \(name): \(error)")
            }
            // Dropping the last strong reference releases the vmnet reservation.
            handles.removeValue(forKey: name)
        }
    }

    func snapshot() throws -> NetworkSnapshot {
        try lock.withLock {
            NetworkSnapshot(
                networks: try database.networks(),
                attachments: try database.attachments()
            )
        }
    }

    func shutdown() {
        lock.withLock {
            // Required by G0: release every vmnet_network_ref, not only interfaces.
            stopped = true
            handles.removeAll()
        }
    }

    private func rebuild() throws {
        for var record in try database.networks() {
            do {
                let handle = try backend.reserve(record)
                try database.updateNetworkRuntime(name: record.name, state: "active", error: nil)
                handles[record.name] = handle
            } catch {
                record.runtimeState = "orphaned"
                record.lastError = String(describing: error)
                try database.updateNetworkRuntime(
                    name: record.name,
                    state: record.runtimeState,
                    error: record.lastError
                )
            }
        }
    }

    private func requireRunning() throws {
        guard !stopped else {
            throw NetworkRegistryError.runtime("network registry is stopping")
        }
    }

    private func validateName(_ value: String, kind: String) throws {
        let valid = !value.isEmpty
            && value.count <= 128
            && value.unicodeScalars.allSatisfy {
                CharacterSet.alphanumerics.contains($0) || "-_./".unicodeScalars.contains($0)
            }
            && value != "."
            && value != ".."
            && !value.contains("..")
        guard valid else {
            throw NetworkRegistryError.invalid("invalid \(kind) name: \(value)")
        }
    }

    private func validateMetadata(
        labels: [String: String],
        project: String?,
        stack: String?
    ) throws {
        guard labels.count <= 64,
              labels.allSatisfy({ !$0.key.isEmpty && $0.key.count <= 128 && $0.value.count <= 256 }),
              project.map({ !$0.isEmpty && $0.count <= 128 }) ?? true,
              stack.map({ !$0.isEmpty && $0.count <= 128 }) ?? true
        else {
            throw NetworkRegistryError.invalid("invalid labels/project/stack metadata")
        }
    }
}
