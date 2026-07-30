import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzHelperMain {
    static func main() async {
        do {
            switch try HelperArguments.parse(
                Array(CommandLine.arguments.dropFirst()),
                environment: ProcessInfo.processInfo.environment
            ) {
            case .version:
                print("vz-helper \(VzDaemonKit.version)")
            case .help:
                print(help)
            case let .launchdPlist(options):
                print(LaunchdPlist.render(options: options))
            case let .run(options):
                try await run(options)
            }
        } catch let error as HelperError {
            fputs("error: \(error)\n", stderr)
            if case .usage = error { fputs("\n\(help)\n", stderr) }
            exit(error.isUsage ? VzExit.usage.rawValue : 1)
        } catch {
            fputs("error: \(error)\n", stderr)
            exit(1)
        }
    }

    private static func run(_ options: RunOptions) async throws {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        guard version.majorVersion >= VzDaemonKit.minMacOSMajor else {
            throw HelperError.invalid("macOS \(VzDaemonKit.minMacOSMajor)+ required")
        }

        let stateDirectory = try StatePaths.stateDirectory(
            environment: ProcessInfo.processInfo.environment
        )
        let lock = try HelperLock(vmID: options.vmID, stateDirectory: stateDirectory)
        defer { withExtendedLifetime(lock) {} }

        signal(SIGPIPE, SIG_IGN)
        let signals = terminationSignals()
        let reporter = SupervisorReporter(
            vmID: options.vmID,
            bundle: options.bundleURL.path,
            socketPath: options.supervisorSocket
        )
        reporter.report(.starting, method: "helper.hello")

        if options.mock {
            print("vm-id=\(options.vmID) state=running mock=true lock=\(lock.url.path)")
            fflush(stdout)
            reporter.report(.running)
            let heartbeat = heartbeatTask(reporter: reporter, state: .running)
            _ = await signals.stream.first { _ in true }
            heartbeat.cancel()
            reporter.report(.stopped)
            return
        }

        var runtime: VirtualMachineRuntime?
        do {
            let created = try VirtualMachineRuntime(options: options)
            runtime = created
            try await created.start()
            print(
                "vm-id=\(options.vmID) state=running serial=\(created.serialLogURL.path)"
            )
            fflush(stdout)
            reporter.report(.running)
            let heartbeat = heartbeatTask(reporter: reporter, state: .running)

            let outcome = await withTaskGroup(of: RunOutcome.self) { group in
                group.addTask {
                    _ = await signals.stream.first { _ in true }
                    return .terminate
                }
                group.addTask {
                    .virtualMachine(await created.waitForStop())
                }
                let first = await group.next() ?? .terminate
                group.cancelAll()
                return first
            }
            heartbeat.cancel()

            switch outcome {
            case .terminate:
                await created.stop()
                reporter.report(.stopped)
            case .virtualMachine(.stopped):
                reporter.report(.stopped)
            case let .virtualMachine(.failed(message)):
                reporter.report(.failed)
                throw HelperError.invalid("virtual machine stopped with error: \(message)")
            }
        } catch {
            if let runtime { await runtime.stop() }
            reporter.report(.failed)
            throw error
        }
    }

    private static func heartbeatTask(
        reporter: SupervisorReporter,
        state: HelperState
    ) -> Task<Void, Never> {
        Task.detached {
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(5))
                if !Task.isCancelled { reporter.report(state, method: "helper.hello") }
            }
        }
    }

    private static func terminationSignals() -> SignalStream {
        signal(SIGINT, SIG_IGN)
        signal(SIGTERM, SIG_IGN)
        let pair = AsyncStream<Int32>.makeStream()
        let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .global())
        let terminate = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
        interrupt.setEventHandler { pair.continuation.yield(SIGINT) }
        terminate.setEventHandler { pair.continuation.yield(SIGTERM) }
        interrupt.resume()
        terminate.resume()
        return SignalStream(stream: pair.stream, sources: [interrupt, terminate])
    }

    private static let help = """
        vz-helper — one process and exactly one VZVirtualMachine per VM (ADR 0002)

        Usage:
          vz-helper version
          vz-helper run --vm-id <id> --bundle <dir>
            [--disk <raw>] [--cidata <iso>] [--supervisor-sock <path>]
          vz-helper launchd-plist --vm-id <id> --bundle <dir>
            [--disk <raw>] [--cidata <iso>] [--supervisor-sock <path>]
            [--executable <path>]

        Bundle defaults: disk.raw, optional cidata.iso, generated nvram.bin.
        Development only: --mock holds lifecycle/lock without creating a VM.
        """
}

private enum RunOutcome {
    case terminate
    case virtualMachine(VirtualMachineEvent)
}

private struct SignalStream: @unchecked Sendable {
    let stream: AsyncStream<Int32>
    let sources: [DispatchSourceSignal]
}

private extension HelperError {
    var isUsage: Bool {
        if case .usage = self { return true }
        return false
    }
}

enum LaunchdPlist {
    static func render(options: LaunchdOptions) -> String {
        let run = options.run
        var arguments = [
            options.executableURL.path,
            "run",
            "--vm-id", run.vmID,
            "--bundle", run.bundleURL.path,
            "--disk", run.diskURL.path,
            "--supervisor-sock", run.supervisorSocket,
        ]
        if let cidata = run.cidataURL {
            arguments += ["--cidata", cidata.path]
        }
        if run.mock { arguments.append("--mock") }

        let argumentXML = arguments.map {
            "        <string>\(xmlEscape($0))</string>"
        }.joined(separator: "\n")
        let label = "com.vzctl.helper.\(safeFileComponent(run.vmID))"
        let logBase = (try? StatePaths.logsDirectory()) ?? URL(fileURLWithPath: "/tmp")
        let stdout = logBase.appendingPathComponent("\(safeFileComponent(run.vmID)).helper.log")
        let stderr = logBase.appendingPathComponent("\(safeFileComponent(run.vmID)).helper.error.log")

        return """
        <?xml version="1.0" encoding="UTF-8"?>
        <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" \
        "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
        <plist version="1.0">
        <dict>
            <key>Label</key>
            <string>\(xmlEscape(label))</string>
            <key>ProgramArguments</key>
            <array>
        \(argumentXML)
            </array>
            <key>RunAtLoad</key>
            <true/>
            <key>KeepAlive</key>
            <dict>
                <key>SuccessfulExit</key>
                <false/>
            </dict>
            <key>ThrottleInterval</key>
            <integer>10</integer>
            <key>ProcessType</key>
            <string>Background</string>
            <key>EnvironmentVariables</key>
            <dict>
                <key>VZCTL_STATE_DIR</key>
                <string>\(xmlEscape(run.stateDirectory.path))</string>
            </dict>
            <key>StandardOutPath</key>
            <string>\(xmlEscape(stdout.path))</string>
            <key>StandardErrorPath</key>
            <string>\(xmlEscape(stderr.path))</string>
        </dict>
        </plist>
        """
    }

    private static func xmlEscape(_ value: String) -> String {
        value
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
            .replacingOccurrences(of: "\"", with: "&quot;")
            .replacingOccurrences(of: "'", with: "&apos;")
    }
}
