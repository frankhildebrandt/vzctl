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
    #expect(options.cpuCount == 2)
    #expect(options.memorySize == 1024 * 1024 * 1024)
}

@Test func helperReadsResourcesFromManifest() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-resources-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    FileManager.default.createFile(
        atPath: directory.appendingPathComponent("disk.raw").path,
        contents: Data()
    )
    try """
    {
      "identity": {
        "nics": [{"index": 0, "mac": "02:12:34:56:78:9a", "address": "dhcp"}]
      },
      "resources": {
        "cpus": 4,
        "memory_mib": 2048
      }
    }
    """.write(
        to: directory.appendingPathComponent("vm.json"),
        atomically: true,
        encoding: .utf8
    )

    let parsed = try HelperArguments.manifestResources(bundleURL: directory)
    #expect(parsed?.cpuCount == 4)
    #expect(parsed?.memorySize == 2048 * 1024 * 1024)

    let command = try HelperArguments.parse(
        ["run", "--vm-id", "web", "--bundle", directory.path, "--mock"],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.cpuCount == 4)
    #expect(options.memorySize == 2048 * 1024 * 1024)
}

@Test func helperAcceptsExplicitCpuAndMemoryOverrides() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-resource-flags-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    FileManager.default.createFile(
        atPath: directory.appendingPathComponent("disk.raw").path,
        contents: Data()
    )
    try """
    {
      "identity": {
        "nics": [{"index": 0, "mac": "02:12:34:56:78:9a", "address": "dhcp"}]
      },
      "resources": { "cpus": 8, "memory_mib": 4096 }
    }
    """.write(
        to: directory.appendingPathComponent("vm.json"),
        atomically: true,
        encoding: .utf8
    )

    let command = try HelperArguments.parse(
        [
            "run", "--vm-id", "web", "--bundle", directory.path,
            "--cpus", "3", "--memory-mib", "1536", "--mock",
        ],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.cpuCount == 3)
    #expect(options.memorySize == 1536 * 1024 * 1024)
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

@Test func helperReadsMountsFromManifest() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-helper-mounts-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let share = directory.appendingPathComponent("share", isDirectory: true)
    try FileManager.default.createDirectory(at: share, withIntermediateDirectories: true)
    FileManager.default.createFile(
        atPath: directory.appendingPathComponent("disk.raw").path,
        contents: Data()
    )
    try """
    {
      "identity": {
        "nics": [{"index": 0, "mac": "02:12:34:56:78:9a", "address": "dhcp"}]
      },
      "mounts": [
        {
          "name": "web-src",
          "source": "\(share.path)",
          "target": "/srv/app",
          "read_only": false
        }
      ]
    }
    """.write(
        to: directory.appendingPathComponent("vm.json"),
        atomically: true,
        encoding: .utf8
    )

    let mounts = try VirtioFSShare.loadManifestMounts(bundleURL: directory)
    #expect(mounts.count == 1)
    #expect(mounts[0].name == "web-src")
    #expect(mounts[0].target == "/srv/app")
    #expect(mounts[0].sourceURL.path == share.standardizedFileURL.path)

    let command = try HelperArguments.parse(
        ["run", "--vm-id", "web", "--bundle", directory.path, "--mock"],
        environment: ["VZCTL_STATE_DIR": directory.path]
    )
    guard case let .run(options) = command else {
        Issue.record("expected run command")
        return
    }
    #expect(options.mounts.count == 1)
    #expect(options.mounts[0].name == "web-src")
}

@Test func virtiofsRejectsReservedDeviceTag() {
    #expect(throws: (any Error).self) {
        try VirtioFSShare.validateName("vzctl")
    }
}

