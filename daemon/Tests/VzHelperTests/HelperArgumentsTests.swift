import Foundation
import Testing
@testable import VzHelper

@Test func helperUsesPreparedRootAndDataDiskBundleDefaults() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-disks-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    FileManager.default.createFile(
        atPath: directory.appendingPathComponent("disk.raw").path,
        contents: Data()
    )
    FileManager.default.createFile(
        atPath: directory.appendingPathComponent("dataDisk.raw").path,
        contents: Data()
    )

    let command = try HelperArguments.parse(
        ["run", "--vm-id", "web", "--bundle", directory.path, "--mock"],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.diskURL.lastPathComponent == "disk.raw")
    #expect(options.dataDiskURL?.lastPathComponent == "dataDisk.raw")
}

@Test func helperAcceptsExplicitDataDiskPath() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-data-flag-\(UUID().uuidString)", isDirectory: true)
    let dataDisk = directory.appendingPathComponent("custom-data.raw")
    let command = try HelperArguments.parse(
        [
            "run", "--vm-id", "web", "--bundle", directory.path,
            "--data-disk", dataDisk.path, "--mock",
        ],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.dataDiskURL == dataDisk.standardizedFileURL)
}
