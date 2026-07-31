import Foundation
import VzDaemonKit

enum HelperCommand {
    case version
    case help
    case run(RunOptions)
    case agentSmoke(RunOptions)
    case launchdPlist(LaunchdOptions)
}

struct RunOptions: Sendable {
    let vmID: String
    let bundleURL: URL
    let supervisorSocket: String
    let diskURL: URL
    let dataDiskURL: URL?
    let cidataURL: URL?
    let macAddress: String?
    let cpuCount: Int
    let memorySize: UInt64
    let agentTokenURL: URL
    let mounts: [VirtioFSMountSpec]
    let timeHintReason: AgentTimeHintReason?
    let mock: Bool
    let stateDirectory: URL
}

struct LaunchdOptions {
    let run: RunOptions
    let executableURL: URL
}

enum HelperArguments {
    static func parse(_ arguments: [String], environment: [String: String]) throws -> HelperCommand {
        guard let command = arguments.first else { return .help }
        switch command {
        case "version":
            return .version
        case "help", "-h", "--help":
            return .help
        case "run":
            return .run(try parseRun(Array(arguments.dropFirst()), environment: environment))
        case "agent-smoke":
            return .agentSmoke(try parseRun(Array(arguments.dropFirst()), environment: environment))
        case "launchd-plist":
            let values = try parseValues(Array(arguments.dropFirst()))
            let executable = values["--executable"].map {
                URL(fileURLWithPath: $0).standardizedFileURL
            } ?? URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
            return .launchdPlist(
                LaunchdOptions(
                    run: try runOptions(values: values, environment: environment),
                    executableURL: executable
                )
            )
        default:
            throw HelperError.usage("unknown command: \(command)")
        }
    }

    private static func parseRun(
        _ arguments: [String],
        environment: [String: String]
    ) throws -> RunOptions {
        try runOptions(values: parseValues(arguments), environment: environment)
    }

    private static func runOptions(
        values: [String: String],
        environment: [String: String]
    ) throws -> RunOptions {
        guard let vmID = values["--vm-id"], !vmID.isEmpty else {
            throw HelperError.usage("missing --vm-id")
        }
        guard let bundle = values["--bundle"], !bundle.isEmpty else {
            throw HelperError.usage("missing --bundle")
        }

        let bundleURL = URL(fileURLWithPath: bundle, isDirectory: true).standardizedFileURL
        let stateDirectory = try StatePaths.stateDirectory(environment: environment)
        let supervisorSocket = values["--supervisor-sock"]
            ?? stateDirectory.appendingPathComponent("vz.sock").path
        let diskURL = values["--disk"].map {
            URL(fileURLWithPath: $0).standardizedFileURL
        } ?? bundleURL.appendingPathComponent("disk.raw")
        let defaultDataDisk = bundleURL.appendingPathComponent("dataDisk.raw")
        let dataDiskURL = values["--data-disk"].map {
            URL(fileURLWithPath: $0).standardizedFileURL
        } ?? (FileManager.default.fileExists(atPath: defaultDataDisk.path) ? defaultDataDisk : nil)
        let defaultCidata = bundleURL.appendingPathComponent("cidata.iso")
        let cidataURL = values["--cidata"].map {
            URL(fileURLWithPath: $0).standardizedFileURL
        } ?? (FileManager.default.fileExists(atPath: defaultCidata.path) ? defaultCidata : nil)
        let macAddress = try values["--mac-address"] ?? manifestMACAddress(bundleURL: bundleURL)
        let resources = try resolveResources(values: values, bundleURL: bundleURL)
        let agentTokenURL = values["--agent-token"].map {
            URL(fileURLWithPath: $0).standardizedFileURL
        } ?? bundleURL.appendingPathComponent("agent.token")
        let timeHintReason: AgentTimeHintReason?
        if let rawReason = values["--time-hint"] {
            guard let reason = AgentTimeHintReason(rawValue: rawReason) else {
                throw HelperError.usage(
                    "--time-hint must be handshake, wake or manual"
                )
            }
            timeHintReason = reason
        } else {
            timeHintReason = nil
        }
        let mounts = try VirtioFSShare.loadManifestMounts(bundleURL: bundleURL)
        try VirtioFSShare.ensureHostDirectories(mounts)

        return RunOptions(
            vmID: vmID,
            bundleURL: bundleURL,
            supervisorSocket: supervisorSocket,
            diskURL: diskURL,
            dataDiskURL: dataDiskURL,
            cidataURL: cidataURL,
            macAddress: macAddress,
            cpuCount: resources.cpuCount,
            memorySize: resources.memorySize,
            agentTokenURL: agentTokenURL,
            mounts: mounts,
            timeHintReason: timeHintReason,
            mock: values["--mock"] != nil,
            stateDirectory: stateDirectory
        )
    }

