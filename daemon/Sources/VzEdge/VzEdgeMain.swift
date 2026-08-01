import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzEdgeMain {
    static func main() {
        let command = CommandLine.arguments.dropFirst().first
        switch command {
        case "version", nil:
            print("vz-edge \(VzDaemonKit.version) (macOS ≥ \(VzDaemonKit.minMacOSMajor))")
            print("ownership: DNS + host proxies + Caddy/Dex runtime")
        case "serve", "run":
            do {
                let state = try resolveStateDirectory()
                let server = try EdgeServer(stateDirectory: state)
                signal(SIGPIPE, SIG_IGN)
                signal(SIGINT, SIG_IGN)
                signal(SIGTERM, SIG_IGN)
                let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .global())
                let terminate = DispatchSource.makeSignalSource(signal: SIGTERM, queue: .global())
                interrupt.setEventHandler { server.stop() }
                terminate.setEventHandler { server.stop() }
                interrupt.resume()
                terminate.resume()
                defer { interrupt.cancel(); terminate.cancel(); server.stop() }
                print("listening: \(server.socketPath)")
                fflush(stdout)
                try server.run()
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        case "doctor":
            do {
                let state = try resolveStateDirectory()
                let health = try VzEdgeClient(
                    socketPath: VzEdgeClient.defaultSocketPath(stateDirectory: state)
                ).health()
                print("edge: \(health)")
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        case "help", "-h", "--help":
            print("vz-edge — DNS, host proxies and embedded service supervisor\n\nCommands: version, doctor, serve, help")
        default:
            fputs("unknown command\n", stderr)
            exit(VzExit.usage.rawValue)
        }
    }

    private static func resolveStateDirectory() throws -> URL {
        if let override = ProcessInfo.processInfo.environment["VZCTL_STATE_DIR"] {
            return URL(fileURLWithPath: override, isDirectory: true)
        }
        return try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask,
            appropriateFor: nil, create: true
        ).appendingPathComponent("vzctl", isDirectory: true)
    }
}
