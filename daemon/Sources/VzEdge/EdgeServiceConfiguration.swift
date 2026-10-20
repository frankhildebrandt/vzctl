import CryptoKit
import Darwin
import Foundation

/// Immutable configs keep the child identity stable across reconcile generations,
/// while changed contents still produce new arguments and restart the child.
enum EdgeServiceConfiguration {
    static func write(
        _ value: String, directory: URL, prefix: String, suffix: String = ""
    ) throws -> String {
        let bytes = Data(value.utf8)
        let digest = SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        _ = chmod(directory.path, 0o700)
        let target = directory.appendingPathComponent("\(prefix).\(digest)\(suffix)")
        if (try? Data(contentsOf: target)) != bytes {
            try bytes.write(to: target, options: .atomic)
        }
        _ = chmod(target.path, 0o600)
        return target.path
    }
}
