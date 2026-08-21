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

    struct ServicesHTTPParams: Equatable, Sendable {
        let name: String
        let method: String
        let path: String
        let headers: [String: String]
        let body: Data?
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

    static func parseNetworkProbe(_ value: JSONValue?) throws -> [String: Any] {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.network_probe params must be an object")
        }
        var forwarded: [String: Any] = [:]
        if case let .string(url)? = params["url"], !url.isEmpty {
            forwarded["url"] = url
        }
        if case let .string(target)? = params["target"], !target.isEmpty {
            forwarded["target"] = target
        }
        if forwarded["url"] != nil && forwarded["target"] != nil {
            throw RouteApplyError.invalid("agent.network_probe cannot set both url and target")
        }
        if forwarded["url"] == nil && forwarded["target"] == nil {
            throw RouteApplyError.invalid("agent.network_probe requires url or target")
        }
        if case let .string(via)? = params["via"] {
            guard ["dns", "ip", "both"].contains(via) else {
                throw RouteApplyError.invalid("agent.network_probe via must be dns, ip, or both")
            }
            forwarded["via"] = via
        }
        if case let .string(connectIP)? = params["connect_ip"], !connectIP.isEmpty {
            forwarded["connect_ip"] = connectIP
        }
        if case let .number(raw)? = params["timeout_ms"] {
            guard raw.rounded() == raw, raw >= 100, raw <= 30_000 else {
                throw RouteApplyError.invalid(
                    "agent.network_probe timeout_ms must be 100...30000"
                )
            }
            forwarded["timeout_ms"] = Int(raw)
        } else {
            forwarded["timeout_ms"] = 5_000
        }
        return forwarded
    }

    static func parseServicesHTTP(_ value: JSONValue?) throws -> ServicesHTTPParams {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.services.http params must be an object")
        }
        guard case let .string(name)? = params["name"], !name.isEmpty else {
            throw RouteApplyError.invalid("agent.services.http requires name")
        }
        guard case let .string(path)? = params["path"], path.hasPrefix("/") else {
            throw RouteApplyError.invalid("agent.services.http requires a root-relative path")
        }
        let method: String
        if case let .string(value)? = params["method"], !value.isEmpty {
            method = value
        } else {
            method = "GET"
        }
        var headers: [String: String] = [:]
        if case let .object(raw)? = params["headers"] {
            for (key, value) in raw {
                guard case let .string(string) = value else {
                    throw RouteApplyError.invalid("agent.services.http headers must be strings")
                }
                headers[key] = string
            }
        }
        let body: Data?
        if case let .string(encoded)? = params["body_b64"] {
            guard let data = Data(base64Encoded: encoded) else {
                throw RouteApplyError.invalid("agent.services.http body_b64 is not valid base64")
            }
            body = data
        } else {
            body = nil
        }
        return ServicesHTTPParams(name: name, method: method, path: path, headers: headers, body: body)
    }

    struct SystemdListParams: Equatable, Sendable {
        let type: String?
        let all: Bool
    }

    struct SystemdControlParams: Equatable, Sendable {
        let unit: String
        let action: String
    }

    struct SystemdEventsParams: Equatable, Sendable {
        let since: String?
        let limit: Int
    }

    static func parseSystemdList(_ value: JSONValue?) throws -> SystemdListParams {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.systemd.list params must be an object")
        }
        let type: String?
        if case let .string(raw)? = params["type"], !raw.isEmpty {
            type = raw
        } else if params["type"] == nil || params["type"] == .null {
            type = nil
        } else {
            throw RouteApplyError.invalid("agent.systemd.list type must be a string")
        }
        let all: Bool
        if case let .bool(value)? = params["all"] {
            all = value
        } else if params["all"] == nil || params["all"] == .null {
            all = false
        } else {
            throw RouteApplyError.invalid("agent.systemd.list all must be a boolean")
        }
        return SystemdListParams(type: type, all: all)
    }

    static func parseSystemdUnit(_ value: JSONValue?) throws -> String {
        guard case let .object(params)? = value,
              case let .string(unit)? = params["unit"],
              !unit.isEmpty
        else {
            throw RouteApplyError.invalid("agent.systemd.show requires unit")
        }
        return unit
    }

    static func parseSystemdControl(_ value: JSONValue?) throws -> SystemdControlParams {
        guard case let .object(params)? = value,
              case let .string(unit)? = params["unit"],
              !unit.isEmpty,
              case let .string(action)? = params["action"],
              ["start", "stop", "restart"].contains(action)
        else {
            throw RouteApplyError.invalid("agent.systemd.control requires unit and action")
        }
        return SystemdControlParams(unit: unit, action: action)
    }

    static func parseSystemdEvents(_ value: JSONValue?) throws -> SystemdEventsParams {
        guard case let .object(params)? = value else {
            throw RouteApplyError.invalid("agent.systemd.events params must be an object")
        }
        let since: String?
        if case let .string(raw)? = params["since"], !raw.isEmpty {
            since = raw
        } else if params["since"] == nil || params["since"] == .null {
            since = nil
        } else {
            throw RouteApplyError.invalid("agent.systemd.events since must be a string")
        }
        let limit: Int
        if case let .number(raw)? = params["limit"] {
            guard raw.rounded() == raw, raw > 0, raw <= 512 else {
                throw RouteApplyError.invalid("agent.systemd.events limit must be 1...512")
            }
            limit = Int(raw)
        } else if params["limit"] == nil || params["limit"] == .null {
            limit = 100
        } else {
            throw RouteApplyError.invalid("agent.systemd.events limit must be a number")
        }
        return SystemdEventsParams(since: since, limit: limit)
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
        "agent.network_probe",
        "agent.stats",
        "agent.services.list",
        "agent.services.http",
        "agent.services.stream",
        "agent.systemd.status",
        "agent.systemd.list",
        "agent.systemd.show",
        "agent.systemd.control",
        "agent.systemd.events",
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
        if method == "agent.services.stream" {
            guard let stateDirectory, let vmID else {
                throw RouteApplyError.invalid("agent.services.stream requires helper state context")
            }
            let parsed = try HelperAgentRequest.parseServicesHTTP(params)
            return try await HelperGuestServiceBridge.start(
                params: parsed,
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
            return JSONValue.fromAny(try client.health())
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
        case "agent.network_probe":
            let forwarded = try HelperAgentRequest.parseNetworkProbe(params)
            let timeoutMS = forwarded["timeout_ms"] as? Int ?? 5_000
            return JSONValue.fromAny(
                try client.networkProbe(
                    params: forwarded,
                    timeout: TimeInterval(timeoutMS) / 1_000 + 5
                )
            )
        case "agent.stats":
            return JSONValue.fromAny(try client.stats())
        case "agent.services.list":
            return .object([
                "services": .array(
                    try client.servicesList().map { JSONValue.fromAny($0) }
                )
            ])
        case "agent.services.http":
            let parsed = try HelperAgentRequest.parseServicesHTTP(params)
            return JSONValue.fromAny(
                try client.servicesHTTP(
                    name: parsed.name,
                    method: parsed.method,
                    path: parsed.path,
                    headers: parsed.headers,
                    body: parsed.body
                )
            )
        case "agent.systemd.status":
            return JSONValue.fromAny(try client.systemdStatus())
        case "agent.systemd.list":
            let listParams = try HelperAgentRequest.parseSystemdList(params)
            return .object([
                "units": .array(
                    try client.systemdList(type: listParams.type, all: listParams.all)
                        .map { JSONValue.fromAny($0) }
                ),
            ])
        case "agent.systemd.show":
            let unit = try HelperAgentRequest.parseSystemdUnit(params)
            return .object(["unit": JSONValue.fromAny(try client.systemdShow(unit: unit))])
        case "agent.systemd.control":
            let control = try HelperAgentRequest.parseSystemdControl(params)
            return JSONValue.fromAny(
                try client.systemdControl(unit: control.unit, action: control.action)
            )
        case "agent.systemd.events":
            let eventsParams = try HelperAgentRequest.parseSystemdEvents(params)
            return JSONValue.fromAny(
                try client.systemdEvents(
                    since: eventsParams.since,
                    limit: eventsParams.limit
                )
            )
        default:
            throw RouteApplyError.invalid("unknown agent method: \(method)")
        }
    }
}