    private static func parseValues(_ arguments: [String]) throws -> [String: String] {
        let valueFlags = Set([
            "--vm-id", "--bundle", "--supervisor-sock", "--disk", "--data-disk", "--cidata",
            "--mac-address", "--cpus", "--memory-mib", "--agent-token", "--executable",
            "--time-hint",
        ])
        let booleanFlags = Set(["--mock"])
        var values: [String: String] = [:]
        var index = 0
        while index < arguments.count {
            let flag = arguments[index]
            if booleanFlags.contains(flag) {
                values[flag] = "true"
                index += 1
            } else if valueFlags.contains(flag) {
                guard index + 1 < arguments.count else {
                    throw HelperError.usage("missing value for \(flag)")
                }
                values[flag] = arguments[index + 1]
                index += 2
            } else {
                throw HelperError.usage("unknown option: \(flag)")
            }
        }
        return values
    }

    private static func resolveResources(
        values: [String: String],
        bundleURL: URL
    ) throws -> (cpuCount: Int, memorySize: UInt64) {
        let defaults = (cpuCount: 2, memorySize: UInt64(1024 * 1024 * 1024))
        let manifest = try manifestResources(bundleURL: bundleURL) ?? defaults
        let cpuCount: Int
        if let raw = values["--cpus"] {
            guard let parsed = Int(raw), parsed > 0 else {
                throw HelperError.usage("--cpus must be a positive integer")
            }
            cpuCount = parsed
        } else {
            cpuCount = manifest.cpuCount
        }
        let memorySize: UInt64
        if let raw = values["--memory-mib"] {
            guard let mib = UInt64(raw), mib > 0 else {
                throw HelperError.usage("--memory-mib must be a positive integer")
            }
            guard mib <= UInt64.max / (1024 * 1024) else {
                throw HelperError.usage("--memory-mib is too large")
            }
            memorySize = mib * 1024 * 1024
        } else {
            memorySize = manifest.memorySize
        }
        return (cpuCount, memorySize)
    }

    private static func manifestMACAddress(bundleURL: URL) throws -> String? {
        let manifestURL = bundleURL.appendingPathComponent("vm.json")
        guard FileManager.default.fileExists(atPath: manifestURL.path) else { return nil }
        do {
            let data = try Data(contentsOf: manifestURL)
            guard
                let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                let identity = root["identity"] as? [String: Any],
                let nics = identity["nics"] as? [[String: Any]],
                let mac = nics.first?["mac"] as? String,
                !mac.isEmpty
            else {
                throw HelperError.invalid(
                    "VM manifest has no identity.nics[0].mac: \(manifestURL.path)"
                )
            }
            return mac
        } catch let error as HelperError {
            throw error
        } catch {
            throw HelperError.invalid("cannot read VM identity from \(manifestURL.path): \(error)")
        }
    }

    static func manifestResources(bundleURL: URL) throws -> (cpuCount: Int, memorySize: UInt64)? {
        let manifestURL = bundleURL.appendingPathComponent("vm.json")
        guard FileManager.default.fileExists(atPath: manifestURL.path) else { return nil }
        do {
            let data = try Data(contentsOf: manifestURL)
            guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
                return nil
            }
            guard let resources = root["resources"] as? [String: Any] else {
                return nil
            }
            let cpuCount: Int
            if let value = resources["cpus"] as? Int {
                cpuCount = value
            } else if let value = resources["cpus"] as? NSNumber {
                cpuCount = value.intValue
            } else {
                cpuCount = 2
            }
            let memoryMib: UInt64
            if let value = resources["memory_mib"] as? Int, value > 0 {
                memoryMib = UInt64(value)
            } else if let value = resources["memory_mib"] as? NSNumber, value.intValue > 0 {
                memoryMib = value.uint64Value
            } else {
                memoryMib = 1024
            }
            guard cpuCount > 0 else {
                throw HelperError.invalid("VM manifest resources.cpus must be > 0")
            }
            guard memoryMib <= UInt64.max / (1024 * 1024) else {
                throw HelperError.invalid("VM manifest resources.memory_mib is too large")
            }
            return (cpuCount, memoryMib * 1024 * 1024)
        } catch let error as HelperError {
            throw error
        } catch {
            throw HelperError.invalid("cannot read VM resources from \(manifestURL.path): \(error)")
        }
    }
}

enum HelperError: Error, CustomStringConvertible {
    case usage(String)
    case system(String, Int32)
    case invalid(String)
    case alreadyRunning(vmID: String, pid: String)

    var description: String {
        switch self {
        case let .usage(message), let .invalid(message):
            return message
        case let .system(operation, code):
            return "\(operation): \(String(cString: strerror(code)))"
        case let .alreadyRunning(vmID, pid):
            return "helper already running for \(vmID) (pid \(pid))"
        }
    }
}
