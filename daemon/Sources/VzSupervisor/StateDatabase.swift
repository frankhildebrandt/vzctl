import Darwin
import Foundation
import SQLite3
import VzDaemonKit

final class StateDatabase {
    private var handle: OpaquePointer?
    private let reconcileLock = NSLock()

    init(path: String) throws {
        guard sqlite3_open_v2(
            path,
            &handle,
            SQLITE_OPEN_CREATE | SQLITE_OPEN_READWRITE | SQLITE_OPEN_FULLMUTEX,
            nil
        ) == SQLITE_OK else {
            let message = handle.map { String(cString: sqlite3_errmsg($0)) } ?? "unknown error"
            sqlite3_close(handle)
            throw SupervisorError.database("open: \(message)")
        }
        do {
            guard chmod(path, 0o600) == 0 else {
                throw SupervisorError.system("chmod state database", errno)
            }

            try execute("PRAGMA journal_mode=WAL;")
            try execute("PRAGMA foreign_keys=ON;")
            try execute(
                """
                CREATE TABLE IF NOT EXISTS resources (
                    id TEXT PRIMARY KEY,
                    kind TEXT NOT NULL,
                    name TEXT NOT NULL,
                    labels_json TEXT NOT NULL DEFAULT '{}',
                    state TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS journal (
                    id TEXT PRIMARY KEY,
                    stack_id TEXT NOT NULL,
                    generation INTEGER NOT NULL,
                    step TEXT NOT NULL,
                    status TEXT NOT NULL,
                    payload TEXT NOT NULL DEFAULT '{}',
                    error TEXT,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS locks (
                    stack_id TEXT PRIMARY KEY,
                    holder TEXT NOT NULL,
                    expires_at TEXT NOT NULL
                );
                CREATE UNIQUE INDEX IF NOT EXISTS one_incomplete_journal_per_stack
                ON journal(stack_id)
                WHERE status IN ('pending', 'running', 'failed');
                CREATE TABLE IF NOT EXISTS networks (
                    name TEXT PRIMARY KEY,
                    cidr TEXT NOT NULL UNIQUE,
                    mode TEXT NOT NULL CHECK (mode = 'shared'),
                    nat_egress INTEGER NOT NULL DEFAULT 1,
                    backend TEXT NOT NULL DEFAULT 'vmnet',
                    labels_json TEXT NOT NULL DEFAULT '{}',
                    project TEXT,
                    stack TEXT,
                    runtime_state TEXT NOT NULL DEFAULT 'pending',
                    last_error TEXT,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS network_attachments (
                    vm_id TEXT NOT NULL,
                    network_name TEXT NOT NULL,
                    ip TEXT NOT NULL,
                    labels_json TEXT NOT NULL DEFAULT '{}',
                    project TEXT,
                    stack TEXT,
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (vm_id, network_name),
                    UNIQUE (network_name, ip),
                    FOREIGN KEY (network_name) REFERENCES networks(name) ON DELETE RESTRICT
                );
                CREATE TABLE IF NOT EXISTS network_defaults (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    name TEXT NOT NULL,
                    cidr TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS port_forwards (
                    bind TEXT NOT NULL,
                    host_port INTEGER NOT NULL,
                    guest_ip TEXT NOT NULL,
                    guest_port INTEGER NOT NULL,
                    vm_id TEXT NOT NULL,
                    source TEXT NOT NULL,
                    project TEXT NOT NULL,
                    stack TEXT NOT NULL,
                    state TEXT NOT NULL DEFAULT 'active',
                    updated_at TEXT NOT NULL,
                    PRIMARY KEY (bind, host_port)
                );
                CREATE TABLE IF NOT EXISTS stacks (
                    id TEXT PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    opened_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS edge_projects (
                    project TEXT PRIMARY KEY,
                    host_services_json TEXT NOT NULL DEFAULT '[]',
                    dns_records_json TEXT NOT NULL DEFAULT '[]',
                    ingress_json TEXT,
                    oidc_json TEXT,
                    updated_at TEXT NOT NULL
                );
                CREATE TABLE IF NOT EXISTS edge_meta (
                    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                    generation INTEGER NOT NULL DEFAULT 0
                );
                INSERT OR IGNORE INTO edge_meta (singleton, generation) VALUES (1, 0);
                """
            )
            // Older DBs: add nat_egress if missing (ignore duplicate-column errors).
            _ = try? execute(
                "ALTER TABLE networks ADD COLUMN nat_egress INTEGER NOT NULL DEFAULT 1;"
            )
            // Older DBs: add backend if missing (vmnet | docker).
            _ = try? execute(
                "ALTER TABLE networks ADD COLUMN backend TEXT NOT NULL DEFAULT 'vmnet';"
            )
            _ = try? execute(
                "ALTER TABLE edge_projects ADD COLUMN dns_records_json TEXT NOT NULL DEFAULT '[]';"
            )
            try quickCheck()
        } catch {
            sqlite3_close(handle)
            handle = nil
            throw error
        }
    }

