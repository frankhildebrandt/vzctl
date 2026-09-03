import Foundation

struct CaddyIntent: Equatable {
    let project: String
    let caddyfile: String
    let binary: String
    let backendHTTP: UInt16
    let backendHTTPS: UInt16
    let workDir: URL
    let configPath: String
}

/// Decide one shared Caddy child vs one child per project.
enum CaddyProcessPlan {
    struct Child: Equatable {
        let project: String
        let name: String
        let binary: String
        let backendHTTP: UInt16
        let workDir: URL
        let configPath: String
        let caddyfile: String
        let reapWorkDirs: [URL]
        let shared: Bool
    }

    /// Shared :80/:443 when every intent uses the same binary and backend ports.
    static func children(from intents: [CaddyIntent]) -> [Child] {
        guard let first = intents.first else { return [] }
        let canShare = intents.allSatisfy {
            $0.binary == first.binary
                && $0.backendHTTP == first.backendHTTP
                && $0.backendHTTPS == first.backendHTTPS
        }
        if canShare {
            return [
                Child(
                    project: "shared",
                    name: "caddy-shared",
                    binary: first.binary,
                    backendHTTP: first.backendHTTP,
                    workDir: URL(fileURLWithPath: ""),
                    configPath: "",
                    caddyfile: CaddyfileMerge.merge(intents.map(\.caddyfile)),
                    reapWorkDirs: intents.map(\.workDir),
                    shared: true
                )
            ]
        }
        return intents.map { intent in
            Child(
                project: intent.project,
                name: "caddy-\(intent.project)",
                binary: intent.binary,
                backendHTTP: intent.backendHTTP,
                workDir: intent.workDir,
                configPath: intent.configPath,
                caddyfile: intent.caddyfile,
                reapWorkDirs: [],
                shared: false
            )
        }
    }
}
