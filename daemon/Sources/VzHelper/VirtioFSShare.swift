import Foundation
@preconcurrency import Virtualization

struct VirtioFSMountSpec: Equatable, Sendable {
    let name: String
    let sourceURL: URL
    let target: String
    let readOnly: Bool
}

enum VirtioFSShare {
    static let deviceTag = "vzctl"
    static let guestMountRoot = "/mnt/vzctl"

    static func loadManifestMounts(bundleURL: URL) throws -> [VirtioFSMountSpec] {
        let manifestURL = bundleURL.appendingPathComponent("vm.json")
        guard FileManager.default.fileExists(atPath: manifestURL.path) else {
            return []
        }
        let data = try Data(contentsOf: manifestURL)
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return []
        }
        guard let mounts = root["mounts"] as? [[String: Any]] else {
            return []
        }
        return try mounts.map { item in
            guard
                let name = item["name"] as? String, !name.isEmpty,
                let source = item["source"] as? String, !source.isEmpty,
                let target = item["target"] as? String, !target.isEmpty
            else {
                throw HelperError.invalid("invalid mounts[] entry in \(manifestURL.path)")
            }
            let readOnly = (item["read_only"] as? Bool) ?? false
            try validateName(name)
            return VirtioFSMountSpec(
                name: name,
                sourceURL: URL(fileURLWithPath: source, isDirectory: true).standardizedFileURL,
                target: target,
                readOnly: readOnly
            )
        }
    }

    static func validateName(_ name: String) throws {
        if name == deviceTag {
            throw HelperError.invalid("mount name \(deviceTag) is reserved")
        }
        guard (1...36).contains(name.count) else {
            throw HelperError.invalid("mount name must be 1-36 characters")
        }
        for (index, scalar) in name.unicodeScalars.enumerated() {
            let ok = CharacterSet.alphanumerics.contains(scalar)
                || (index > 0 && (scalar == "-" || scalar == "_"))
            guard ok else {
                throw HelperError.invalid(
                    "mount name must match [A-Za-z0-9][A-Za-z0-9_-]*"
                )
            }
        }
    }

    static func ensureHostDirectories(_ mounts: [VirtioFSMountSpec]) throws {
        for mount in mounts {
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(
                atPath: mount.sourceURL.path,
                isDirectory: &isDirectory
            ), isDirectory.boolValue else {
                throw HelperError.invalid(
                    "mount source is not a directory: \(mount.sourceURL.path)"
                )
            }
        }
    }

    static func placeholderDirectory(bundleURL: URL) throws -> URL {
        let url = bundleURL.appendingPathComponent("virtiofs-empty", isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    static func makeShare(
        mounts: [VirtioFSMountSpec],
        bundleURL: URL
    ) throws -> VZDirectoryShare {
        try ensureHostDirectories(mounts)
        var directories: [String: VZSharedDirectory] = [:]
        if mounts.isEmpty {
            let placeholder = try placeholderDirectory(bundleURL: bundleURL)
            directories["_empty"] = VZSharedDirectory(url: placeholder, readOnly: true)
        } else {
            for mount in mounts {
                directories[mount.name] = VZSharedDirectory(
                    url: mount.sourceURL,
                    readOnly: mount.readOnly
                )
            }
        }
        return VZMultipleDirectoryShare(directories: directories)
    }

    static func makeDeviceConfiguration(
        mounts: [VirtioFSMountSpec],
        bundleURL: URL
    ) throws -> VZVirtioFileSystemDeviceConfiguration {
        let device = VZVirtioFileSystemDeviceConfiguration(tag: deviceTag)
        device.share = try makeShare(mounts: mounts, bundleURL: bundleURL)
        return device
    }
}
