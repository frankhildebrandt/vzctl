import Darwin
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
        case "help", "-h", "--help":
            print(
                """
                vz-supervisor — stack supervisor (P0 stub)

                Commands:
                  version
                  doctor
                  help

                Owns: vmnet registry, DNS listeners, apply journal (ADR 0002/0003).
                Spawns: vz-helper per VM via launchd (not yet wired).
                """
            )
        default:
            fputs("unknown: \(args.first!)\n", stderr)
            exit(VzExit.usage.rawValue)
        }
    }
}