    deinit {
        sqlite3_close(handle)
    }

    private func execute(_ sql: String) throws {
        var error: UnsafeMutablePointer<CChar>?
        guard sqlite3_exec(handle, sql, nil, nil, &error) == SQLITE_OK else {
            let message = error.map { String(cString: $0) } ?? "unknown error"
            sqlite3_free(error)
            throw SupervisorError.database(message)
        }
    }

    private func quickCheck() throws {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(handle, "PRAGMA quick_check;", -1, &statement, nil) == SQLITE_OK else {
            throw SupervisorError.database("could not prepare quick_check")
        }
        defer { sqlite3_finalize(statement) }

        guard sqlite3_step(statement) == SQLITE_ROW,
              let value = sqlite3_column_text(statement, 0),
              String(cString: value) == "ok"
        else {
            throw SupervisorError.database("quick_check failed")
        }
    }

    func insertNetwork(_ record: NetworkRecord) throws {
        try withStatement(
            """
            INSERT INTO networks
                (name, cidr, mode, nat_egress, backend, labels_json, project, stack, runtime_state, last_error, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?);
            """
        ) { statement in
            try bind(record.name, at: 1, to: statement)
            try bind(record.cidr, at: 2, to: statement)
            try bind(record.mode, at: 3, to: statement)
            try bind(Int64(record.natEgress ? 1 : 0), at: 4, to: statement)
            try bind(record.backend, at: 5, to: statement)
            try bind(try labelsJSON(record.labels), at: 6, to: statement)
            try bind(record.project, at: 7, to: statement)
            try bind(record.stack, at: 8, to: statement)
            try bind(record.runtimeState, at: 9, to: statement)
            try bind(record.lastError, at: 10, to: statement)
            try bind(record.updatedAt, at: 11, to: statement)
            try stepDone(statement)
        }
    }

    func updateNetworkRuntime(name: String, state: String, error: String?) throws {
        try withStatement(
            "UPDATE networks SET runtime_state = ?, last_error = ?, updated_at = ? WHERE name = ?;"
        ) { statement in
            try bind(state, at: 1, to: statement)
            try bind(error, at: 2, to: statement)
            try bind(ISO8601DateFormatter().string(from: Date()), at: 3, to: statement)
            try bind(name, at: 4, to: statement)
            try stepDone(statement)
        }
    }

    func deleteNetwork(name: String) throws {
        try withStatement("DELETE FROM networks WHERE name = ?;") { statement in
            try bind(name, at: 1, to: statement)
            try stepDone(statement)
            guard sqlite3_changes(handle) == 1 else {
                throw SupervisorError.database("network not found: \(name)")
            }
        }
    }

    func networks() throws -> [NetworkRecord] {
        try withStatement(
            """
            SELECT name, cidr, mode, nat_egress, backend, labels_json, project, stack,
                   runtime_state, last_error, updated_at
            FROM networks ORDER BY name;
            """
        ) { statement in
            var records: [NetworkRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                records.append(
                    NetworkRecord(
                        name: text(statement, 0),
                        cidr: text(statement, 1),
                        mode: text(statement, 2),
                        natEgress: sqlite3_column_int(statement, 3) != 0,
                        backend: text(statement, 4),
                        labels: try labels(from: text(statement, 5)),
                        project: optionalText(statement, 6),
                        stack: optionalText(statement, 7),
                        runtimeState: text(statement, 8),
                        lastError: optionalText(statement, 9),
                        updatedAt: text(statement, 10)
                    )
                )
            }
            guard sqlite3_errcode(handle) == SQLITE_OK || sqlite3_errcode(handle) == SQLITE_DONE else {
                throw databaseError()
            }
            return records
        }
    }

    func insertAttachment(_ record: NetworkAttachmentRecord) throws {
        try withStatement(
            """
            INSERT INTO network_attachments
                (vm_id, network_name, ip, labels_json, project, stack, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?);
            """
        ) { statement in
            try bind(record.vmID, at: 1, to: statement)
            try bind(record.networkName, at: 2, to: statement)
            try bind(record.ip, at: 3, to: statement)
            try bind(try labelsJSON(record.labels), at: 4, to: statement)
            try bind(record.project, at: 5, to: statement)
            try bind(record.stack, at: 6, to: statement)
            try bind(record.updatedAt, at: 7, to: statement)
            try stepDone(statement)
        }
    }

