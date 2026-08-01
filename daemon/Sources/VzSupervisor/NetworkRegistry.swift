import CoreFoundation
import Dispatch
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

/// Holds the process-local vmnet reservation plus a host-side interface.
///
/// G0: `.0` is only bindable after `vmnet_interface_start_with_network`
/// (bridge inet appears). Without that, DNS/ingress fail with EADDRNOTAVAIL.
final class NativeVmnetHandle: NetworkRuntimeHandle, @unchecked Sendable {
    let network: vmnet_network_ref
    private let interface: interface_ref?

    init(network: vmnet_network_ref, interface: interface_ref?) {
        self.network = network
        self.interface = interface
    }

    deinit {
        if let interface {
            let queue = DispatchQueue(label: "vzctl.vmnet.stop")
            let sem = DispatchSemaphore(value: 0)
            let status = vmnet_stop_interface(interface, queue) { _ in
                sem.signal()
            }
            if status == .VMNET_SUCCESS {
                _ = sem.wait(timeout: .now() + .seconds(5))
            }
        }
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

        // natEgress:false → host-only (no Internet NAT); true → shared NAT44.
        let operationMode: operating_modes_t =
            record.natEgress ? .VMNET_SHARED_MODE : .VMNET_HOST_MODE

        var status: vmnet_return_t = .VMNET_SUCCESS
        guard let configuration = vmnet_network_configuration_create(operationMode, &status)
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
        do {
            let interface = try Self.startHostInterface(network: network, name: record.name)
            return NativeVmnetHandle(network: network, interface: interface)
        } catch {
            releaseOpaqueCF(network)
            throw error
        }
    }

