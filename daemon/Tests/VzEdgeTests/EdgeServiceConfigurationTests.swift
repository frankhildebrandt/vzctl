import Foundation
import Testing
@testable import VzEdge

@Test func edgeServiceConfigsAreStableImmutableAndPrivate() throws {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-config-test-\(UUID().uuidString)")
    defer { try? FileManager.default.removeItem(at: root) }
    let first = try EdgeServiceConfiguration.write("one", directory: root, prefix: "Caddyfile")
    let repeated = try EdgeServiceConfiguration.write("one", directory: root, prefix: "Caddyfile")
    let changed = try EdgeServiceConfiguration.write("two", directory: root, prefix: "Caddyfile")
    #expect(first == repeated)
    #expect(changed != first)
    #expect(try String(contentsOfFile: first, encoding: .utf8) == "one")
    #expect(try String(contentsOfFile: changed, encoding: .utf8) == "two")
    #expect(try FileManager.default.contentsOfDirectory(atPath: root.path).count == 2)
    let mode = try FileManager.default.attributesOfItem(atPath: first)[.posixPermissions] as? NSNumber
    #expect(mode?.intValue == 0o600)
}
