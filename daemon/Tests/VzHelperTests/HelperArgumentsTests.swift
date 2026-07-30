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
    try """
    {
      "identity": {
        "nics": [{"index": 0, "mac": "02:12:34:56:78:9a", "address": "dhcp"}]
      }
    }
    """.write(
        to: directory.appendingPathComponent("vm.json"),
        atomically: true,
        encoding: .utf8
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
    #expect(options.cidataURL == nil)
    #expect(options.macAddress == "02:12:34:56:78:9a")
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

@Test func helperAcceptsExplicitMACAddressWithoutManifest() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-mac-flag-\(UUID().uuidString)", isDirectory: true)
    let command = try HelperArguments.parse(
        [
            "run", "--vm-id", "web", "--bundle", directory.path,
            "--mac-address", "02:aa:bb:cc:dd:ee", "--mock",
        ],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.macAddress == "02:aa:bb:cc:dd:ee")
}
