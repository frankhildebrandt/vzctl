import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzSupervisorMain {
    static func main() {
        let args = Array(CommandLine.arguments.dropFirst())
        switch args.first {
        case "version", nil:
            print("vz-supervisor \(VzDaemonKit.version) (macOS ≥ \(VzDaemonKit.minMacOSMajor))")
            print("ownership: vmnet+DNS+journal (ADR 0002)")
        case "doctor":
            let v = ProcessInfo.processInfo.operatingSystemVersion
            print("host: macOS \(v.majorVersion).\(v.minorVersion).\(v.patchVersion)")
            if v.majorVersion < VzDaemonKit.minMacOSMajor {
                fputs("error: macOS \(VzDaemonKit.minMacOSMajor)+ required (ADR 0001)\n", stderr)
                exit(VzExit.hostTooOld.rawValue)
            }
            print("doctor: ok")
        case "serve", "run":
            let v = ProcessInfo.processInfo.operatingSystemVersion
            guard v.majorVersion >= VzDaemonKit.minMacOSMajor else {
                fputs("error: macOS \(VzDaemonKit.minMacOSMajor)+ required (ADR 0001)\n", stderr)
                exit(VzExit.hostTooOld.rawValue)
            }
            do {
                let stateDirectory: URL
                if let override = ProcessInfo.processInfo.environment["VZCTL_STATE_DIR"] {
                    stateDirectory = URL(fileURLWithPath: override, isDirectory: true)
                } else {
                    stateDirectory = try FileManager.default.url(
                        for: .applicationSupportDirectory,
                        in: .userDomainMask,
                        appropriateFor: nil,
                        create: true
                    ).appendingPathComponent("vzctl", isDirectory: true)
                }
                let server = try SupervisorServer(stateDirectory: stateDirectory)
                signal(SIGPIPE, SIG_IGN)
                signal(SIGINT, SIG_IGN)
                signal(SIGTERM, SIG_IGN)
                let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .global())
                let terminate = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
                interrupt.setEventHandler(handler: { @Sendable in server.stop() })
                terminate.setEventHandler(handler: { @Sendable in server.stop() })
                interrupt.resume()
                terminate.resume()
                defer {
                    interrupt.cancel()
                    terminate.cancel()
                    server.stop()
                }
                print("listening: \(server.socketPath)")
                print("state: \(server.databasePath)")
                fflush(stdout)
                try server.run()
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        case "help", "-h", "--help":
            print(
                """
                vz-supervisor — stack supervisor (P0)

                Commands:
                  version
                  doctor
                  serve
                  help

                Owns: vmnet registry, DNS listeners, apply journal (ADR 0002/0003).
                Accepts: helper.hello/helper.state and exposes records via vm.list.
                Helper launchd jobs are standalone in #10; supervisor spawning follows.
                """
            )
        default:
            fputs("unknown: \(args.first!)\n", stderr)
            exit(VzExit.usage.rawValue)
        }
    }
}