    func updateAttachment(_ record: NetworkAttachmentRecord) throws {
        try withStatement(
            """
            UPDATE network_attachments
            SET ip = ?, labels_json = ?, project = ?, stack = ?, updated_at = ?
            WHERE vm_id = ? AND network_name = ?;
            """
        ) { statement in
            try bind(record.ip, at: 1, to: statement)
            try bind(try labelsJSON(record.labels), at: 2, to: statement)
            try bind(record.project, at: 3, to: statement)
            try bind(record.stack, at: 4, to: statement)
            try bind(record.updatedAt, at: 5, to: statement)
            try bind(record.vmID, at: 6, to: statement)
            try bind(record.networkName, at: 7, to: statement)
            try stepDone(statement)
            guard sqlite3_changes(handle) == 1 else {
                throw SupervisorError.database(
                    "attachment not found: \(record.vmID) on \(record.networkName)"
                )
            }
        }
    }

    func deleteAttachment(vmID: String, networkName: String) throws {
        try withStatement(
            "DELETE FROM network_attachments WHERE vm_id = ? AND network_name = ?;"
        ) { statement in
            try bind(vmID, at: 1, to: statement)
            try bind(networkName, at: 2, to: statement)
            try stepDone(statement)
            guard sqlite3_changes(handle) == 1 else {
                throw SupervisorError.database(
                    "attachment not found: \(vmID) on \(networkName)"
                )
            }
        }
    }

    /// Deletes every attachment row for a VM (orphan / force-purge cleanup).
    @discardableResult
    func deleteAttachments(vmID: String) throws -> Int {
        try withStatement("DELETE FROM network_attachments WHERE vm_id = ?;") { statement in
            try bind(vmID, at: 1, to: statement)
            try stepDone(statement)
            return Int(sqlite3_changes(handle))
        }
    }

    func attachments() throws -> [NetworkAttachmentRecord] {
        try withStatement(
            """
            SELECT vm_id, network_name, ip, labels_json, project, stack, updated_at
            FROM network_attachments ORDER BY network_name, vm_id;
            """
        ) { statement in
            var records: [NetworkAttachmentRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                records.append(
                    NetworkAttachmentRecord(
                        vmID: text(statement, 0),
                        networkName: text(statement, 1),
                        ip: text(statement, 2),
                        labels: try labels(from: text(statement, 3)),
                        project: optionalText(statement, 4),
                        stack: optionalText(statement, 5),
                        updatedAt: text(statement, 6)
                    )
                )
            }
            guard sqlite3_errcode(handle) == SQLITE_OK || sqlite3_errcode(handle) == SQLITE_DONE else {
                throw databaseError()
            }
            return records
        }
    }

    func setDefaultNetwork(_ record: DefaultNetworkRecord) throws {
        try withStatement(
            """
            INSERT INTO network_defaults (singleton, name, cidr, updated_at)
            VALUES (1, ?, ?, ?)
            ON CONFLICT(singleton) DO UPDATE SET
                name = excluded.name,
                cidr = excluded.cidr,
                updated_at = excluded.updated_at;
            """
        ) { statement in
            try bind(record.name, at: 1, to: statement)
            try bind(record.cidr, at: 2, to: statement)
            try bind(record.updatedAt, at: 3, to: statement)
            try stepDone(statement)
        }
    }

    func defaultNetwork() throws -> DefaultNetworkRecord? {
        try withStatement(
            "SELECT name, cidr, updated_at FROM network_defaults WHERE singleton = 1;"
        ) { statement in
            let result = sqlite3_step(statement)
            if result == SQLITE_DONE { return nil }
            guard result == SQLITE_ROW else { throw databaseError() }
            return DefaultNetworkRecord(
                name: text(statement, 0),
                cidr: text(statement, 1),
                updatedAt: text(statement, 2)
            )
        }
    }

