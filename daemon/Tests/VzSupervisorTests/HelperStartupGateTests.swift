import Darwin
import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

@Test func restartWaitsForActualHelperLockReleaseWithoutRemovingIt() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vz-start-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("helper.lock")
    let fd = open(url.path, O_RDWR | O_CREAT, 0o600)
    #expect(fd >= 0)
    defer { close(fd) }
    #expect(flock(fd, LOCK_EX | LOCK_NB) == 0)
    try "42\n".write(to: url, atomically: false, encoding: .utf8)
    #expect(throws: (any Error).self) {
        try HelperStartupGate.waitForRelease(
            lockURL: url, previousProcessAlive: { false }, timeout: 0.03
        )
    }
    #expect(try String(contentsOf: url, encoding: .utf8) == "42\n")
    #expect(flock(fd, LOCK_UN) == 0)
    try HelperStartupGate.waitForRelease(
        lockURL: url, previousProcessAlive: { false }, timeout: 0.03
    )
    #expect(throws: (any Error).self) {
        try HelperStartupGate.waitForRelease(
            lockURL: url, previousProcessAlive: { true }, timeout: 0.03
        )
    }
}

@Test func helperStartDoesNotSucceedWhenChildExitsBeforeClaimingLock() throws {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/true")
    try process.run()
    defer { process.waitUntilExit() }
    #expect(throws: (any Error).self) {
        try HelperStartupGate.waitForClaim(
            process: process, lockURL: URL(fileURLWithPath: "/tmp/absent-\(UUID())"),
            timeout: 0.2
        )
    }
}

@Test func vmStartRPCFailsBeforeSpawningWhileStoppedHelperStillOwnsLock() throws {
    let state = URL(fileURLWithPath: "/tmp/vz-rpc-\(UUID().uuidString.prefix(8))")
    defer { try? FileManager.default.removeItem(at: state) }
    let server = try SupervisorServer(stateDirectory: state)
    defer { server.stop() }
    server.helperStartupTimeout = 0.03
    let vmID = "lab/router"
    let url = HelperStartupGate.lockURL(vmID: vmID, stateDirectory: state)
    try FileManager.default.createDirectory(
        at: url.deletingLastPathComponent(), withIntermediateDirectories: true
    )
    let fd = open(url.path, O_RDWR | O_CREAT, 0o600)
    #expect(fd >= 0)
    defer { flock(fd, LOCK_UN); close(fd) }
    #expect(flock(fd, LOCK_EX | LOCK_NB) == 0)
    try "\(getpid())\n".write(to: url, atomically: false, encoding: .utf8)
    server.helpers[vmID] = HelperRecord(
        vmID: vmID, state: "stopped", pid: Int(getpid()), bundle: "/tmp/bundle", lastError: nil
    )
    let response = server.dispatchRPC(method: "vm.start", params: .object([
        "vm_id": .string(vmID), "bundle": .string("/tmp/bundle"),
    ]))
    #expect(response.error?.message.contains("previous helper") == true)
    #expect(response.result == nil)
    #expect(server.helperProcesses[vmID] == nil)
    #expect(server.helpers[vmID]?.state == "stopped")
    #expect(try HelperStartupGate.isClaimed(by: getpid(), lockURL: url))
}

@Test func helperStartRequiresTheNewChildToHoldItsLock() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vz-claim-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("helper.lock")
    let process = Process()
    let input = Pipe()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["python3", "-c", """
import fcntl,os,sys
with open(sys.argv[1],'w') as f:
 fcntl.flock(f,fcntl.LOCK_EX)
 f.write(str(os.getpid())+'\\n');f.flush()
 sys.stdin.read()
""", url.path]
    process.standardInput = input
    process.standardOutput = FileHandle.nullDevice
    try process.run()
    defer { try? input.fileHandleForWriting.close(); process.waitUntilExit() }
    try HelperStartupGate.waitForClaim(process: process, lockURL: url, timeout: 3)
    #expect(try HelperStartupGate.isClaimed(by: process.processIdentifier, lockURL: url))
    #expect(try !HelperStartupGate.isClaimed(by: getpid(), lockURL: url))
    #expect(process.isRunning)
}

@Test func claimObserverNeverLocksAStalePIDFileWhileChildIsStarting() throws {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("vz-stale-\(UUID())")
    defer { try? FileManager.default.removeItem(at: url) }
    try "42\n".write(to: url, atomically: false, encoding: .utf8)
    #expect(try !HelperStartupGate.isClaimed(
        by: getpid(), lockURL: url, beforeLockProbe: {
            Issue.record("observer must not acquire the stale file lock before the child")
        }
    ))
    try "\(getpid())\n".write(to: url, atomically: false, encoding: .utf8)
    // PID text alone is not ownership: a stale same-PID file is still free.
    #expect(try !HelperStartupGate.isClaimed(by: getpid(), lockURL: url))
}

@Test func oldHelperReportCannotOverwriteNewHelperGeneration() throws {
    let state = URL(fileURLWithPath: "/tmp/vz-gen-\(UUID().uuidString.prefix(8))")
    defer { try? FileManager.default.removeItem(at: state) }
    let server = try SupervisorServer(stateDirectory: state)
    defer { server.stop() }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/cat")
    let input = Pipe()
    process.standardInput = input
    process.standardOutput = FileHandle.nullDevice
    try process.run()
    defer { try? input.fileHandleForWriting.close(); process.waitUntilExit() }
    let vmID = "lab/router"
    server.helperProcesses[vmID] = process
    server.helpers[vmID] = HelperRecord(
        vmID: vmID, state: "starting", pid: Int(process.processIdentifier),
        bundle: "/tmp/new-bundle", lastError: nil
    )
    _ = server.dispatchRPC(method: "helper.state", params: .object([
        "vm_id": .string(vmID), "state": .string("stopped"), "pid": .number(42),
        "bundle": .string("/tmp/old-bundle"),
    ]))
    #expect(server.helpers[vmID]?.pid == Int(process.processIdentifier))
    #expect(server.helpers[vmID]?.state == "starting")
    let oldProcess = Process()
    oldProcess.executableURL = URL(fileURLWithPath: "/usr/bin/true")
    try oldProcess.run()
    oldProcess.waitUntilExit()
    server.expectedHelperStops.insert(vmID)
    server.helperTerminated(oldProcess, vmID: vmID, bundle: "/tmp/old-bundle")
    #expect(server.helperProcesses[vmID] === process)
    #expect(server.helpers[vmID]?.state == "starting")
    #expect(server.expectedHelperStops.contains(vmID))
    server.helpers[vmID] = HelperRecord(
        vmID: vmID, state: "running", pid: Int(process.processIdentifier),
        bundle: "/tmp/new-bundle", lastError: nil
    )
    #expect(try server.confirmedHelperStartRecord(process, vmID: vmID).state == "running")
    server.helpers[vmID] = HelperRecord(
        vmID: vmID, state: "failed", pid: Int(process.processIdentifier),
        bundle: "/tmp/new-bundle", lastError: "fixture startup failure"
    )
    #expect(throws: (any Error).self) {
        try server.confirmedHelperStartRecord(process, vmID: vmID)
    }
}
