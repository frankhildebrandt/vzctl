import Foundation
import SQLite3
import Testing
@testable import VzDaemonKit
@testable import VzSupervisor

private let legacyNetworkSchema = """
CREATE TABLE networks (
 name TEXT PRIMARY KEY, cidr TEXT NOT NULL UNIQUE, mode TEXT NOT NULL,
 labels_json TEXT NOT NULL DEFAULT '{}', project TEXT, stack TEXT,
 runtime_state TEXT NOT NULL DEFAULT 'active', last_error TEXT,
 updated_at TEXT NOT NULL DEFAULT 'legacy'
);
CREATE TABLE network_attachments (
 vm_id TEXT NOT NULL, network_name TEXT NOT NULL, ip TEXT NOT NULL,
 labels_json TEXT NOT NULL DEFAULT '{}', project TEXT, stack TEXT,
 updated_at TEXT NOT NULL DEFAULT 'legacy',
 PRIMARY KEY(vm_id,network_name), UNIQUE(network_name,ip),
 FOREIGN KEY(network_name) REFERENCES networks(name)
);
"""

private func identitySQL(_ path: String, _ sql: String) throws -> [String] {
    var db: OpaquePointer?
    guard sqlite3_open(path, &db) == SQLITE_OK else { throw NSError(domain: "SQLite", code: 1) }
    defer { sqlite3_close(db) }
    var error: UnsafeMutablePointer<CChar>?
    guard sqlite3_exec(db, sql, nil, nil, &error) == SQLITE_OK else {
        let message = error.map { String(cString: $0) } ?? "SQLite error"
        sqlite3_free(error)
        throw NSError(domain: message, code: 2)
    }
    var statement: OpaquePointer?
    guard sqlite3_prepare_v2(db, "SELECT COALESCE(logical_name,'NULL') FROM networks ORDER BY name", -1, &statement, nil) == SQLITE_OK else { return [] }
    defer { sqlite3_finalize(statement) }
    var values: [String] = []
    while sqlite3_step(statement) == SQLITE_ROW {
        if let text = sqlite3_column_text(statement, 0) { values.append(String(cString: text)) }
    }
    return values
}

private final class IdentityRuntime: NetworkRuntimeBackend, NetworkRuntimeHandle, @unchecked Sendable {
    let releasesOnShutdown = true
    var acquired: [String] = []
    var serialized: [String] = []
    var released: [String] = []
    func reserve(_ network: NetworkRecord) throws -> any NetworkRuntimeHandle {
        acquired.append(network.id)
        return self
    }
    func serialize(name: String, handle: any NetworkRuntimeHandle) throws -> Data {
        serialized.append(name)
        return Data("blob".utf8)
    }
    func release(name: String, handle: any NetworkRuntimeHandle) throws { released.append(name) }
}

@Test func legacyNetworkMigrationPreservesRuntimeKeysAttachmentsAndOwners() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("identity-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appendingPathComponent("state.sqlite").path
    _ = try identitySQL(path, legacyNetworkSchema + """
    INSERT INTO networks(name,cidr,mode,project,stack) VALUES('lan','10.92.0.0/24','shared','iq-store','iq-store:iq-store');
    INSERT INTO network_attachments(vm_id,network_name,ip,project,stack)
    VALUES('shop/client','lan','10.92.0.10','shop','shop:shop');
    """)
    let database = try StateDatabase(path: path)
    let before = try database.networks()
    #expect(before.first?.id == "lan")
    #expect(before.first?.name == "lan")
    #expect(try database.attachments().first?.networkID == "lan")
    #expect(try database.attachments().first?.project == "shop")
    let backend = IdentityRuntime()
    let registry = try NetworkRegistry(database: database, backend: backend)
    defer { registry.shutdown() }
    #expect(backend.acquired == ["lan"])
    _ = try registry.create(name: "lan", cidr: "10.123.0.0/24", mode: "shared",
                            labels: [:], project: "controller-lab", stack: "controller-lab:controller-lab")
    #expect(backend.acquired == ["lan", "controller-lab/lan"])
    _ = try registry.serializedAttachments(for: "shop/client")
    #expect(backend.serialized == ["lan"])
    let selection = try registry.ensureVMNetwork(vmID: "shop/client", requestedNetwork: "lan", vmIsStopped: true)
    #expect(selection.network.id == "lan")
    #expect(selection.attachment.project == "shop")
    #expect(throws: NetworkRegistryError.self) { try registry.delete(name: "lan") }
    #expect(throws: NetworkRegistryError.self) { try registry.delete(name: "lan", project: "unknown") }
    try registry.delete(name: "lan", project: "controller-lab")
    #expect(backend.released == ["controller-lab/lan"])
    #expect(try registry.snapshot().networks.first?.id == "lan")
    #expect(try registry.snapshot().attachments.count == 1)
    // Reopening migrates idempotently without changing legacy IDs.
    let reopened = try StateDatabase(path: path)
    #expect(try reopened.networks().first?.id == "lan")
    #expect(try reopened.attachments().first?.project == "shop")
    try registry.detach(vmID: "shop/client", networkName: "lan", project: "shop", vmIsStopped: true)
    #expect(try registry.snapshot().attachments.isEmpty)
    #expect(backend.released == ["controller-lab/lan"])
}

@Test func failedNetworkIdentityMigrationRollsBackBackfill() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("identity-rollback-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let path = directory.appendingPathComponent("state.sqlite").path
    _ = try identitySQL(path, legacyNetworkSchema + """
    ALTER TABLE networks ADD COLUMN logical_name TEXT;
    INSERT INTO networks(name,cidr,mode,project,logical_name) VALUES
    ('a','10.10.0.0/24','shared','demo','lan'),
    ('b','10.11.0.0/24','shared','demo','lan'),
    ('c','10.12.0.0/24','shared','demo',NULL);
    """)
    #expect(throws: (any Error).self) { _ = try StateDatabase(path: path) }
    #expect(try identitySQL(path, "SELECT 1;") == ["lan", "lan", "NULL"])
}
