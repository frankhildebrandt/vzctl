import Foundation
import VzDaemonKit

@main
enum VzHelperMain {
    static func main() {
        let args = Array(CommandLine.arguments.dropFirst())
        let vmID = args.first { !$0.hasPrefix("-") } ?? "(unset)"
        switch args.first {
        case "help", "-h", "--help":
            print(
                """
                vz-helper — per-VM process (P0 stub)

                Usage:
                  vz-helper <vm-id>
                  vz-helper version

                Owns: VZVirtualMachine (ADR 0002). Connects UDS back to supervisor.
                """
            )
        case "version":
            print("vz-helper \(VzDaemonKit.version)")
        default:
            print("vz-helper \(VzDaemonKit.version) vm-id=\(vmID)")
            print("stub: would load VM config + vmnet attachment, then run VZVirtualMachine")
            // Alpha stub exits 0 so launchd smoke tests can succeed later.
        }
    }
}
