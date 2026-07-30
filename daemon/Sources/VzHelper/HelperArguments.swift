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
    let cidataURL: URL?
    let agentTokenURL: URL
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
        let defaultCidata = bundleURL.appendingPathComponent("cidata.iso")
        let cidataURL = values["--cidata"].map {
            URL(fileURLWithPath: $0).standardizedFileURL
        } ?? (FileManager.default.fileExists(atPath: defaultCidata.path) ? defaultCidata : nil)
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

        return RunOptions(
            vmID: vmID,
            bundleURL: bundleURL,
            supervisorSocket: supervisorSocket,
            diskURL: diskURL,
            cidataURL: cidataURL,
            agentTokenURL: agentTokenURL,
            timeHintReason: timeHintReason,
            mock: values["--mock"] != nil,
            stateDirectory: stateDirectory
        )
    }

    private static func parseValues(_ arguments: [String]) throws -> [String: String] {
        let valueFlags = Set([
            "--vm-id", "--bundle", "--supervisor-sock", "--disk", "--cidata", "--agent-token",
            "--executable", "--time-hint",
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
