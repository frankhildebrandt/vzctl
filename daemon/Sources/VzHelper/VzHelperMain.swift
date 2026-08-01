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
            case let .agentSmoke(options):
                try await agentSmoke(options)
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

    private static func agentSmoke(_ options: RunOptions) async throws {
        let version = ProcessInfo.processInfo.operatingSystemVersion
        guard version.majorVersion >= VzDaemonKit.minMacOSMajor else {
            throw HelperError.invalid("macOS \(VzDaemonKit.minMacOSMajor)+ required")
        }
        guard !options.mock else {
            throw HelperError.usage("agent-smoke does not support --mock")
        }
        let token = try AgentToken.load(from: options.agentTokenURL)
        let lock = try HelperLock(vmID: options.vmID, stateDirectory: options.stateDirectory)
        defer { withExtendedLifetime(lock) {} }

        signal(SIGPIPE, SIG_IGN)
        let runtime = try VirtualMachineRuntime(options: options)
        do {
            try await runtime.start()
            print("vm=running transport=virtio-vsock port=\(guestAgentPort)")
            fflush(stdout)

            let client = try await connectReady(runtime: runtime, token: token, timeout: 90)
            defer { client.close() }
            let hello = try client.version()
            try requireCapabilities(hello.capabilities)
            try client.ping(nonce: "helper-agent-e2e")
            let health = try client.health()
            print("hello=ok version=\(hello.version) ping=ok health=\(health)")

            if let reason = options.timeHintReason {
                let result = try client.timeHint(reason: reason)
                printTimeHint(result, reason: reason)
                await runtime.stop()
                return
            }

            let executed = try client.exec(
                argv: ["/bin/sh", "-c", "printf helper-stdout; printf helper-stderr >&2"]
            )
            guard
                executed.exit == 0,
                executed.stdout == "helper-stdout",
                executed.stderr == "helper-stderr",
                !executed.truncated
            else {
                throw HelperError.invalid("guest agent exec returned unexpected output")
            }
            print("exec=ok exit=0 stdout=ok stderr=ok")

            let interfaces = try client.reportIP()
            let addressCount = interfaces.reduce(0) { $0 + $1.addresses.count }
            guard addressCount > 0 else {
                throw HelperError.invalid("guest agent report_ip returned no addresses")
            }
            print("report_ip=ok interfaces=\(interfaces.count) addresses=\(addressCount)")

            do {
                _ = try client.exec(
                    argv: ["/bin/sleep", "2"],
                    timeoutMilliseconds: 25,
                    helperTimeout: 1
                )
                throw HelperError.invalid("guest agent exec timeout was not enforced")
            } catch let GuestAgentError.remote(code, _, _) where code == "timeout" {
                print("timeout=ok")
            }

            let stopClient = try await connectAuthenticated(runtime: runtime, token: token)
            do {
                _ = try stopClient.exec(
                    argv: ["/usr/bin/pkill", "-STOP", "-x", "vzctl-agent"],
                    timeoutMilliseconds: 5_000,
                    helperTimeout: 0.5
                )
                throw HelperError.invalid("agent-stop command unexpectedly returned")
            } catch let error as GuestAgentError {
                switch error {
                case .timeout, .unavailable:
                    break
                default:
                    throw error
                }
            }
            stopClient.close()
            try await Task.sleep(for: .milliseconds(100))
            do {
                let downClient = try await connectAuthenticated(
                    runtime: runtime,
                    token: token,
                    connectTimeout: 1,
                    helloTimeout: 0.5
                )
                downClient.close()
                throw HelperError.invalid("agent-down check unexpectedly connected")
            } catch let error as GuestAgentError {
                print("agent_down=ok error=\(error)")
            }
            print("happy_path=virtio-vsock ssh=false")
            await runtime.stop()
        } catch {
            await runtime.stop()
            throw error
        }
    }

    private static func connectReady(
        runtime: VirtualMachineRuntime,
        token: String,
        timeout: TimeInterval
    ) async throws -> GuestAgentClient {
        let deadline = Date().addingTimeInterval(timeout)
        var lastError: Error = GuestAgentError.unavailable("agent did not become ready")
        while Date() < deadline {
            do {
                let client = try await connectAuthenticated(runtime: runtime, token: token)
                let hello = try client.version()
                try requireCapabilities(hello.capabilities)
                return client
            } catch let error as GuestAgentError {
                if case let .remote(code, _, _) = error, code == "auth" {
                    throw error
                }
                lastError = error
            } catch {
                lastError = error
            }
            try await Task.sleep(for: .seconds(1))
        }
        throw lastError
    }

    private static func connectAuthenticated(
        runtime: VirtualMachineRuntime,
        token: String,
        connectTimeout: TimeInterval = 5,
        helloTimeout: TimeInterval = 2
    ) async throws -> GuestAgentClient {
        let client = try await runtime.connectToGuestAgent(timeout: connectTimeout)
        do {
            let hello = try client.hello(
                token: token,
                helperVersion: VzDaemonKit.version,
                timeout: helloTimeout
            )
            try requireCapabilities(hello.capabilities)
            return client
        } catch {
            client.close()
            throw error
        }
    }

    private static func requireCapabilities(_ capabilities: [String]) throws {
        let required = Set(["ping", "version", "health", "exec", "report_ip", "time_hint"])
        let missing = required.subtracting(capabilities)
        guard missing.isEmpty else {
            throw GuestAgentError.protocolViolation(
                "missing capabilities: \(missing.sorted().joined(separator: ","))"
            )
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
            let timeSyncToken: String?
            if FileManager.default.fileExists(atPath: options.agentTokenURL.path) {
                timeSyncToken = try AgentToken.load(from: options.agentTokenURL)
            } else {
                timeSyncToken = nil
            }
            let vmnetNICs: [HelperVmnetNIC]
            if options.mock {
                vmnetNICs = []
            } else {
                vmnetNICs = try HelperVmnetClient.fetchAttachments(
                    vmID: options.vmID,
                    socketPath: options.supervisorSocket,
                    bundleURL: options.bundleURL
                )
            }
            if vmnetNICs.isEmpty {
                fputs(
                    "vmnet attachments: none (NAT fallback for standalone helper)\n",
                    stderr
                )
            } else {
                let summary = vmnetNICs
                    .map { "\($0.networkName)=\($0.ip)" }
                    .joined(separator: ",")
                fputs("vmnet attachments: \(summary)\n", stderr)
            }
            let created = try VirtualMachineRuntime(options: options, vmnetNICs: vmnetNICs)
            runtime = created
            try await created.start()
            try created.startConsoleServer(
                stateDirectory: stateDirectory,
                vmID: options.vmID
            )
            defer { created.stopConsoleServer() }
            let control = timeSyncToken.map { token in
                HelperControlServer(
                    vmID: options.vmID,
                    stateDirectory: stateDirectory,
                    routeHandler: { operation, plan in
                        try await RouterGuestConfigurator.run(
                            operation,
                            plan,
                            runtime: created,
                            token: token
                        )
                    },
                    agentHandler: { method, params in
                        try await HelperAgentProxy.run(
                            method: method,
                            params: params,
                            runtime: created,
                            token: token,
                            stateDirectory: stateDirectory,
                            vmID: options.vmID
                        )
                    },
                    mountHandler: { method, params in
                        switch method {
                        case "mount.list":
                            return HelperMountConfigurator.list(runtime: created)
                        case "mount.add":
                            return try await HelperMountConfigurator.add(
                                params: params,
                                runtime: created,
                                token: token
                            )
                        case "mount.remove":
                            return try await HelperMountConfigurator.remove(
                                params: params,
                                runtime: created,
                                token: token
                            )
                        default:
                            throw HelperError.invalid("unknown mount method: \(method)")
                        }
                    }
                )
            }
            try control?.start()
            defer { control?.stop() }
            if let token = timeSyncToken {
                Task {
                    try? await Task.sleep(for: .seconds(8))
                    try? await HelperMountConfigurator.applyManifestMounts(
                        runtime: created,
                        token: token
                    )
                }
            }
            print(
                "vm-id=\(options.vmID) state=running serial=\(created.serialLogURL.path)"
            )
            fflush(stdout)
            reporter.report(.running)
            let heartbeat = heartbeatTask(reporter: reporter, state: .running)
            let timeSync = timeSyncToken.map {
                guestTimeSyncTask(runtime: created, token: $0, reporter: reporter)
            }

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
            timeSync?.cancel()

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

    private static func guestTimeSyncTask(
        runtime: VirtualMachineRuntime,
        token: String,
        reporter: SupervisorReporter
    ) -> Task<Void, Never> {
        Task.detached {
            await sendTimeHint(
                runtime: runtime,
                token: token,
                reason: .handshake,
                reporter: reporter,
                connectTimeout: 90
            )

            var detector = HostWakeDetector()
            _ = detector.observe(Date())
            while !Task.isCancelled {
                do {
                    try await Task.sleep(for: .seconds(1))
                } catch {
                    return
                }
                if detector.observe(Date()) {
                    await sendTimeHint(
                        runtime: runtime,
                        token: token,
                        reason: .wake,
                        reporter: reporter,
                        connectTimeout: 30
                    )
                }
            }
        }
    }

    private static func sendTimeHint(
        runtime: VirtualMachineRuntime,
        token: String,
        reason: AgentTimeHintReason,
        reporter: SupervisorReporter,
        connectTimeout: TimeInterval
    ) async {
        do {
            let client = try await connectReady(
                runtime: runtime,
                token: token,
                timeout: connectTimeout
            )
            defer { client.close() }
            let result = try client.timeHint(reason: reason)
            printTimeHint(result, reason: reason)
            if result.action == .stepped {
                reporter.reportClockCorrection(result, reason: reason)
            }
        } catch {
            fputs("time_hint reason=\(reason.rawValue) failed: \(error)\n", stderr)
        }
    }

    private static func printTimeHint(
        _ result: AgentTimeHintResult,
        reason: AgentTimeHintReason
    ) {
        let event = result.action == .stepped ? "vm.clock_corrected" : "vm.clock_checked"
        print(
            "event=\(event) reason=\(reason.rawValue) "
                + "observed_guest_unix_ms=\(result.observedGuestUnixMS) "
                + "offset_ms=\(result.offsetMS) action=\(result.action.rawValue)"
        )
        fflush(stdout)
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
            [--disk <raw>] [--data-disk <raw>] [--cidata <iso>] [--agent-token <file>]
            [--supervisor-sock <path>]
          vz-helper agent-smoke --vm-id <id> --bundle <dir>
            [--disk <raw>] [--data-disk <raw>] [--cidata <iso>] [--agent-token <file>]
            [--time-hint handshake|wake|manual]
          vz-helper launchd-plist --vm-id <id> --bundle <dir>
            [--disk <raw>] [--data-disk <raw>] [--cidata <iso>] [--agent-token <file>]
            [--supervisor-sock <path>]
            [--executable <path>]

        Bundle defaults: disk.raw, optional dataDisk.raw/cidata.iso, agent.token, generated nvram.bin.
        run sends time_hint after agent handshake and after detected host wake when agent.token exists.
        agent-smoke --time-hint sends one hint and skips the destructive exec/down checks.
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
            "--agent-token", run.agentTokenURL.path,
        ]
        if let dataDisk = run.dataDiskURL {
            arguments += ["--data-disk", dataDisk.path]
        }
        if let cidata = run.cidataURL {
            arguments += ["--cidata", cidata.path]
        }
        if let macAddress = run.macAddress {
            arguments += ["--mac-address", macAddress]
        }
        if run.mock { arguments.append("--mock") }

        let argumentXML = arguments.map {
            "        <string>\(xmlEscape($0))</string>"
        }.joined(separator: "\n")
        let label = "com.vzctl.helper.\(StateFileName.component(run.vmID))"
        let logBase = (try? StatePaths.logsDirectory()) ?? URL(fileURLWithPath: "/tmp")
        let stdout = logBase.appendingPathComponent(
            "\(StateFileName.component(run.vmID)).helper.log"
        )
        let stderr = logBase.appendingPathComponent(
            "\(StateFileName.component(run.vmID)).helper.error.log"
        )

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
