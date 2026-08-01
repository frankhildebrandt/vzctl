import Foundation
import VzDaemonKit

enum RestStatusBundle {
    static func build(path: String, jobs: RestJobRunner, rpcVMs: () throws -> JSONValue) throws -> JSONValue {
        var sections: [String: JSONValue] = [:]

        sections["dns"] = section(try? jobs.runSync(arguments: ["dns", "status", "--format", "json"]))
        sections["certs"] = section(try? jobs.runSync(arguments: ["certs", "fingerprint", "--format", "json"]))
        sections["oidc"] = section(try? jobs.runSync(arguments: ["oidc", "status", "--format", "json"]))
        sections["diff"] = section(try? jobs.runSync(arguments: ["diff", "-C", path, "--format", "json"]))

        let configURL = RestStackStore.configURL(for: path)
        let configText = (try? String(contentsOf: configURL, encoding: .utf8)) ?? ""
        let desired = desiredVMIds(from: configText)
        let projectName = projectName(from: configText)

        let vmListResult = try? jobs.runSync(arguments: ["vm", "list", "--format", "json"])
        let allVMs: [JSONValue]
        if let stdout = vmListResult?.stdout,
           let parsed = decodeJSON(stdout),
           case let .object(obj) = parsed,
           case let .array(vms)? = obj["vms"]
        {
            allVMs = vms
        } else if let rpc = try? rpcVMs(), case let .array(vms) = rpc {
            allVMs = vms
        } else {
            allVMs = []
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
            "status": .string("ok"),
            "sections": .object(sections),
        ])
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
        let enabled = yaml.contains("ingress:") && yaml.contains("enabled: true")
        // Best-effort host extraction
        var routes: [JSONValue] = []
        let pattern = #"host:\s*([^\s#]+)"#
        if let regex = try? NSRegularExpression(pattern: pattern) {
            let range = NSRange(yaml.startIndex..., in: yaml)
            for match in regex.matches(in: yaml, range: range) {
                if let r = Range(match.range(at: 1), in: yaml) {
                    let host = String(yaml[r]).trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
                    routes.append(
                        .object([
                            "host": .string(host),
                            "url": .string("https://\(host)"),
                        ])
                    )
                }
            }
        }
        return .object([
            "enabled": .bool(enabled),
            "project": project.map(JSONValue.string) ?? .null,
            "routes": .array(routes),
        ])
    }
}