    /// Activates the host bridge so gateway `.0` appears and is bindable.
    private static func startHostInterface(
        network: vmnet_network_ref,
        name: String
    ) throws -> interface_ref {
        let desc = xpc_dictionary_create(nil, nil, 0)
        xpc_dictionary_set_bool(desc, vmnet_allocate_mac_address_key, true)

        let queue = DispatchQueue(label: "vzctl.vmnet.\(name)")
        let sem = DispatchSemaphore(value: 0)
        var completionStatus: vmnet_return_t = .VMNET_FAILURE

        guard let iface = vmnet_interface_start_with_network(network, desc, queue, { status, _ in
            completionStatus = status
            sem.signal()
        }) else {
            throw NetworkRegistryError.runtime(
                "vmnet interface start for \(name) returned nil"
            )
        }

        let wait = sem.wait(timeout: .now() + .seconds(15))
        guard wait == .success else {
            _ = vmnet_stop_interface(iface, queue) { _ in }
            throw NetworkRegistryError.runtime(
                "vmnet interface start for \(name) timed out"
            )
        }
        guard completionStatus == .VMNET_SUCCESS else {
            _ = vmnet_stop_interface(iface, queue) { _ in }
            throw NetworkRegistryError.runtime(
                "vmnet interface start for \(name) failed (\(completionStatus.rawValue))"
            )
        }
        return iface
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

struct SerializedVmnetAttachment: Sendable {
    let networkName: String
    let ip: String
    let blob: Data

    var json: JSONValue {
        .object([
            "network": .string(networkName),
            "ip": .string(ip),
            "serialization": .string(VmnetSerialization.base64(from: blob)),
        ])
    }
}

struct VMNetworkSelection: Sendable {
    let network: NetworkRecord
    let attachment: NetworkAttachmentRecord
    let automatic: Bool
    let created: Bool

    var json: JSONValue {
        .object([
            "network": network.json,
            "attachment": attachment.json,
            "automatic": .bool(automatic),
            "created": .bool(created),
            "prefix": .number(Double((try? IPv4CIDR(network.cidr).prefix) ?? 0)),
        ])
    }
}

final class NetworkRegistry: @unchecked Sendable {
    static let automaticLabel = "vzctl.dev/default-attachment"
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
        natEgress: Bool = true,
        backend: String = NetworkRecord.backendVmnet,
        labels: [String: String],
        project: String?,
        stack: String?
    ) throws -> NetworkRecord {
        try lock.withLock {
            try requireRunning()
            return try createLocked(
                name: name,
                cidr: rawCIDR,
                mode: mode,
                natEgress: natEgress,
                backend: backend,
                labels: labels,
                project: project,
                stack: stack
            )
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
            if !network.isDockerBackend {
                guard handles[networkName] != nil else {
                    throw NetworkRegistryError.runtime(
                        "network \(networkName) is not active: \(network.lastError ?? "rebuild failed")"
                    )
                }
            }
            let cidr = try IPv4CIDR(network.cidr)
            guard cidr.containsAttachment(ip) else {
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
                let current = try database.attachments().filter { $0.vmID == vmID }
                if current.contains(where: { $0.networkName == networkName }) {
                    try database.updateAttachment(record)
                } else {
                    try database.insertAttachment(record)
                }
                for old in current where old.labels[Self.automaticLabel] == "true"
                    && old.networkName != networkName
                {
                    try database.deleteAttachment(
                        vmID: old.vmID,
                        networkName: old.networkName
                    )
                }
            } catch {
                throw NetworkRegistryError.conflict(
                    "cannot attach \(vmID) to \(networkName): VM/network or IP already attached"
                )
            }
            return record
        }
    }

    func setDefault(name: String, cidr rawCIDR: String) throws -> DefaultNetworkRecord {
        try lock.withLock {
            try requireRunning()
            try validateName(name, kind: "network")
            let cidr: IPv4CIDR
            do {
                cidr = try IPv4CIDR(rawCIDR)
            } catch {
                throw NetworkRegistryError.invalid(String(describing: error))
            }
            _ = try ensureDefaultNetworkLocked(name: name, cidr: cidr.canonical)
            let current = try database.defaultNetwork()
            if current?.name == name, current?.cidr == cidr.canonical {
                return current!
            }
            let record = DefaultNetworkRecord(name: name, cidr: cidr.canonical)
            try database.setDefaultNetwork(record)
            return record
        }
    }

    func defaultNetwork() throws -> (DefaultNetworkRecord, NetworkRecord?)? {
        try lock.withLock {
            guard let configured = try database.defaultNetwork() else { return nil }
            let network = try database.networks().first {
                $0.name == configured.name
                    && $0.cidr == configured.cidr
                    && $0.mode == "shared"
            }
            return (configured, network)
        }
    }

    func ensureVMNetwork(
        vmID: String,
        requestedNetwork: String?,
        vmIsStopped: Bool
    ) throws -> VMNetworkSelection {
        try lock.withLock {
            try requireRunning()
            try validateName(vmID, kind: "VM")
            guard vmIsStopped else {
                throw NetworkRegistryError.conflict(
                    "VM \(vmID) must be stopped before changing network attachments"
                )
            }
            let current = try database.attachments().filter { $0.vmID == vmID }
            let explicit = current.filter { $0.labels[Self.automaticLabel] != "true" }

            let network: NetworkRecord
            let automatic: Bool
            if let requestedNetwork {
                try validateName(requestedNetwork, kind: "network")
                // Multi-homed VMs (attach_nets before create) already have several
                // explicit attachments — match by name, not only the first row.
                if let existing = explicit.first(where: { $0.networkName == requestedNetwork }),
                   let selected = try database.networks().first(where: {
                       $0.name == existing.networkName
                   })
                {
                    return VMNetworkSelection(
                        network: selected,
                        attachment: existing,
                        automatic: false,
                        created: false
                    )
                }
                if !explicit.isEmpty {
                    throw NetworkRegistryError.conflict(
                        "VM \(vmID) already has explicit network attachments"
                    )
                }
                guard let selected = try database.networks().first(where: {
                    $0.name == requestedNetwork
                }) else {
                    throw NetworkRegistryError.notFound("network not found: \(requestedNetwork)")
                }
                network = selected
                automatic = false
            } else {
                if let existing = explicit.first,
                   let selected = try database.networks().first(where: {
                       $0.name == existing.networkName
                   })
                {
                    return VMNetworkSelection(
                        network: selected,
                        attachment: existing,
                        automatic: false,
                        created: false
                    )
                }
                guard let configured = try database.defaultNetwork() else {
                    throw NetworkRegistryError.notFound(
                        "default network is not configured; run vzctl net default set <name> --cidr <CIDR>"
                    )
                }
                network = try ensureDefaultNetworkLocked(
                    name: configured.name,
                    cidr: configured.cidr
                )
                automatic = true
            }

            if let existing = current.first(where: { $0.networkName == network.name }) {
                if !automatic, existing.labels[Self.automaticLabel] == "true" {
                    var promoted = existing
                    promoted.labels.removeValue(forKey: Self.automaticLabel)
                    promoted.updatedAt = ISO8601DateFormatter().string(from: Date())
                    try database.updateAttachment(promoted)
                    return VMNetworkSelection(
                        network: network,
                        attachment: promoted,
                        automatic: false,
                        created: false
                    )
                }
                return VMNetworkSelection(
                    network: network,
                    attachment: existing,
                    automatic: automatic,
                    created: false
                )
            }

            let used = Set(
                try database.attachments()
                    .filter { $0.networkName == network.name }
                    .map(\.ip)
            )
            let cidr = try IPv4CIDR(network.cidr)
            var offset: UInt32 = 10
            var ip: String?
            while let candidate = cidr.guestAddress(offset: offset) {
                if !used.contains(candidate) {
                    ip = candidate
                    break
                }
                offset += 1
            }
            guard let ip else {
                throw NetworkRegistryError.conflict(
                    "no free guest IP in network \(network.name)"
                )
            }
            var labels: [String: String] = [:]
            if automatic { labels[Self.automaticLabel] = "true" }
            let record = NetworkAttachmentRecord(
                vmID: vmID,
                networkName: network.name,
                ip: ip,
                labels: labels,
                project: network.project,
                stack: network.stack
            )
            do {
                try database.insertAttachment(record)
                for old in current where old.labels[Self.automaticLabel] == "true" {
                    try database.deleteAttachment(
                        vmID: old.vmID,
                        networkName: old.networkName
                    )
                }
            } catch {
                throw NetworkRegistryError.conflict(
                    "cannot allocate network for \(vmID): \(error)"
                )
            }
            return VMNetworkSelection(
                network: network,
                attachment: record,
                automatic: automatic,
                created: true
            )
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

    /// Detaches every network attachment for a VM. Caller must ensure the VM is stopped
    /// (or helper state already cleared for orphan purge).
    @discardableResult
    func detachAll(vmID: String, vmIsStopped: Bool) throws -> [String] {
        try lock.withLock {
            try requireRunning()
            guard vmIsStopped else {
                throw NetworkRegistryError.conflict(
                    "VM \(vmID) must be stopped before changing network attachments"
                )
            }
            let names = try database.attachments()
                .filter { $0.vmID == vmID }
                .map(\.networkName)
            let removed = try database.deleteAttachments(vmID: vmID)
            _ = removed
            return names.sorted()
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

    /// Portable vmnet blobs for every attachment of `vmID` (supervisor-owned refs stay live).
    /// Docker-backend attachments are logical (docker0) and are omitted from helper NICs.
    func serializedAttachments(for vmID: String) throws -> [SerializedVmnetAttachment] {
        try lock.withLock {
            try requireRunning()
            let networks = Dictionary(uniqueKeysWithValues: try database.networks().map { ($0.name, $0) })
            let attachments = try database.attachments()
                .filter { $0.vmID == vmID }
                .sorted { lhs, rhs in
                    if lhs.networkName != rhs.networkName {
                        return lhs.networkName < rhs.networkName
                    }
                    return lhs.ip < rhs.ip
                }
            return try attachments.compactMap { attachment in
                if networks[attachment.networkName]?.isDockerBackend == true {
                    return nil
                }
                guard let handle = handles[attachment.networkName] as? NativeVmnetHandle else {
                    throw NetworkRegistryError.runtime(
                        "network \(attachment.networkName) is not active for helper attach"
                    )
                }
                let blob: Data
                do {
                    blob = try VmnetSerialization.blob(from: handle.network)
                } catch {
                    throw NetworkRegistryError.runtime(
                        "cannot serialize network \(attachment.networkName): \(error)"
                    )
                }
                return SerializedVmnetAttachment(
                    networkName: attachment.networkName,
                    ip: attachment.ip,
                    blob: blob
                )
            }
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
            if record.isDockerBackend {
                try database.updateNetworkRuntime(name: record.name, state: "active", error: nil)
                continue
            }
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

    private func ensureDefaultNetworkLocked(name: String, cidr: String) throws -> NetworkRecord {
        if let existing = try database.networks().first(where: { $0.name == name }) {
            guard existing.cidr == cidr, existing.mode == "shared" else {
                throw NetworkRegistryError.conflict(
                    "network \(name) exists with \(existing.cidr), expected \(cidr)"
                )
            }
            guard handles[name] != nil else {
                throw NetworkRegistryError.runtime(
                    "network \(name) is not active: \(existing.lastError ?? "rebuild failed")"
                )
            }
            return existing
        }
        return try createLocked(
            name: name,
            cidr: cidr,
            mode: "shared",
            labels: [:],
            project: nil,
            stack: nil
        )
    }

    private func createLocked(
        name: String,
        cidr rawCIDR: String,
        mode: String,
        natEgress: Bool = true,
        backend: String = NetworkRecord.backendVmnet,
        labels: [String: String],
        project: String?,
        stack: String?
    ) throws -> NetworkRecord {
        try validateName(name, kind: "network")
        guard mode == "shared" else {
            throw NetworkRegistryError.invalid(
                "bridged mode is unsupported in v0.1; use --mode shared"
            )
        }
        let normalizedBackend = backend.isEmpty ? NetworkRecord.backendVmnet : backend
        guard normalizedBackend == NetworkRecord.backendVmnet
            || normalizedBackend == NetworkRecord.backendDocker
        else {
            throw NetworkRegistryError.invalid(
                "unsupported network backend \(backend); use vmnet or docker"
            )
        }
        if normalizedBackend == NetworkRecord.backendDocker, natEgress {
            throw NetworkRegistryError.invalid(
                "docker backend networks must set nat_egress=false"
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
            natEgress: natEgress,
            backend: normalizedBackend,
            labels: labels,
            project: project,
            stack: stack
        )
        if record.isDockerBackend {
            do {
                try database.insertNetwork(record)
            } catch {
                throw NetworkRegistryError.conflict("cannot persist network \(name): \(error)")
            }
            record.runtimeState = "active"
            return record
        }
        let handle = try self.backend.reserve(record)
        do {
            try database.insertNetwork(record)
        } catch {
            throw NetworkRegistryError.conflict("cannot persist network \(name): \(error)")
        }
        handles[name] = handle
        record.runtimeState = "active"
        return record
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
