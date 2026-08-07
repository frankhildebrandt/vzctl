import Foundation
import VzDaemonKit

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
    /// When true, dropping handles on CP shutdown releases the runtime reservation
    /// (in-process / test backends). Remote `vz-net` returns false so CP crashes
    /// do not orphan-or-release CIDRs held by `vz-net`.
    var releasesOnShutdown: Bool { get }

    func reserve(_ network: NetworkRecord) throws -> any NetworkRuntimeHandle
    func serialize(name: String, handle: any NetworkRuntimeHandle) throws -> Data
    func release(name: String, handle: any NetworkRuntimeHandle) throws
}

/// Marker handle for a network reserved in `vz-net`.
final class RemoteVmnetHandle: NetworkRuntimeHandle, @unchecked Sendable {
    let name: String
    init(name: String) { self.name = name }
}

struct RemoteVzNetBackend: NetworkRuntimeBackend {
    let client: VzNetClient
    var releasesOnShutdown: Bool { false }

    func reserve(_ network: NetworkRecord) throws -> any NetworkRuntimeHandle {
        do {
            _ = try client.acquire(
                name: network.name,
                cidr: network.cidr,
                mode: network.mode,
                natEgress: network.natEgress
            )
            return RemoteVmnetHandle(name: network.name)
        } catch let error as VzNetClientError {
            throw NetworkRegistryError.runtime(error.description)
        } catch {
            throw NetworkRegistryError.runtime(String(describing: error))
        }
    }

    func serialize(name: String, handle: any NetworkRuntimeHandle) throws -> Data {
        _ = handle
        do {
            return try client.serialize(name: name)
        } catch let error as VzNetClientError {
            throw NetworkRegistryError.runtime(error.description)
        } catch {
            throw NetworkRegistryError.runtime(String(describing: error))
        }
    }

    func release(name: String, handle: any NetworkRuntimeHandle) throws {
        _ = handle
        do {
            try client.release(name: name)
        } catch let error as VzNetClientError {
            throw NetworkRegistryError.runtime(error.description)
        } catch {
            throw NetworkRegistryError.runtime(String(describing: error))
        }
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

    init(
        database: StateDatabase,
        backend: any NetworkRuntimeBackend
    ) throws {
        self.database = database
        self.backend = backend
        try rebuild()
    }

    /// Production path: desired state in SQLite, runtime refs in `vz-net`.
    convenience init(database: StateDatabase, stateDirectory: URL) throws {
        let client = VzNetClient(
            socketPath: VzNetClient.defaultSocketPath(stateDirectory: stateDirectory)
        )
        try self.init(database: database, backend: RemoteVzNetBackend(client: client))
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
            if let handle = handles.removeValue(forKey: name) {
                try backend.release(name: name, handle: handle)
            }
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

    /// Last-resort recovery for an opt-in stack after every attached helper has
    /// stopped. Desired state and attachments stay unchanged.
    func recreateRuntime(name: String) throws {
        try lock.withLock {
            try requireRunning()
            guard let record = try database.networks().first(where: { $0.name == name }) else {
                throw NetworkRegistryError.notFound("network not found: \(name)")
            }
            guard !record.isDockerBackend else {
                throw NetworkRegistryError.invalid("docker backend has no vmnet runtime")
            }
            guard let current = handles.removeValue(forKey: name) else {
                throw NetworkRegistryError.runtime("network \(name) has no live handle")
            }
            do {
                try backend.release(name: name, handle: current)
                let replacement = try backend.reserve(record)
                handles[name] = replacement
                try database.updateNetworkRuntime(name: name, state: "active", error: nil)
            } catch {
                try? database.updateNetworkRuntime(
                    name: name,
                    state: "orphaned",
                    error: String(describing: error)
                )
                throw NetworkRegistryError.runtime(
                    "cannot recreate network \(name): \(error)"
                )
            }
        }
    }

    /// Portable vmnet blobs for every attachment of `vmID` (vz-net-owned refs stay live).
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
                guard let handle = handles[attachment.networkName] else {
                    throw NetworkRegistryError.runtime(
                        "network \(attachment.networkName) is not active for helper attach"
                    )
                }
                let blob: Data
                do {
                    blob = try backend.serialize(name: attachment.networkName, handle: handle)
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
            // Control-plane shutdown must NOT release vz-net refs (ADR 0002).
            // In-process/test backends still drop handles so deinit releases.
            stopped = true
            if backend.releasesOnShutdown {
                let snapshot = handles
                handles.removeAll()
                for (name, handle) in snapshot {
                    try? backend.release(name: name, handle: handle)
                }
            } else {
                handles.removeAll()
            }
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
