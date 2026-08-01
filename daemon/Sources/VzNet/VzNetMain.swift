import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzNetMain {
    static func main() {
        let args = Array(CommandLine.arguments.dropFirst())
        switch args.first {
        case "version", nil:
            print("vz-net \(VzDaemonKit.version) (macOS ≥ \(VzDaemonKit.minMacOSMajor))")
            print("ownership: vmnet refs + host bridges (ADR 0002)")
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
                let stateDirectory = try resolveStateDirectory()
                let server = try NetServer(stateDirectory: stateDirectory)
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
                print("state: \(stateDirectory.path)")
                fflush(stdout)
                try server.run()
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        case "help", "-h", "--help":
            print(
                """
                vz-net — HyperNetwork Supervisor (vmnet refs)

                Commands:
                  version
                  doctor
                  serve
                  help

                Env:
                  VZCTL_STATE_DIR   state directory (default: ~/Library/Application Support/vzctl)

                Owns: vmnet_network_ref + host bridges. Contract: docs/specs/vz-net-v1.md
                """
            )
        default:
            fputs("unknown: \(args.first!)\n", stderr)
            exit(VzExit.usage.rawValue)
        }
    }

    private static func resolveStateDirectory() throws -> URL {
        if let override = ProcessInfo.processInfo.environment["VZCTL_STATE_DIR"] {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return try FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        ).appendingPathComponent("vzctl", isDirectory: true)
    }
}