    func replacePortForwards(project: String, stack: String, records: [PortForwardRecord]) throws {
        try execute("BEGIN IMMEDIATE;")
        do {
            try withStatement(
                "DELETE FROM port_forwards WHERE project = ? AND stack = ?;"
            ) { statement in
                try bind(project, at: 1, to: statement)
                try bind(stack, at: 2, to: statement)
                try stepDone(statement)
            }
            for record in records {
                try withStatement(
                    """
                    INSERT INTO port_forwards
                        (bind, host_port, guest_ip, guest_port, vm_id, source, project, stack, state, updated_at)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?);
                    """
                ) { statement in
                    try bind(record.bind, at: 1, to: statement)
                    try bind(Int64(record.hostPort), at: 2, to: statement)
                    try bind(record.guestIP, at: 3, to: statement)
                    try bind(Int64(record.guestPort), at: 4, to: statement)
                    try bind(record.vmID, at: 5, to: statement)
                    try bind(record.source, at: 6, to: statement)
                    try bind(record.project, at: 7, to: statement)
                    try bind(record.stack, at: 8, to: statement)
                    try bind(record.state, at: 9, to: statement)
                    try bind(record.updatedAt, at: 10, to: statement)
                    try stepDone(statement)
                }
            }
            try execute("COMMIT;")
        } catch {
            try? execute("ROLLBACK;")
            throw error
        }
    }

    func deletePortForwards(project: String, stack: String) throws {
        try withStatement(
            "DELETE FROM port_forwards WHERE project = ? AND stack = ?;"
        ) { statement in
            try bind(project, at: 1, to: statement)
            try bind(stack, at: 2, to: statement)
            try stepDone(statement)
        }
    }

    /// Deletes every port-forward row for a VM (orphan / force-purge cleanup).
    @discardableResult
    func deletePortForwards(vmID: String) throws -> Int {
        try withStatement("DELETE FROM port_forwards WHERE vm_id = ?;") { statement in
            try bind(vmID, at: 1, to: statement)
            try stepDone(statement)
            return Int(sqlite3_changes(handle))
        }
    }

    func portForwards(project: String? = nil, stack: String? = nil) throws -> [PortForwardRecord] {
        var sql =
            """
            SELECT bind, host_port, guest_ip, guest_port, vm_id, source, project, stack, state, updated_at
            FROM port_forwards
            """
        var filters: [String] = []
        if project != nil { filters.append("project = ?") }
        if stack != nil { filters.append("stack = ?") }
        if !filters.isEmpty {
            sql += " WHERE " + filters.joined(separator: " AND ")
        }
        sql += " ORDER BY host_port, bind;"
        return try withStatement(sql) { statement in
            var index: Int32 = 1
            if let project {
                try bind(project, at: index, to: statement)
                index += 1
            }
            if let stack {
                try bind(stack, at: index, to: statement)
            }
            var records: [PortForwardRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                records.append(
                    PortForwardRecord(
                        bind: text(statement, 0),
                        hostPort: UInt16(sqlite3_column_int64(statement, 1)),
                        guestIP: text(statement, 2),
                        guestPort: UInt16(sqlite3_column_int64(statement, 3)),
                        vmID: text(statement, 4),
                        source: text(statement, 5),
                        project: text(statement, 6),
                        stack: text(statement, 7),
                        state: text(statement, 8),
                        updatedAt: text(statement, 9)
                    )
                )
            }
            guard sqlite3_errcode(handle) == SQLITE_OK || sqlite3_errcode(handle) == SQLITE_DONE else {
                throw databaseError()
            }
            return records
        }
    }

    func edgeProjects() throws -> [EdgeProjectRecord] {
        try withStatement(
            """
            SELECT project, host_services_json, dns_records_json, ingress_json, oidc_json, updated_at
            FROM edge_projects ORDER BY project;
            """
        ) { statement in
            var records: [EdgeProjectRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                records.append(
                    EdgeProjectRecord(
                        project: text(statement, 0),
                        hostServices: try decodeJSON(text(statement, 1)),
                        dnsRecords: try decodeJSON(text(statement, 2)),
                        ingress: try optionalText(statement, 3).map { try decodeJSON($0) },
                        oidc: try optionalText(statement, 4).map { try decodeJSON($0) },
                        updatedAt: text(statement, 5)
                    )
                )
            }
            return records
        }
    }

    func setEdgeHostServices(project: String, hosts: JSONValue) throws {
        try upsertEdgeProject(project: project, column: "host_services_json", value: hosts)
    }

    func setEdgeDNSRecords(project: String, records: JSONValue) throws {
        try upsertEdgeProject(project: project, column: "dns_records_json", value: records)
    }

    func setEdgeIngress(project: String, value: JSONValue?) throws {
        try upsertEdgeProject(project: project, column: "ingress_json", value: value)
    }

    func setEdgeOIDC(project: String, value: JSONValue?) throws {
        try upsertEdgeProject(project: project, column: "oidc_json", value: value)
    }

