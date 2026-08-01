import Foundation

enum RestListenSpec: Equatable, Sendable {
    case unix(path: String)
    case tcp(host: String, port: UInt16)

    var description: String {
        switch self {
        case let .unix(path):
            return "unix:\(path)"
        case let .tcp(host, port):
            return "tcp:\(host):\(port)"
        }
    }

    static func parse(_ raw: String) throws -> RestListenSpec {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.hasPrefix("unix:") {
            let path = String(trimmed.dropFirst("unix:".count))
            guard !path.isEmpty else {
                throw RestConfigError.invalidListen("unix path is empty")
            }
            return .unix(path: path)
        }
        if trimmed.hasPrefix("tcp:") {
            let rest = String(trimmed.dropFirst("tcp:".count))
            guard let colon = rest.lastIndex(of: ":") else {
                throw RestConfigError.invalidListen("tcp listen requires host:port")
            }
            let host = String(rest[..<colon])
            let portText = String(rest[rest.index(after: colon)...])
            guard !host.isEmpty, let port = UInt16(portText), port > 0 else {
                throw RestConfigError.invalidListen("invalid tcp host/port")
            }
            // v1: loopback only
            let allowed = host == "127.0.0.1" || host == "localhost" || host == "::1"
            guard allowed else {
                throw RestConfigError.invalidListen("tcp listen must be loopback (127.0.0.1 / ::1)")
            }
            return .tcp(host: host == "localhost" ? "127.0.0.1" : host, port: port)
        }
        throw RestConfigError.invalidListen(
            "listen must be unix:<path> or tcp:<host>:<port>, got \(trimmed)"
        )
    }
}

enum RestConfigError: Error, CustomStringConvertible, Equatable {
    case invalidListen(String)

    var description: String {
        switch self {
        case let .invalidListen(message):
            return message
        }
    }
}

enum RestConfig {
    static func resolve(
        stateDirectory: URL,
        flagValue: String?,
        environment: [String: String] = ProcessInfo.processInfo.environment
    ) throws -> RestListenSpec {
        if let flagValue, !flagValue.isEmpty {
            return try RestListenSpec.parse(flagValue)
        }
        if let env = environment["VZCTL_API_LISTEN"], !env.isEmpty {
            return try RestListenSpec.parse(env)
        }
        let path = stateDirectory.appendingPathComponent("api.sock").path
        return .unix(path: path)
    }

    /// Parse `serve` argv for `--api-listen <spec>` (consumes flag + value).
    static func parseServeArgs(_ args: [String]) throws -> (apiListen: String?, remaining: [String]) {
        var apiListen: String?
        var remaining: [String] = []
        var i = 0
        while i < args.count {
            let arg = args[i]
            if arg == "--api-listen" {
                i += 1
                guard i < args.count else {
                    throw RestConfigError.invalidListen("--api-listen requires a value")
                }
                apiListen = args[i]
            } else if arg.hasPrefix("--api-listen=") {
                apiListen = String(arg.dropFirst("--api-listen=".count))
            } else {
                remaining.append(arg)
            }
            i += 1
        }
        return (apiListen, remaining)
    }
}
