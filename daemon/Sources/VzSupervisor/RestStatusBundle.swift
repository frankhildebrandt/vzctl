import Foundation
import VzDaemonKit

enum RestStatusBundle {
    static func build(path: String, jobs: RestJobRunner, rpcVMs: () throws -> JSONValue,
                      inspectStack: ((String) throws -> StackStateRecord)? = nil) throws -> JSONValue {
        var sections: [String: JSONValue] = [:]

        let configURL = RestStackStore.configURL(for: path)
        let configText = (try? String(contentsOf: configURL, encoding: .utf8)) ?? ""
        var desired = desiredVMIds(from: configText)
        var projectName = projectName(from: configText)

        let healthResult = try? jobs.runSync(arguments: ["stack", "status", "-C", path, "--format", "json"])
        sections["health"] = section(healthResult)
        let health = healthResult.flatMap { decodeJSON($0.stdout) }

        sections["dns"] = section(try? jobs.runSync(arguments: ["dns", "status", "--format", "json"]))
        sections["certs"] = section(try? jobs.runSync(arguments: ["certs", "fingerprint", "--format", "json"]))
        var oidcArgs = ["oidc", "status", "--format", "json"]
        if let projectName {
            oidcArgs += ["--project", projectName]
        }
        if case let .object(report)? = health,
           case let .object(stack)? = report["stack"],
           case let .object(oidc)? = stack["oidc"], oidc["enabled"] == .bool(false)
        {
            sections["oidc"] = .object([
                "ok": .bool(true), "data": .object(["enabled": .bool(false), "status": .string("disabled")]),
            ])
        } else {
            sections["oidc"] = section(try? jobs.runSync(arguments: oidcArgs))
        }
        sections["diff"] = section(try? jobs.runSync(arguments: ["diff", "-C", path, "--format", "json"]))

        let allVMs: [JSONValue]
        if case let .object(report)? = health, case let .array(vms)? = report["vms"] {
            allVMs = vms
        } else if let rpc = try? rpcVMs(), case let .array(vms) = rpc {
            allVMs = vms
        } else {
            allVMs = []
        }

        if case let .object(report)? = health, case let .object(stack)? = report["stack"],
           case let .string(project)? = stack["project"]
        {
            projectName = project
            desired = allVMs.compactMap { vm in
                guard case let .object(item) = vm, case let .string(name)? = item["name"] else { return nil }
                return name
            }
        }

        var items: [JSONValue] = []
        var running = 0
        var starting = 0
        var stopping = 0
        var stopped = 0
        var missing = 0
        var other = 0

        for shortId in desired {
            let runtimeId = projectName.map { "\($0)/\(shortId)" } ?? shortId
            let found = findVM(in: allVMs, runtimeId: runtimeId, shortId: shortId)
            let resolvedId = stringField(found, "id") ?? stringField(found, "vm_id") ?? runtimeId
            let state = stringField(found, "state") ?? "missing"
            switch state {
            case "running": running += 1
            case "starting": starting += 1
            case "stopping": stopping += 1
            case "stopped": stopped += 1
            case "missing": missing += 1
            default: other += 1
            }
            items.append(
                .object([
                    "id": .string(resolvedId),
                    "name": .string(shortId),
                    "state": .string(state),
                    "present": .bool(found != nil),
                ])
            )
        }

        let desiredN = desired.count
        let phase = inventoryPhase(
            desired: desiredN,
            running: running,
            starting: starting,
            stopping: stopping,
            stopped: stopped,
            missing: missing,
            other: other
        )

        var stackId: JSONValue = .null
        if case let .object(diffSec)? = sections["diff"],
           case let .object(data)? = diffSec["data"],
           let sid = data["stack_id"]
        {
            stackId = sid
        }

        sections["apply"] = applySection(stackID: {
            if case let .string(id) = stackId { return id }
            return nil
        }(), inspect: inspectStack)

        sections["stack"] = .object([
            "ok": .bool(true),
            "data": .object([
                "phase": .string(phase),
                "label": .string(phaseLabel(phase)),
                "stack_id": stackId,
                "project": projectName.map(JSONValue.string) ?? .null,
                "vms": .object([
                    "desired": .number(Double(desiredN)),
                    "running": .number(Double(running)),
                    "starting": .number(Double(starting)),
                    "stopping": .number(Double(stopping)),
                    "stopped": .number(Double(stopped)),
                    "missing": .number(Double(missing)),
                    "other": .number(Double(other)),
                ]),
                "items": .array(items),
            ]),
        ])

        sections["ingress"] = .object([
            "ok": .bool(configText.contains("ingress:")),
            "data": ingressFromConfig(configText, project: projectName),
        ])

        return .object([
            "apiVersion": .string("vzctl.dev/v1"),
            "command": .string("status.bundle"),
            "observed_at": .string(ISO8601DateFormatter().string(from: Date())),
            "status": .string("ok"),
            "sections": .object(sections),
        ])
    }