    func nextEdgeGeneration() throws -> Int64 {
        try reconcileLock.withLock {
            try execute("BEGIN IMMEDIATE;")
            do {
                try execute("UPDATE edge_meta SET generation = generation + 1 WHERE singleton = 1;")
                let generation = try withStatement(
                    "SELECT generation FROM edge_meta WHERE singleton = 1;"
                ) { statement -> Int64 in
                    guard sqlite3_step(statement) == SQLITE_ROW else { throw databaseError() }
                    return sqlite3_column_int64(statement, 0)
                }
                try execute("COMMIT;")
                return generation
            } catch {
                try? execute("ROLLBACK;")
                throw error
            }
        }
    }

    private func upsertEdgeProject(project: String, column: String, value: JSONValue?) throws {
        guard ["host_services_json", "dns_records_json", "ingress_json", "oidc_json"]
            .contains(column)
        else {
            throw SupervisorError.database("invalid edge project column")
        }
        let now = ISO8601DateFormatter().string(from: Date())
        let encoded = try value.map(encodeJSON)
        let defaultHosts = try encodeJSON(.array([]))
        try withStatement(
            """
            INSERT OR IGNORE INTO edge_projects
                (project, host_services_json, updated_at)
            VALUES (?, ?, ?);
            """
        ) { statement in
            try bind(project, at: 1, to: statement)
            try bind(defaultHosts, at: 2, to: statement)
            try bind(now, at: 3, to: statement)
            try stepDone(statement)
        }
        try withStatement(
            "UPDATE edge_projects SET \(column) = ?, updated_at = ? WHERE project = ?;"
        ) { statement in
            if let encoded { try bind(encoded, at: 1, to: statement) }
            else { sqlite3_bind_null(statement, 1) }
            try bind(now, at: 2, to: statement)
            try bind(project, at: 3, to: statement)
            try stepDone(statement)
        }
    }

    private func encodeJSON(_ value: JSONValue) throws -> String {
        let data = try JSONEncoder().encode(value)
        guard let text = String(data: data, encoding: .utf8) else {
            throw SupervisorError.database("cannot encode edge JSON")
        }
        return text
    }

    private func decodeJSON(_ value: String) throws -> JSONValue {
        guard let data = value.data(using: .utf8) else {
            throw SupervisorError.database("cannot decode edge JSON")
        }
        return try JSONDecoder().decode(JSONValue.self, from: data)
    }

    func stackState(stackID: String) throws -> StackStateRecord {
        try reconcileLock.withLock {
            StackStateRecord(
                resources: try resources(stackID: stackID),
                journal: try incompleteJournal(stackID: stackID),
                lease: try lease(stackID: stackID)
            )
        }
    }

    func beginApply(
        stackID: String,
        holder: String,
        desiredHash: String,
        payload: String,
        resume: Bool
    ) throws -> JournalRecord {
        try reconcileLock.withLock {
            try execute("BEGIN IMMEDIATE;")
            do {
                if let current = try incompleteJournal(stackID: stackID) {
                    if let currentLease = try lease(stackID: stackID),
                       leaseBlocks(currentLease, for: holder)
                    {
                        throw ReconcileDatabaseError.leaseHeld(currentLease)
                    }
                    guard resume else { throw ReconcileDatabaseError.incomplete(current) }
                    guard current.desiredHash == desiredHash else {
                        throw ReconcileDatabaseError.generationChanged
                    }
                    try upsertLease(stackID: stackID, holder: holder)
                    try updateJournal(
                        id: current.id,
                        step: current.step,
                        status: "running",
                        error: nil
                    )
                    try execute("COMMIT;")
                    return try journal(id: current.id)!
                }
                if let currentLease = try lease(stackID: stackID),
                   leaseBlocks(currentLease, for: holder)
                {
                    throw ReconcileDatabaseError.leaseHeld(currentLease)
                }
                try upsertLease(stackID: stackID, holder: holder)
                let generation = try nextGeneration(stackID: stackID)
                let id = UUID().uuidString.lowercased()
                let now = timestamp()
                try withStatement(
                    """
                    INSERT INTO journal
                        (id, stack_id, generation, step, status, payload, error, created_at, updated_at)
                    VALUES (?, ?, ?, 'validate', 'running', ?, NULL, ?, ?);
                    """
                ) { statement in
                    try bind(id, at: 1, to: statement)
                    try bind(stackID, at: 2, to: statement)
                    sqlite3_bind_int64(statement, 3, sqlite3_int64(generation))
                    try bind(payload, at: 4, to: statement)
                    try bind(now, at: 5, to: statement)
                    try bind(now, at: 6, to: statement)
                    try stepDone(statement)
                }
                try execute("COMMIT;")
                return try journal(id: id)!
            } catch {
                try? execute("ROLLBACK;")
                throw error
            }
        }
    }

