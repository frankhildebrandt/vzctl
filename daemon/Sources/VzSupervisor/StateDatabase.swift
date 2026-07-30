import Darwin
import Foundation
import SQLite3
import VzDaemonKit

final class StateDatabase {
    private var handle: OpaquePointer?

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
                CREATE TABLE IF NOT EXISTS networks (
                    name TEXT PRIMARY KEY,
                    cidr TEXT NOT NULL UNIQUE,
                    mode TEXT NOT NULL CHECK (mode = 'shared'),
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
                """
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
                (name, cidr, mode, labels_json, project, stack, runtime_state, last_error, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?);
            """
        ) { statement in
            try bind(record.name, at: 1, to: statement)
            try bind(record.cidr, at: 2, to: statement)
            try bind(record.mode, at: 3, to: statement)
            try bind(try labelsJSON(record.labels), at: 4, to: statement)
            try bind(record.project, at: 5, to: statement)
            try bind(record.stack, at: 6, to: statement)
            try bind(record.runtimeState, at: 7, to: statement)
            try bind(record.lastError, at: 8, to: statement)
            try bind(record.updatedAt, at: 9, to: statement)
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
            SELECT name, cidr, mode, labels_json, project, stack,
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
                        labels: try labels(from: text(statement, 3)),
                        project: optionalText(statement, 4),
                        stack: optionalText(statement, 5),
                        runtimeState: text(statement, 6),
                        lastError: optionalText(statement, 7),
                        updatedAt: text(statement, 8)
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

private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
