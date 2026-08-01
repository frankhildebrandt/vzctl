import Foundation
import VzDaemonKit

struct StackRegistryRecord: Sendable {
    var id: String
    var path: String
    var name: String
    var openedAt: String

    var json: JSONValue {
        .object([
            "id": .string(id),
            "path": .string(path),
            "name": .string(name),
            "openedAt": .string(openedAt),
        ])
    }
}

enum RestStackStore {
    static func configURL(for path: String) -> URL {
        URL(fileURLWithPath: path).appendingPathComponent("hypernetwork.config.yaml")
    }

    static func diagramURL(for path: String) -> URL {
        URL(fileURLWithPath: path)
            .appendingPathComponent(".vzctl", isDirectory: true)
            .appendingPathComponent("diagram.json")
    }

    static func readText(at url: URL) throws -> String {
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw StackStoreError.notFound(url.path)
        }
        return try String(contentsOf: url, encoding: .utf8)
    }

    static func writeText(_ content: String, at url: URL) throws {
        let dir = url.deletingLastPathComponent()
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try content.write(to: url, atomically: true, encoding: .utf8)
    }

    static func stackId(from path: String, explicit: String?) -> String {
        if let explicit, !explicit.isEmpty { return explicit }
        let name = URL(fileURLWithPath: path).lastPathComponent
        return name.isEmpty ? UUID().uuidString.lowercased() : name
    }
}

enum StackStoreError: Error, CustomStringConvertible {
    case notFound(String)

    var description: String {
        switch self {
        case let .notFound(path):
            return "not found: \(path)"
        }
    }
}