    func advanceApply(
        id: String,
        stackID: String,
        holder: String,
        step: String,
        status: String,
        error: String?
    ) throws -> JournalRecord {
        try reconcileLock.withLock {
            guard let current = try incompleteJournal(stackID: stackID), current.id == id else {
                throw ReconcileDatabaseError.noIncomplete
            }
            guard let currentLease = try lease(stackID: stackID),
                  currentLease.holder == holder,
                  currentLease.expiresAt > Date()
            else {
                throw ReconcileDatabaseError.leaseLost
            }
            try upsertLease(stackID: stackID, holder: holder)
            try updateJournal(id: id, step: step, status: status, error: error)
            return try journal(id: id)!
        }
    }

    func finishApply(
        id: String,
        stackID: String,
        holder: String,
        resourcesJSON: String
    ) throws {
        try reconcileLock.withLock {
            try execute("BEGIN IMMEDIATE;")
            do {
                guard let current = try incompleteJournal(stackID: stackID), current.id == id else {
                    throw ReconcileDatabaseError.noIncomplete
                }
                guard let currentLease = try lease(stackID: stackID),
                      currentLease.holder == holder
                else {
                    throw ReconcileDatabaseError.leaseLost
                }
                let resources = try JSONDecoder().decode(
                    [StackResourceRecord].self,
                    from: Data(resourcesJSON.utf8)
                )
                try withStatement(
                    "DELETE FROM resources WHERE json_extract(labels_json, '$.stack_id') = ?;"
                ) { statement in
                    try bind(stackID, at: 1, to: statement)
                    try stepDone(statement)
                }
                for resource in resources {
                    var labels = resource.labels
                    labels["stack_id"] = stackID
                    try withStatement(
                        """
                        INSERT INTO resources (id, kind, name, labels_json, state, updated_at)
                        VALUES (?, ?, ?, ?, ?, ?);
                        """
                    ) { statement in
                        try bind("\(stackID):\(resource.kind):\(resource.name)", at: 1, to: statement)
                        try bind(resource.kind, at: 2, to: statement)
                        try bind(resource.name, at: 3, to: statement)
                        try bind(try labelsJSON(labels), at: 4, to: statement)
                        try bind(resource.state, at: 5, to: statement)
                        try bind(timestamp(), at: 6, to: statement)
                        try stepDone(statement)
                    }
                }
                try updateJournal(id: id, step: "done", status: "done", error: nil)
                try deleteLease(stackID: stackID, holder: holder)
                try execute("COMMIT;")
            } catch {
                try? execute("ROLLBACK;")
                throw error
            }
        }
    }

    func abortApply(stackID: String, holder: String) throws -> JournalRecord {
        try reconcileLock.withLock {
            guard let current = try incompleteJournal(stackID: stackID) else {
                throw ReconcileDatabaseError.noIncomplete
            }
            if let currentLease = try lease(stackID: stackID),
               leaseBlocks(currentLease, for: holder)
            {
                throw ReconcileDatabaseError.leaseHeld(currentLease)
            }
            try updateJournal(id: current.id, step: current.step, status: "aborted", error: nil)
            try deleteLease(stackID: stackID, holder: nil)
            return try journal(id: current.id)!
        }
    }

    private func resources(stackID: String) throws -> [StackResourceRecord] {
        try withStatement(
            """
            SELECT kind, name, labels_json, state
            FROM resources
            WHERE json_extract(labels_json, '$.stack_id') = ?
            ORDER BY kind, name;
            """
        ) { statement in
            try bind(stackID, at: 1, to: statement)
            var result: [StackResourceRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                result.append(
                    StackResourceRecord(
                        kind: text(statement, 0),
                        name: text(statement, 1),
                        labels: try labels(from: text(statement, 2)),
                        state: text(statement, 3)
                    )
                )
            }
            return result
        }
    }

