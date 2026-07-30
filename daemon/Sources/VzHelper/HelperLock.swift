import Darwin
import Foundation
import VzDaemonKit

enum StatePaths {
    static func stateDirectory(environment: [String: String]) throws -> URL {
        if let override = environment["VZCTL_STATE_DIR"] {
            return URL(fileURLWithPath: override, isDirectory: true).standardizedFileURL
        }
        return try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        ).appendingPathComponent("vzctl", isDirectory: true)
    }

    static func logsDirectory() throws -> URL {
        try FileManager.default.url(
            for: .libraryDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        .appendingPathComponent("Logs", isDirectory: true)
        .appendingPathComponent("vzctl", isDirectory: true)
    }
}

final class HelperLock {
    let url: URL
    private let descriptor: Int32

    init(vmID: String, stateDirectory: URL) throws {
        let directory = stateDirectory.appendingPathComponent("helpers", isDirectory: true)
        try FileManager.default.createDirectory(
            at: directory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        guard chmod(directory.path, 0o700) == 0 else {
            throw HelperError.system("chmod helper state directory", errno)
        }

        url = directory.appendingPathComponent("\(StateFileName.component(vmID)).lock")
        descriptor = open(url.path, O_RDWR | O_CREAT, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw HelperError.system("open lock", errno) }
        guard fchmod(descriptor, S_IRUSR | S_IWUSR) == 0 else {
            let code = errno
            close(descriptor)
            throw HelperError.system("chmod lock", code)
        }
        guard flock(descriptor, LOCK_EX | LOCK_NB) == 0 else {
            let holder = (try? String(contentsOf: url, encoding: .utf8))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            close(descriptor)
            throw HelperError.alreadyRunning(
                vmID: vmID,
                pid: holder.flatMap { $0.isEmpty ? nil : $0 } ?? "unknown"
            )
        }

        let payload = "\(getpid())\n"
        guard ftruncate(descriptor, 0) == 0,
              lseek(descriptor, 0, SEEK_SET) >= 0,
              payload.withCString({ Darwin.write(descriptor, $0, strlen($0)) }) >= 0
        else {
            let code = errno
            flock(descriptor, LOCK_UN)
            close(descriptor)
            throw HelperError.system("write lock", code)
        }
        fsync(descriptor)
    }

    deinit {
        flock(descriptor, LOCK_UN)
        close(descriptor)
    }
}
