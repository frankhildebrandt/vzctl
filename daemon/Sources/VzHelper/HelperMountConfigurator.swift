import Foundation
import VzDaemonKit

enum HelperMountConfigurator {
    static func list(runtime: VirtualMachineRuntime) -> JSONValue {
        let mounts = runtime.currentMounts().map { mount in
            JSONValue.object([
                "name": .string(mount.name),
                "source": .string(mount.sourceURL.path),
                "target": .string(mount.target),
                "read_only": .bool(mount.readOnly),
            ])
        }
        return .object([
            "device_tag": .string(VirtioFSShare.deviceTag),
            "guest_root": .string(VirtioFSShare.guestMountRoot),
            "mounts": .array(mounts),
        ])
    }

    static func add(
        params: JSONValue?,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws -> JSONValue {
        let mount = try parseMount(params)
        var next = runtime.currentMounts()
        if let index = next.firstIndex(where: { $0.name == mount.name }) {
            if next[index].target != mount.target {
                throw HelperError.invalid(
                    "mount name \(mount.name) already maps to \(next[index].target)"
                )
            }
            next[index] = mount
        } else if next.contains(where: { $0.target == mount.target }) {
            throw HelperError.invalid("mount target \(mount.target) is already in use")
        } else {
            next.append(mount)
        }
        try await runtime.applyShare(next)
        try await guestMount(mount: mount, runtime: runtime, token: token)
        return list(runtime: runtime)
    }

    static func remove(
        params: JSONValue?,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws -> JSONValue {
        guard case let .object(values)? = params else {
            throw HelperError.invalid("mount.remove params must be an object")
        }
        let name: String?
        if case let .string(value)? = values["name"] {
            name = value
        } else {
            name = nil
        }
        let target: String?
        if case let .string(value)? = values["target"] {
            target = value
        } else {
            target = nil
        }
        guard name != nil || target != nil else {
            throw HelperError.invalid("mount.remove requires name or target")
        }
        var next = runtime.currentMounts()
        let removed = next.filter { mount in
            if let name, mount.name == name { return true }
            if let target, mount.target == target { return true }
            return false
        }
        guard !removed.isEmpty else {
            throw HelperError.invalid("mount not found")
        }
        next.removeAll { mount in
            removed.contains(where: { $0.name == mount.name })
        }
        for mount in removed {
            try await guestUnmount(mount: mount, runtime: runtime, token: token)
        }
        try await runtime.applyShare(next)
        return list(runtime: runtime)
    }

    static func applyManifestMounts(
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws {
        for mount in runtime.currentMounts() {
            try await guestMount(mount: mount, runtime: runtime, token: token)
        }
    }

    private static func parseMount(_ value: JSONValue?) throws -> VirtioFSMountSpec {
        guard case let .object(params)? = value,
              case let .string(name)? = params["name"],
              case let .string(source)? = params["source"],
              case let .string(target)? = params["target"]
        else {
            throw HelperError.invalid(
                "mount.add requires name, source, and target strings"
            )
        }
        let readOnly: Bool
        if case let .bool(value)? = params["read_only"] {
            readOnly = value
        } else {
            readOnly = false
        }
        try VirtioFSShare.validateName(name)
        guard target.hasPrefix("/"), target.count > 1 else {
            throw HelperError.invalid("mount target must be an absolute path")
        }
        return VirtioFSMountSpec(
            name: name,
            sourceURL: URL(fileURLWithPath: source, isDirectory: true).standardizedFileURL,
            target: target,
            readOnly: readOnly
        )
    }

    private static func guestMount(
        mount: VirtioFSMountSpec,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws {
        let client = try await runtime.connectToGuestAgent(timeout: 10)
        defer { client.close() }
        let hello = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        do {
            try client.fsMount(
                name: mount.name,
                target: mount.target,
                readOnly: mount.readOnly,
                timeout: 30
            )
            return
        } catch let GuestAgentError.remote(code, _, _) where code == "unsupported" {
            // Sealed images may still ship agents without fs.mount; CLI falls
            // back to exec+virtiofs-bind — match that here for boot apply.
            fputs(
                "fs.mount unsupported (agent=\(hello.version)); falling back to exec bind\n",
                stderr
            )
        }
        try execVirtiofsBind(
            client: client,
            action: "mount",
            name: mount.name,
            target: mount.target,
            readOnly: mount.readOnly
        )
    }

    private static func guestUnmount(
        mount: VirtioFSMountSpec,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws {
        let client = try await runtime.connectToGuestAgent(timeout: 10)
        defer { client.close() }
        let hello = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        do {
            try client.fsUnmount(name: mount.name, target: mount.target)
            return
        } catch let GuestAgentError.remote(code, _, _) where code == "unsupported" {
            fputs(
                "fs.unmount unsupported (agent=\(hello.version)); falling back to exec unmount\n",
                stderr
            )
        }
        try execVirtiofsBind(
            client: client,
            action: "unmount",
            name: mount.name,
            target: mount.target,
            readOnly: false
        )
    }

    /// Run `/usr/local/lib/vzctl/virtiofs-bind` via agent exec (PID-1 mount ns).
    private static func execVirtiofsBind(
        client: GuestAgentClient,
        action: String,
        name: String,
        target: String,
        readOnly: Bool
    ) throws {
        var argv = [
            "sudo", "-n", "/usr/local/lib/vzctl/virtiofs-bind",
            action, name, target,
        ]
        if action == "mount", readOnly {
            argv.append("ro")
        }
        let result = try client.exec(argv: argv, timeoutMilliseconds: 30_000)
        guard result.exit == 0 else {
            let detail = result.stderr.isEmpty ? result.stdout : result.stderr
            let message = detail.isEmpty ? "virtiofs-bind \(action) failed" : detail
            throw HelperError.invalid(
                "guest virtiofs \(action) \(name) → \(target): \(message)"
            )
        }
    }
}