    private func incompleteJournal(stackID: String) throws -> JournalRecord? {
        try withStatement(
            """
            SELECT id, stack_id, generation, step, status, payload, error, created_at, updated_at
            FROM journal
            WHERE stack_id = ? AND status IN ('pending', 'running', 'failed')
            ORDER BY generation DESC LIMIT 1;
            """
        ) { statement in
            try bind(stackID, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { return nil }
            return journalRecord(statement)
        }
    }

    private func journal(id: String) throws -> JournalRecord? {
        try withStatement(
            """
            SELECT id, stack_id, generation, step, status, payload, error, created_at, updated_at
            FROM journal WHERE id = ?;
            """
        ) { statement in
            try bind(id, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { return nil }
            return journalRecord(statement)
        }
    }

    private func journalRecord(_ statement: OpaquePointer) -> JournalRecord {
        JournalRecord(
            id: text(statement, 0),
            stackID: text(statement, 1),
            generation: Int(sqlite3_column_int64(statement, 2)),
            step: text(statement, 3),
            status: text(statement, 4),
            payload: text(statement, 5),
            error: optionalText(statement, 6),
            createdAt: text(statement, 7),
            updatedAt: text(statement, 8)
        )
    }

    private func lease(stackID: String) throws -> LeaseRecord? {
        try withStatement(
            "SELECT holder, expires_at FROM locks WHERE stack_id = ?;"
        ) { statement in
            try bind(stackID, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { return nil }
            let raw = text(statement, 1)
            return LeaseRecord(
                holder: text(statement, 0),
                expiresAt: ISO8601DateFormatter().date(from: raw) ?? .distantPast,
                expiresAtText: raw
            )
        }
    }

    private func upsertLease(stackID: String, holder: String) throws {
        // Long steps (image pull/bake) can exceed a short TTL without checkpoints.
        let expires = ISO8601DateFormatter().string(from: Date().addingTimeInterval(300))
        try withStatement(
            """
            INSERT INTO locks (stack_id, holder, expires_at) VALUES (?, ?, ?)
            ON CONFLICT(stack_id) DO UPDATE SET holder = excluded.holder, expires_at = excluded.expires_at;
            """
        ) { statement in
            try bind(stackID, at: 1, to: statement)
            try bind(holder, at: 2, to: statement)
            try bind(expires, at: 3, to: statement)
            try stepDone(statement)
        }
    }

    /// A foreign lease blocks only while it is unexpired and its holder PID is alive.
    /// Expired leases are stealable even if a hung process is still around (ADR 0003).
    private func leaseBlocks(_ lease: LeaseRecord, for holder: String) -> Bool {
        guard lease.holder != holder else { return false }
        guard lease.expiresAt > Date() else { return false }
        return holderIsAlive(lease.holder)
    }

    private func deleteLease(stackID: String, holder: String?) throws {
        let sql = holder == nil
            ? "DELETE FROM locks WHERE stack_id = ?;"
            : "DELETE FROM locks WHERE stack_id = ? AND holder = ?;"
        try withStatement(sql) { statement in
            try bind(stackID, at: 1, to: statement)
            if let holder { try bind(holder, at: 2, to: statement) }
            try stepDone(statement)
        }
    }

    private func updateJournal(
        id: String,
        step: String,
        status: String,
        error: String?
    ) throws {
        try withStatement(
            "UPDATE journal SET step = ?, status = ?, error = ?, updated_at = ? WHERE id = ?;"
        ) { statement in
            try bind(step, at: 1, to: statement)
            try bind(status, at: 2, to: statement)
            try bind(error, at: 3, to: statement)
            try bind(timestamp(), at: 4, to: statement)
            try bind(id, at: 5, to: statement)
            try stepDone(statement)
        }
    }

    private func nextGeneration(stackID: String) throws -> Int {
        try withStatement(
            "SELECT COALESCE(MAX(generation), 0) + 1 FROM journal WHERE stack_id = ?;"
        ) { statement in
            try bind(stackID, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { throw databaseError() }
            return Int(sqlite3_column_int64(statement, 0))
        }
    }

    private func timestamp() -> String {
        ISO8601DateFormatter().string(from: Date())
    }

    private func holderIsAlive(_ holder: String) -> Bool {
        guard let rawPID = holder.split(separator: ":").last,
              let pid = Int32(rawPID),
              pid > 0
        else {
            return true
        }
        return Darwin.kill(pid, 0) == 0 || errno == EPERM
    }

    func listStacks() throws -> [StackRegistryRecord] {
        try withStatement(
            "SELECT id, path, name, opened_at FROM stacks ORDER BY opened_at DESC;"
        ) { statement in
            var rows: [StackRegistryRecord] = []
            while sqlite3_step(statement) == SQLITE_ROW {
                rows.append(
                    StackRegistryRecord(
                        id: text(statement, 0),
                        path: text(statement, 1),
                        name: text(statement, 2),
                        openedAt: text(statement, 3)
                    )
                )
            }
            return rows
        }
    }

    func getStack(id: String) throws -> StackRegistryRecord? {
        try withStatement(
            "SELECT id, path, name, opened_at FROM stacks WHERE id = ? LIMIT 1;"
        ) { statement in
            try bind(id, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { return nil }
            return StackRegistryRecord(
                id: text(statement, 0),
                path: text(statement, 1),
                name: text(statement, 2),
                openedAt: text(statement, 3)
            )
        }
    }

    func getStackByPath(_ path: String) throws -> StackRegistryRecord? {
        try withStatement(
            "SELECT id, path, name, opened_at FROM stacks WHERE path = ? LIMIT 1;"
        ) { statement in
            try bind(path, at: 1, to: statement)
            guard sqlite3_step(statement) == SQLITE_ROW else { return nil }
            return StackRegistryRecord(
                id: text(statement, 0),
                path: text(statement, 1),
                name: text(statement, 2),
                openedAt: text(statement, 3)
            )
        }
    }

    func upsertStack(_ record: StackRegistryRecord) throws {
        try withStatement(
            """
            INSERT INTO stacks (id, path, name, opened_at)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                path = excluded.path,
                name = excluded.name,
                opened_at = excluded.opened_at;
            """
        ) { statement in
            try bind(record.id, at: 1, to: statement)
            try bind(record.path, at: 2, to: statement)
            try bind(record.name, at: 3, to: statement)
            try bind(record.openedAt, at: 4, to: statement)
            try stepDone(statement)
        }
    }

    func deleteStack(id: String) throws {
        try withStatement("DELETE FROM stacks WHERE id = ?;") { statement in
            try bind(id, at: 1, to: statement)
            try stepDone(statement)
        }
    }

    private func withStatement<T>(_ sql: String, body: (OpaquePointer) throws -> T) throws -> T {
        var statement: OpaquePointer?
        guard sqlite3_prepare_v2(handle, sql, -1, &statement, nil) == SQLITE_OK,
              let statement
        else {
            throw databaseError()
        }
        defer { sqlite3_finalize(statement) }
        return try body(statement)
    }

    private func bind(_ value: String?, at index: Int32, to statement: OpaquePointer) throws {
        let result: Int32
        if let value {
            result = sqlite3_bind_text(statement, index, value, -1, SQLITE_TRANSIENT)
        } else {
            result = sqlite3_bind_null(statement, index)
        }
        guard result == SQLITE_OK else { throw databaseError() }
    }

    private func bind(_ value: Int64, at index: Int32, to statement: OpaquePointer) throws {
        guard sqlite3_bind_int64(statement, index, value) == SQLITE_OK else {
            throw databaseError()
        }
    }

    private func stepDone(_ statement: OpaquePointer) throws {
        guard sqlite3_step(statement) == SQLITE_DONE else { throw databaseError() }
    }

    private func text(_ statement: OpaquePointer, _ index: Int32) -> String {
        String(cString: sqlite3_column_text(statement, index))
    }

    private func optionalText(_ statement: OpaquePointer, _ index: Int32) -> String? {
        guard sqlite3_column_type(statement, index) != SQLITE_NULL else { return nil }
        return text(statement, index)
    }

    private func labelsJSON(_ labels: [String: String]) throws -> String {
        let data = try JSONEncoder().encode(labels)
        guard let value = String(data: data, encoding: .utf8) else {
            throw SupervisorError.database("labels are not UTF-8")
        }
        return value
    }

    private func labels(from value: String) throws -> [String: String] {
        guard let data = value.data(using: .utf8) else {
            throw SupervisorError.database("stored labels are not UTF-8")
        }
        return try JSONDecoder().decode([String: String].self, from: data)
    }

    private func databaseError() -> SupervisorError {
        SupervisorError.database(String(cString: sqlite3_errmsg(handle)))
    }
}

enum ReconcileDatabaseError: Error {
    case incomplete(JournalRecord)
    case leaseHeld(LeaseRecord)
    case generationChanged
    case noIncomplete
    case leaseLost
}

struct EdgeProjectRecord: Sendable {
    let project: String
    let hostServices: JSONValue
    let dnsRecords: JSONValue
    let ingress: JSONValue?
    let oidc: JSONValue?
    let updatedAt: String
}

struct StackStateRecord {
    let resources: [StackResourceRecord]
    let journal: JournalRecord?
    let lease: LeaseRecord?
}

struct StackResourceRecord: Codable {
    let kind: String
    let name: String
    var labels: [String: String]
    let state: String
}

struct JournalRecord {
    let id: String
    let stackID: String
    let generation: Int
    let step: String
    let status: String
    let payload: String
    let error: String?
    let createdAt: String
    let updatedAt: String

    var desiredHash: String? {
        payloadValue("desired_hash") as? String
    }

    var operationMode: String? {
        payloadValue("mode") as? String
    }

    private func payloadValue(_ key: String) -> Any? {
        guard let data = payload.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return value[key]
    }
}

struct LeaseRecord {
    let holder: String
    let expiresAt: Date
    let expiresAtText: String
}

private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
