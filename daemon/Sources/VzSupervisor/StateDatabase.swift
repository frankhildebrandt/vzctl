import Darwin
import Foundation
import SQLite3

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
}