    /// Reuse the canonical apply state without publishing journal payload/resources.
    /// An unavailable identity or database read must not masquerade as no journal.
    static func applySection(stackID: String?, inspect: ((String) throws -> StackStateRecord)?) -> JSONValue {
        let unavailable: JSONValue = .object(["ok": .bool(false), "data": .null])
        guard let stackID, !stackID.isEmpty, let inspect,
              let state = try? inspect(stackID) else { return unavailable }
        var journal: JSONValue = .null
        if let current = state.journal {
            guard current.stackID == stackID,
                  ["pending", "running", "failed"].contains(current.status) else { return unavailable }
            let mode = current.operationMode.flatMap { ["up", "apply", "down"].contains($0) ? $0 : nil }
            journal = .object([
                "id": .string(current.id), "generation": .number(Double(current.generation)),
                "status": .string(current.status), "step": .string(String(current.step.prefix(128))),
                "mode": mode.map(JSONValue.string) ?? .null,
                "error": current.error.map { .string(String($0.prefix(4096))) } ?? .null,
                "updated_at": .string(current.updatedAt),
            ])
        }
        return .object(["ok": .bool(true), "data": .object([
            "stack_id": .string(stackID), "journal": journal,
            "lease": state.lease.map { .object([
                "holder": .string(String($0.holder.prefix(256))),
                "expires_at": .string($0.expiresAtText),
            ]) } ?? .null,
        ])])
    }

    private static func section(_ result: (exitCode: Int32, stdout: String, stderr: String)?) -> JSONValue {
        guard let result else {
            return .object([
                "ok": .bool(false),
                "exit_code": .null,
                "data": .null,
                "stderr": .string("unavailable"),
            ])
        }
        return .object([
            "ok": .bool(result.exitCode == 0),
            "exit_code": .number(Double(result.exitCode)),
            "data": decodeJSON(result.stdout) ?? .null,
            "stderr": .string(result.stderr.trimmingCharacters(in: .whitespacesAndNewlines)),
        ])
    }

