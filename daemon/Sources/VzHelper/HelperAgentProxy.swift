import Foundation
import VzDaemonKit

enum HelperAgentRequest {
    struct ExecParams: Equatable, Sendable {
        let cmd: [String]
        let cwd: String?
        let env: [String: String]
        let timeoutMilliseconds: Int
        let stdin: Data?
    }

    struct ExecTTYParams: Equatable, Sendable {
        let cmd: [String]
        let cwd: String?
        let env: [String: String]
        let cols: Int
        let rows: Int
    }

    struct CAInjectParams: Equatable, Sendable {
        let pem: String
        let fingerprint: String
        let name: String
    }

    static func parseExec(_ value: JSONValue?) throws -> ExecParams {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.exec params must be an object")
        }
        guard case let .array(rawCmd)? = params["cmd"], !rawCmd.isEmpty else {
            throw RouteApplyError.invalid("agent.exec requires non-empty cmd[]")
        }
        let cmd = try rawCmd.map { item -> String in
            guard case let .string(value) = item else {
                throw RouteApplyError.invalid("agent.exec cmd entries must be strings")
            }
            return value
        }
        let cwd: String?
        if case let .string(value)? = params["cwd"] {
            cwd = value
        } else if params["cwd"] == nil || params["cwd"] == .null {
            cwd = nil
        } else {
            throw RouteApplyError.invalid("agent.exec cwd must be a string")
        }
        var env: [String: String] = [:]
        if case let .object(rawEnv)? = params["env"] {
            for (key, value) in rawEnv {
                guard case let .string(string) = value else {
                    throw RouteApplyError.invalid("agent.exec env values must be strings")
                }
                env[key] = string
            }
        } else if params["env"] != nil && params["env"] != .null {
            throw RouteApplyError.invalid("agent.exec env must be an object")
        }
        let timeoutMilliseconds: Int
        if case let .number(value)? = params["timeout_ms"] {
            guard value.rounded() == value, value > 0, value <= Double(Int.max) else {
                throw RouteApplyError.invalid("agent.exec timeout_ms must be a positive integer")
            }
            timeoutMilliseconds = Int(value)
        } else if params["timeout_ms"] == nil || params["timeout_ms"] == .null {
            timeoutMilliseconds = 30_000
        } else {
            throw RouteApplyError.invalid("agent.exec timeout_ms must be a number")
        }
        let stdin: Data?
        if case let .string(encoded)? = params["stdin_b64"] {
            guard let data = Data(base64Encoded: encoded) else {
                throw RouteApplyError.invalid("agent.exec stdin_b64 is not valid base64")
            }
            stdin = data
        } else if params["stdin_b64"] == nil || params["stdin_b64"] == .null {
            stdin = nil
        } else {
            throw RouteApplyError.invalid("agent.exec stdin_b64 must be a string")
        }
        return ExecParams(
            cmd: cmd,
            cwd: cwd,
            env: env,
            timeoutMilliseconds: timeoutMilliseconds,
            stdin: stdin
        )
    }

    static func parseExecTTY(_ value: JSONValue?) throws -> ExecTTYParams {
        let base = try parseExec(value)
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.exec_tty params must be an object")
        }
        if base.stdin != nil {
            throw RouteApplyError.invalid("agent.exec_tty does not accept stdin_b64")
        }
        let cols: Int
        if case let .number(value)? = params["cols"] {
            guard value.rounded() == value, value >= 1, value <= 65_535 else {
                throw RouteApplyError.invalid("agent.exec_tty cols must be 1...65535")
            }
            cols = Int(value)
        } else if params["cols"] == nil || params["cols"] == .null {
            cols = 80
        } else {
            throw RouteApplyError.invalid("agent.exec_tty cols must be a number")
        }
        let rows: Int
        if case let .number(value)? = params["rows"] {
            guard value.rounded() == value, value >= 1, value <= 65_535 else {
                throw RouteApplyError.invalid("agent.exec_tty rows must be 1...65535")
            }
            rows = Int(value)
        } else if params["rows"] == nil || params["rows"] == .null {
            rows = 24
        } else {
            throw RouteApplyError.invalid("agent.exec_tty rows must be a number")
        }
        return ExecTTYParams(cmd: base.cmd, cwd: base.cwd, env: base.env, cols: cols, rows: rows)
    }

    static func parseCAInject(_ value: JSONValue?) throws -> CAInjectParams {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.ca_inject params must be an object")
        }
        guard case let .string(pem)? = params["pem"], !pem.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw RouteApplyError.invalid("agent.ca_inject requires pem")
        }
        guard case let .string(fingerprint)? = params["fingerprint"], !fingerprint.isEmpty else {
            throw RouteApplyError.invalid("agent.ca_inject requires fingerprint")
        }
        let name: String
        if case let .string(value)? = params["name"] {
            name = value
        } else if params["name"] == nil || params["name"] == .null {
            name = "vzctl-local"
        } else {
            throw RouteApplyError.invalid("agent.ca_inject name must be a string")
        }
        return CAInjectParams(pem: pem, fingerprint: fingerprint, name: name)
    }
}

