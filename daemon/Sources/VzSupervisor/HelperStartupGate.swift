import Darwin
import Foundation
import VzDaemonKit

enum HelperStartupGate {
    static func lockURL(vmID: String, stateDirectory: URL) -> URL {
        stateDirectory.appendingPathComponent("helpers")
            .appendingPathComponent("\(StateFileName.component(vmID)).lock")
    }

    static func isClaimed(
        by pid: pid_t, lockURL: URL, beforeLockProbe: (() -> Void)? = nil
    ) throws -> Bool {
        let lock = try owner(of: lockURL, expectedPID: pid, beforeLockProbe: beforeLockProbe)
        return lock.held && lock.pid == pid
    }

    static func waitForRelease(
        lockURL: URL, previousProcessAlive: () -> Bool,
        timeout: TimeInterval = 5
    ) throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(timeout))
        while true {
            let lock = try owner(of: lockURL)
            if !lock.held && !previousProcessAlive() { return }
            guard ContinuousClock.now < deadline else {
                throw StartupError("previous helper has not released its process/lock before timeout")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }

    static func waitForClaim(
        process: Process, lockURL: URL, timeout: TimeInterval = 5
    ) throws {
        let deadline = ContinuousClock.now.advanced(by: .seconds(timeout))
        while true {
            guard process.isRunning else {
                throw StartupError("new helper exited before claiming its VM lock")
            }
            if try isClaimed(by: process.processIdentifier, lockURL: lockURL) { return }
            guard ContinuousClock.now < deadline else {
                throw StartupError("new helper did not claim its VM lock before timeout")
            }
            Thread.sleep(forTimeInterval: 0.01)
        }
    }

    private static func owner(
        of url: URL, expectedPID: pid_t? = nil, beforeLockProbe: (() -> Void)? = nil
    ) throws -> (held: Bool, pid: pid_t?) {
        let fd = open(url.path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
        if fd < 0 {
            if errno == ENOENT { return (false, nil) }
            throw StartupError("cannot inspect helper lock (errno \(errno))")
        }
        defer { close(fd) }
        func readPID() throws -> pid_t? {
            var bytes = [UInt8](repeating: 0, count: 64)
            let count = pread(fd, &bytes, bytes.count, 0)
            guard count >= 0 else {
                throw StartupError("cannot read helper lock owner (errno \(errno))")
            }
            return pid_t(String(decoding: bytes.prefix(count), as: UTF8.self)
                .trimmingCharacters(in: .whitespacesAndNewlines))
        }
        // The child uses LOCK_NB. Probing a still-free stale lock ourselves
        // could make its startup fail. The child writes its PID only AFTER
        // acquiring the lock, so never probe until that exact PID is visible.
        if let expectedPID, try readPID() != expectedPID { return (false, nil) }
        beforeLockProbe?()
        if flock(fd, LOCK_EX | LOCK_NB) == 0 {
            _ = flock(fd, LOCK_UN)
            return (false, nil)
        }
        guard errno == EWOULDBLOCK else {
            throw StartupError("cannot inspect helper lock ownership (errno \(errno))")
        }
        return (true, try readPID())
    }

    struct StartupError: Error, CustomStringConvertible {
        let description: String
        init(_ description: String) { self.description = description }
    }
}