    private static func decodeJSON(_ text: String) -> JSONValue? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let data = trimmed.data(using: .utf8) else { return nil }
        return try? JSONDecoder().decode(JSONValue.self, from: data)
    }

    private static func desiredVMIds(from yaml: String) -> [String] {
        var ids: [String] = []
        var seenVMs = false
        for rawLine in yaml.split(separator: "\n") {
            let line = String(rawLine)
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed == "vms:" || trimmed.hasPrefix("vms:") {
                seenVMs = true
                continue
            }
            guard seenVMs else { continue }
            // Sibling key at 2-space indent (spec-level) ends the block.
            if line.range(of: #"^[a-zA-Z]"#, options: .regularExpression) != nil {
                break
            }
            if line.range(of: #"^\s{2}[a-zA-Z]"#, options: .regularExpression) != nil,
               !line.hasPrefix("    ")
            {
                break
            }
            if let regex = try? NSRegularExpression(pattern: #"^\s{4}([A-Za-z0-9_-]+):\s*(#.*)?$"#),
               let match = regex.firstMatch(in: line, range: NSRange(line.startIndex..., in: line)),
               let nameRange = Range(match.range(at: 1), in: line)
            {
                ids.append(String(line[nameRange]))
            }
        }
        return ids
    }

    private static func projectName(from yaml: String) -> String? {
        for rawLine in yaml.split(separator: "\n") {
            let line = String(rawLine).trimmingCharacters(in: .whitespaces)
            if line.hasPrefix("project:") {
                let value = line.dropFirst("project:".count).trimmingCharacters(in: .whitespaces)
                let cleaned = value.trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
                return cleaned.isEmpty ? nil : cleaned
            }
            if line.hasPrefix("name:") {
                // metadata.name fallback later
            }
        }
        // metadata.name
        var inMetadata = false
        for rawLine in yaml.split(separator: "\n") {
            let line = String(rawLine)
            if line.trimmingCharacters(in: .whitespaces).hasPrefix("metadata:") {
                inMetadata = true
                continue
            }
            if inMetadata {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.hasPrefix("name:") {
                    let value = trimmed.dropFirst("name:".count).trimmingCharacters(in: .whitespaces)
                    let cleaned = value.trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
                    return cleaned.isEmpty ? nil : cleaned
                }
                if !line.hasPrefix(" ") && !line.hasPrefix("\t") && !trimmed.isEmpty {
                    break
                }
            }
        }
        return nil
    }

    private static func findVM(in vms: [JSONValue], runtimeId: String, shortId: String) -> [String: JSONValue]? {
        for vm in vms {
            guard case let .object(obj) = vm else { continue }
            let id = stringField(obj, "id") ?? stringField(obj, "vm_id")
            if id == runtimeId || id == shortId { return obj }
        }
        return nil
    }

    private static func stringField(_ obj: [String: JSONValue]?, _ key: String) -> String? {
        guard let obj, case let .string(value)? = obj[key] else { return nil }
        return value
    }

    private static func inventoryPhase(
        desired: Int,
        running: Int,
        starting: Int,
        stopping: Int,
        stopped: Int,
        missing: Int,
        other: Int
    ) -> String {
        if desired == 0 { return "down" }
        if starting > 0 { return "starting" }
        if stopping > 0 { return "stopping" }
        if running == desired { return "running" }
        if running > 0 { return "partial" }
        if missing == desired || stopped == desired { return "down" }
        if other > 0 { return "failed" }
        return "unknown"
    }

    private static func phaseLabel(_ phase: String) -> String {
        switch phase {
        case "down": return "Down"
        case "starting": return "Starting"
        case "stopping": return "Stopping"
        case "reconciling": return "Up (Reconciling)"
        case "running": return "Up (Running)"
        case "partial": return "Up (Partial)"
        case "failed": return "Failed"
        default: return "Unknown"
        }
    }

    private static func ingressFromConfig(_ yaml: String, project: String?) -> JSONValue {
        var inIngress = false
        var ingressIndent = 0
        var inRoutes = false
        var routesIndent = 0
        var enabled = true
        var hostAliases = true
        var httpsPort: UInt16 = 443
        var routeMaps: [[String: JSONValue]] = []
        var current: [String: JSONValue]?

        func flushRoute() {
            guard let map = current,
                  case let .string(host) = map["host"],
                  !host.isEmpty
            else {
                current = nil
                return
            }
            routeMaps.append(map)
            current = nil
        }

        for rawLine in yaml.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(rawLine)
            let trimmedEnd = line.trimmingCharacters(in: .newlines)
            if trimmedEnd.trimmingCharacters(in: .whitespaces).isEmpty {
                continue
            }
            let indent = line.prefix(while: { $0 == " " }).count
            let content = trimmedEnd.trimmingCharacters(in: .whitespaces)
            if content.hasPrefix("#") {
                continue
            }

            if !inIngress {
                if content == "ingress:" || content.hasPrefix("ingress:") {
                    inIngress = true
                    ingressIndent = indent
                }
                continue
            }

            if indent <= ingressIndent {
                break
            }

            if inRoutes {
                let routeBase = routesIndent
                if indent <= routeBase {
                    flushRoute()
                    inRoutes = false
                } else if content.hasPrefix("- ") {
                    flushRoute()
                    var map: [String: JSONValue] = [:]
                    let rest = String(content.dropFirst(2)).trimmingCharacters(in: .whitespaces)
                    if let colon = rest.firstIndex(of: ":") {
                        let key = String(rest[..<colon]).trimmingCharacters(in: .whitespaces)
                        let value = yamlScalar(String(rest[rest.index(after: colon)...]))
                        ingressRouteField(key: key, value: value, into: &map)
                    }
                    current = map
                    continue
                } else if var map = current {
                    if let colon = content.firstIndex(of: ":") {
                        let key = String(content[..<colon]).trimmingCharacters(in: .whitespaces)
                        let value = yamlScalar(String(content[content.index(after: colon)...]))
                        ingressRouteField(key: key, value: value, into: &map)
                        current = map
                    }
                    continue
                }
            }

            if content == "routes:" || content.hasPrefix("routes:") {
                flushRoute()
                inRoutes = true
                routesIndent = indent
                continue
            }

            if let colon = content.firstIndex(of: ":") {
                let key = String(content[..<colon]).trimmingCharacters(in: .whitespaces)
                let value = yamlScalar(String(content[content.index(after: colon)...]))
                switch key {
                case "enabled":
                    enabled = yamlBool(value, default: true)
                case "hostAliases":
                    hostAliases = yamlBool(value, default: true)
                case "httpsPort":
                    if let port = UInt16(value) {
                        httpsPort = port
                    }
                default:
                    break
                }
            }
        }
        flushRoute()

        var seenHosts = Set<String>()
        var routes: [JSONValue] = []
        for map in routeMaps {
            guard case let .string(host) = map["host"], !host.isEmpty else { continue }
            if seenHosts.contains(host) {
                continue
            }
            seenHosts.insert(host)
            let to: JSONValue = map["to"] ?? .string("")
            let requires: JSONValue = map["requires"] ?? .array([])
            let url = httpsURL(host: host, port: httpsPort)
            var route: [String: JSONValue] = [
                "host": .string(host),
                "url": .string(url),
                "to": to,
                "requires": requires,
            ]
            if hostAliases, let short = host.split(separator: ".").first {
                let aliasHost = "\(short).localhost"
                route["alias"] = .object([
                    "host": .string(aliasHost),
                    "url": .string(httpsURL(host: aliasHost, port: httpsPort)),
                ])
            } else {
                route["alias"] = .null
            }
            routes.append(.object(route))
        }

        return .object([
            "enabled": .bool(enabled),
            "https_port": .number(Double(httpsPort)),
            "host_aliases": .bool(hostAliases),
            "project": project.map(JSONValue.string) ?? .null,
            "routes": .array(routes),
        ])
    }

    private static func ingressRouteField(key: String, value: String, into map: inout [String: JSONValue]) {
        switch key {
        case "host", "to":
            map[key] = .string(value)
        case "requires":
            map["requires"] = parseRequiresInline(value)
        default:
            break
        }
    }

    private static func yamlScalar(_ raw: String) -> String {
        raw.trimmingCharacters(in: .whitespacesAndNewlines)
            .trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
    }

    private static func yamlBool(_ raw: String, default defaultValue: Bool) -> Bool {
        switch yamlScalar(raw).lowercased() {
        case "true", "yes", "on": return true
        case "false", "no", "off": return false
        default: return defaultValue
        }
    }

    private static func parseRequiresInline(_ raw: String) -> JSONValue {
        let trimmed = raw.trimmingCharacters(in: .whitespaces)
        if trimmed.hasPrefix("["), trimmed.hasSuffix("]") {
            let inner = trimmed.dropFirst().dropLast()
            let items = inner.split(separator: ",")
                .map { yamlScalar(String($0)) }
                .filter { !$0.isEmpty }
                .map(JSONValue.string)
            return .array(items)
        }
        let one = yamlScalar(trimmed)
        return one.isEmpty ? .array([]) : .array([.string(one)])
    }

    private static func httpsURL(host: String, port: UInt16) -> String {
        port == 443 ? "https://\(host)" : "https://\(host):\(port)"
    }
}