enum HelperAgentProxy {
    static let methods: Set<String> = [
        "agent.exec",
        "agent.exec_tty",
        "agent.health",
        "agent.version",
        "agent.report_ip",
        "agent.ping",
        "agent.ca_inject",
    ]

    static func run(
        method: String,
        params: JSONValue?,
        runtime: VirtualMachineRuntime,
        token: String,
        stateDirectory: URL? = nil,
        vmID: String? = nil
    ) async throws -> JSONValue {
        if method == "agent.exec_tty" {
            guard let stateDirectory, let vmID else {
                throw RouteApplyError.invalid("agent.exec_tty requires helper state context")
            }
            let tty = try HelperAgentRequest.parseExecTTY(params)
            return try await HelperExecTTYBridge.start(
                params: tty,
                runtime: runtime,
                token: token,
                stateDirectory: stateDirectory,
                vmID: vmID
            )
        }

        let client = try await runtime.connectToGuestAgent(timeout: 5)
        defer { client.close() }
        _ = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        switch method {
        case "agent.exec":
            let exec = try HelperAgentRequest.parseExec(params)
            let result = try client.exec(
                argv: exec.cmd,
                cwd: exec.cwd,
                environment: exec.env,
                stdin: exec.stdin,
                timeoutMilliseconds: exec.timeoutMilliseconds
            )
            return .object([
                "exit": .number(Double(result.exit)),
                "stdout": .string(result.stdout),
                "stderr": .string(result.stderr),
                "truncated": .bool(result.truncated),
            ])
        case "agent.health":
            return .object(["status": .string(try client.health())])
        case "agent.version":
            let version = try client.version()
            return .object([
                "v": .number(1),
                "agent_version": .string(version.version),
                "capabilities": .array(version.capabilities.map { .string($0) }),
            ])
        case "agent.report_ip":
            let interfaces = try client.reportIP()
            return .object([
                "interfaces": .array(
                    interfaces.map { iface in
                        .object([
                            "name": .string(iface.name),
                            "mac": .string(iface.mac),
                            "addresses": .array(iface.addresses.map { .string($0) }),
                        ])
                    }
                ),
            ])
        case "agent.ping":
            try client.ping()
            return .object(["pong": .bool(true)])
        case "agent.ca_inject":
            let ca = try HelperAgentRequest.parseCAInject(params)
            let result = try client.caInject(
                pem: ca.pem,
                fingerprint: ca.fingerprint,
                name: ca.name,
                timeout: 60
            )
            guard
                let installed = result["installed"] as? Bool,
                let fingerprint = result["fingerprint"] as? String,
                let name = result["name"] as? String
            else {
                throw RouteApplyError.guest("ca_inject returned an invalid result")
            }
            var response: [String: JSONValue] = [
                "installed": .bool(installed),
                "fingerprint": .string(fingerprint),
                "name": .string(name),
            ]
            if let path = result["path"] as? String {
                response["path"] = .string(path)
            }
            return .object(response)
        default:
            throw RouteApplyError.invalid("unknown agent method: \(method)")
        }
    }
}
