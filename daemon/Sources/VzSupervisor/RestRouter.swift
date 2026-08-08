import Darwin
import Foundation
import VzDaemonKit

final class RestRouter: @unchecked Sendable {
    weak var server: SupervisorServer?
    let jobs: RestJobRunner
    let database: StateDatabase
    let stateDirectory: URL

    init(jobs: RestJobRunner, database: StateDatabase, stateDirectory: URL) {
        self.jobs = jobs
        self.database = database
        self.stateDirectory = stateDirectory
    }

    /// Returns true if the connection was taken over for streaming (SSE).
    func handleStreaming(request: RestHTTPRequest, client: Int32) -> Bool {
        let segments = RestHTTP.pathSegments(request.path)
        guard request.method == "GET", segments.count >= 2, segments[0] == "v1" else {
            return false
        }
        if segments.count == 2, segments[1] == "events" {
            streamEvents(request: request, client: client)
            return true
        }
        if segments.count == 4, segments[1] == "jobs", segments[3] == "log" {
            streamJobLog(jobId: segments[2], client: client)
            return true
        }
        return false
    }

    func handle(_ request: RestHTTPRequest) -> RestHTTPResponse {
        do {
            return try route(request)
        } catch let error as RestRouteError {
            return (try? RestHTTPResponse.error(
                error.status,
                code: error.code,
                message: error.message,
                details: error.details
            )) ?? RestHTTPResponse.text(error.status, error.message)
        } catch {
            return (try? RestHTTPResponse.error(
                500,
                code: .internalError,
                message: String(describing: error)
            )) ?? RestHTTPResponse.text(500, String(describing: error))
        }
    }

    private func route(_ request: RestHTTPRequest) throws -> RestHTTPResponse {
        let segments = RestHTTP.pathSegments(request.path)
        guard segments.first == "v1" else {
            throw RestRouteError(404, .notFound, "unknown path")
        }
        let rest = Array(segments.dropFirst())
        let method = request.method

        if rest == ["health"], method == "GET" {
            return try rpcOK("daemon.health")
        }
        if rest == ["version"], method == "GET" {
            return try rpcOK("daemon.version")
        }

        if rest.first == "jobs" {
            return try routeJobs(method: method, rest: rest)
        }
        if rest.first == "stacks" {
            return try routeStacks(request: request, method: method, rest: rest)
        }
        if rest.first == "vms" {
            return try routeVMs(request: request, method: method, rest: rest)
        }
        if rest.first == "nets" {
            return try routeNets(request: request, method: method, rest: rest)
        }
        if rest == ["ports"], method == "GET" {
            return try rpcOK("port.list")
        }
        if rest.first == "images" {
            return try routeImages(request: request, method: method, rest: rest)
        }
        if rest.first == "projects" {
            return try routeProjects(request: request, method: method, rest: rest)
        }
        if rest.first == "doctor", rest.count == 1, method == "GET" {
            return try workerJSON(["doctor", "--format", "json"])
        }
        if rest.first == "certs" {
            return try routeCerts(method: method, rest: rest)
        }
        if rest.first == "dns" {
            return try routeDNS(request: request, method: method, rest: rest)
        }
        if rest.first == "oidc" {
            return try routeOIDC(request: request, method: method, rest: rest)
        }
        if rest == ["host", "reboot"], method == "POST" {
            return try hostReboot()
        }

        throw RestRouteError(404, .notFound, "unknown path /\(request.path)")
    }

    // MARK: - Jobs

    private func routeJobs(method: String, rest: [String]) throws -> RestHTTPResponse {
        guard rest.count == 2, method == "GET" else {
            throw RestRouteError(404, .notFound, "job route")
        }
        guard let job = jobs.get(rest[1]) else {
            throw RestRouteError(404, .notFound, "job not found")
        }
        return try RestHTTPResponse.jsonValue(200, job.json)
    }

    // MARK: - Stacks

    private func routeStacks(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest.count == 1 {
            if method == "GET" {
                let rows = try database.listStacks().map(\.json)
                return try RestHTTPResponse.jsonValue(200, .object(["stacks": .array(rows)]))
            }
            if method == "POST" {
                return try createStack(request)
            }
        }
        guard rest.count >= 2 else { throw RestRouteError(404, .notFound, "stack route") }
        let stackId = rest[1]
        guard let stack = try database.getStack(id: stackId) else {
            throw RestRouteError(404, .notFound, "stack not found")
        }

        if rest.count == 2 {
            if method == "GET" {
                return try RestHTTPResponse.jsonValue(200, stack.json)
            }
            if method == "DELETE" {
                try database.deleteStack(id: stackId)
                return RestHTTPResponse.empty(204)
            }
        }

        if rest.count == 3 {
            switch (rest[2], method) {
            case ("config", "GET"):
                let text = try RestStackStore.readText(at: RestStackStore.configURL(for: stack.path))
                return RestHTTPResponse.text(200, text, contentType: "text/yaml; charset=utf-8")
            case ("config", "PUT"):
                let content = try bodyText(request)
                try RestStackStore.writeText(content, at: RestStackStore.configURL(for: stack.path))
                return try RestHTTPResponse.jsonValue(200, .object(["ok": .bool(true)]))
            case ("diagram", "GET"):
                let text = try RestStackStore.readText(at: RestStackStore.diagramURL(for: stack.path))
                return RestHTTPResponse.text(200, text, contentType: "application/json; charset=utf-8")
            case ("diagram", "PUT"):
                let content = try bodyText(request)
                try RestStackStore.writeText(content, at: RestStackStore.diagramURL(for: stack.path))
                return try RestHTTPResponse.jsonValue(200, .object(["ok": .bool(true)]))
            case ("validate", "POST"):
                return try workerJSON(["validate", "-C", stack.path, "--format", "json"])
            case ("diff", "GET"):
                return try workerJSON(["diff", "-C", stack.path, "--format", "json"])
            case ("status", "GET"):
                return try RestHTTPResponse.jsonValue(
                    200,
                    try RestStatusBundle.build(
                        path: stack.path,
                        jobs: jobs,
                        rpcVMs: { try self.rpcResult("vm.list") }
                    )
                )
            case ("up", "POST"):
                return try startStackJob(kind: "up", path: stack.path, body: request.body)
            case ("apply", "POST"):
                return try startStackJob(kind: "apply", path: stack.path, body: request.body)
            case ("down", "POST"):
                return try startStackJob(kind: "down", path: stack.path, body: request.body)
            default:
                break
            }
        }
        throw RestRouteError(404, .notFound, "stack sub-route")
    }

    private func createStack(_ request: RestHTTPRequest) throws -> RestHTTPResponse {
        let obj = try bodyObject(request)
        guard case let .string(path)? = obj["path"], !path.isEmpty else {
            throw RestRouteError(400, .badRequest, "path required")
        }
        let name: String
        if case let .string(n)? = obj["name"], !n.isEmpty {
            name = n
        } else {
            name = URL(fileURLWithPath: path).lastPathComponent
        }
        let id: String
        if case let .string(explicit)? = obj["id"], !explicit.isEmpty {
            id = explicit
        } else if let existing = try database.getStackByPath(path) {
            id = existing.id
        } else {
            id = RestStackStore.stackId(from: path, explicit: nil)
        }
        let createScaffold = optionalBool(obj["create"]) ?? false
        if createScaffold {
            try FileManager.default.createDirectory(
                atPath: path,
                withIntermediateDirectories: true
            )
            let config = RestStackStore.configURL(for: path)
            if !FileManager.default.fileExists(atPath: config.path) {
                let yaml = """
                apiVersion: hypernetwork/v1
                kind: Environment
                metadata:
                  name: \(name)
                spec:
                  networks: {}
                  vms: {}
                """
                try RestStackStore.writeText(yaml, at: config)
            }
        }
        let record = StackRegistryRecord(
            id: id,
            path: path,
            name: name,
            openedAt: ISO8601DateFormatter().string(from: Date())
        )
        try database.upsertStack(record)
        return try RestHTTPResponse.jsonValue(201, record.json)
    }

    private func startStackJob(kind: String, path: String, body: Data) throws -> RestHTTPResponse {
        let obj = (try? JSONDecoder().decode(JSONValue.self, from: body)).flatMap { value -> [String: JSONValue]? in
            if case let .object(o) = value { return o }
            return nil
        } ?? [:]
        var args = [kind, "-C", path, "--format", "json", "--progress", "plain"]
        if kind == "down" {
            if optionalBool(obj["purge"]) == true { args.append("--purge") }
        } else {
            if optionalBool(obj["force"]) == true { args.append("--force") }
            if optionalBool(obj["resume"]) == true { args.append("--resume") }
            if optionalBool(obj["abort"]) == true { args.append("--abort") }
        }
        let jobId = jobs.start(kind: "stack.\(kind)", arguments: args)
        return try RestHTTPResponse.jsonValue(202, .object(["jobId": .string(jobId)]))
    }

    // MARK: - VMs

    private func routeVMs(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest.count == 1 {
            if method == "GET" {
                // Prefer worker list (merges bundles); fall back to runtime RPC.
                if let listed = try? workerJSON(["vm", "list", "--format", "json"]) {
                    return listed
                }
                return try rpcOK("vm.list")
            }
            if method == "POST" {
                return try createVM(request)
            }
        }
        guard rest.count >= 2 else { throw RestRouteError(404, .notFound, "vm route") }
        let vmId = rest[1]

        if rest.count == 2 {
            switch method {
            case "GET":
                return try workerJSON(["vm", "inspect", vmId, "--format", "json"])
            case "PATCH":
                return try modifyVM(vmId: vmId, request: request)
            case "DELETE":
                var args = ["vm", "delete", vmId, "--format", "json"]
                if request.query["force"] == "1" || request.query["force"] == "true" {
                    args.append("--force")
                }
                return try workerJSON(args)
            default:
                break
            }
        }

        if rest.count == 3 {
            switch (rest[2], method) {
            case ("start", "POST"):
                return try workerJSON(["vm", "start", vmId, "--format", "json"])
            case ("stop", "POST"):
                return try workerJSON(["vm", "stop", vmId, "--format", "json"])
            case ("mounts", "GET"):
                return try workerJSON(["vm", "mounts", vmId, "--format", "json"])
            case ("mounts", "POST"):
                return try addMount(vmId: vmId, request: request)
            default:
                break
            }
        }

        if rest.count == 4, rest[2] == "mounts", method == "DELETE" {
            return try workerJSON([
                "vm", "unmount", vmId, "--tag", rest[3], "--format", "json",
            ])
        }

        throw RestRouteError(404, .notFound, "vm sub-route")
    }

    private func createVM(_ request: RestHTTPRequest) throws -> RestHTTPResponse {
        let obj = try bodyObject(request)
        guard case let .string(id)? = obj["id"], !id.isEmpty else {
            throw RestRouteError(400, .badRequest, "id required")
        }
        guard case let .string(from)? = obj["from"], !from.isEmpty else {
            throw RestRouteError(400, .badRequest, "from required")
        }
        var args = ["vm", "create", id, "--from", from, "--format", "json"]
        if case let .number(disk)? = obj["diskGib"] ?? obj["dataDiskGib"] {
            args += ["--disk", String(Int(disk))]
        }
        if case let .number(cpus)? = obj["cpus"] {
            args += ["--cpus", String(Int(cpus))]
        }
        if case let .string(memory)? = obj["memory"] {
            args += ["--memory", memory]
        }
        if case let .string(network)? = obj["network"] {
            args += ["--network", network]
        }
        if case let .string(project)? = obj["project"] {
            args += ["--project", project]
        }
        if case let .string(password)? = obj["rootPassword"] {
            args += ["--root-password", password]
        }
        if case let .array(roles)? = obj["roles"] {
            for role in roles {
                if case let .string(r) = role { args += ["--role", r] }
            }
        }
        return try workerJSON(args)
    }

    private func modifyVM(vmId: String, request: RestHTTPRequest) throws -> RestHTTPResponse {
        let obj = try bodyObject(request)
        var args = ["vm", "modify", vmId, "--format", "json"]
        if case let .number(cpus)? = obj["cpus"] {
            args += ["--cpus", String(Int(cpus))]
        }
        if case let .string(memory)? = obj["memory"] {
            args += ["--memory", memory]
        }
        return try workerJSON(args)
    }

    private func addMount(vmId: String, request: RestHTTPRequest) throws -> RestHTTPResponse {
        let obj = try bodyObject(request)
        guard case let .string(source)? = obj["source"],
              case let .string(target)? = obj["target"]
        else {
            throw RestRouteError(400, .badRequest, "source and target required")
        }
        var args = [
            "vm", "mount", vmId, "--source", source, "--target", target, "--format", "json",
        ]
        if case let .string(tag)? = obj["tag"] { args += ["--tag", tag] }
        if optionalBool(obj["readOnly"]) == true { args.append("--ro") }
        return try workerJSON(args)
    }

    // MARK: - Nets

    private func routeNets(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest.count == 1 {
            if method == "GET" { return try rpcOK("net.list") }
            if method == "POST" {
                let obj = try bodyObject(request)
                return try rpcOK("net.create", params: .object(obj))
            }
        }
        if rest.count == 2, rest[1] == "default" {
            if method == "GET" { return try rpcOK("net.default.show") }
            if method == "PUT" {
                let obj = try bodyObject(request)
                return try rpcOK("net.default.set", params: .object(obj))
            }
        }
        guard rest.count >= 2 else { throw RestRouteError(404, .notFound, "net route") }
        let name = rest[1]
        if rest.count == 2, method == "DELETE" {
            return try rpcOK("net.delete", params: .object(["name": .string(name)]))
        }
        if rest.count == 3, rest[2] == "attach", method == "POST" {
            var obj = try bodyObject(request)
            obj["network"] = .string(name)
            return try rpcOK("net.attach", params: .object(obj))
        }
        if rest.count == 3, rest[2] == "detach", method == "POST" {
            var obj = try bodyObject(request)
            obj["network"] = .string(name)
            return try rpcOK("net.detach", params: .object(obj))
        }
        throw RestRouteError(404, .notFound, "net sub-route")
    }

    // MARK: - Images

    private func routeImages(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest.count == 1, method == "GET" {
            return try workerJSON(["image", "list", "--format", "json"])
        }
        guard rest.count == 3, method == "POST" else {
            throw RestRouteError(404, .notFound, "image route")
        }
        let alias = rest[1]
        let action = rest[2]
        guard ["pull", "bake", "seal"].contains(action) else {
            throw RestRouteError(404, .notFound, "unknown image action")
        }
        var arguments = ["image", action, alias]
        if action == "bake" || action == "seal" {
            let body = try bodyObject(request)
            guard case let .string(tag)? = body["tag"], !tag.isEmpty else {
                throw RestRouteError(400, .badRequest, "image \(action) requires body.tag")
            }
            arguments += ["--tag", tag]
        }
        arguments += ["--format", "json"]
        let jobId = jobs.start(
            kind: "image.\(action)",
            arguments: arguments
        )
        return try RestHTTPResponse.jsonValue(202, .object(["jobId": .string(jobId)]))
    }

    // MARK: - Docker / projects

    private func routeProjects(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        guard rest.count >= 2 else { throw RestRouteError(404, .notFound, "project route") }
        let project = rest[1]

        if rest.count >= 3, rest[2] == "containers" {
            if rest.count == 3 {
                if method == "GET" {
                    return try workerJSON([
                        "docker", "ps", "--project", project, "--all", "--format", "json",
                    ])
                }
                if method == "POST" {
                    let obj = try bodyObject(request)
                    var args = ["docker", "run", "--project", project, "--format", "json"]
                    if case let .string(image)? = obj["image"] {
                        args.append(image)
                    }
                    if case let .array(cmd)? = obj["cmd"] {
                        for part in cmd {
                            if case let .string(s) = part { args.append(s) }
                        }
                    }
                    return try workerJSON(args)
                }
            }
            if rest.count == 4 {
                let containerId = rest[3]
                if method == "GET" {
                    return try workerJSON([
                        "docker", "inspect", "--project", project, containerId, "--format", "json",
                    ])
                }
            }
            if rest.count == 5, method == "POST" {
                let containerId = rest[3]
                let action = rest[4]
                guard ["start", "stop", "restart"].contains(action) else {
                    throw RestRouteError(404, .notFound, "container action")
                }
                return try workerJSON([
                    "docker", action, "--project", project, containerId, "--format", "json",
                ])
            }
        }

        if rest.count == 4, rest[2] == "oidc", rest[3] == "secret", method == "PUT" {
            let content = try bodyText(request)
            let secretURL = stateDirectory
                .appendingPathComponent("projects", isDirectory: true)
                .appendingPathComponent(project, isDirectory: true)
                .appendingPathComponent("oidc", isDirectory: true)
                .appendingPathComponent("uplink-secret")
            try FileManager.default.createDirectory(
                at: secretURL.deletingLastPathComponent(),
                withIntermediateDirectories: true
            )
            try content.write(to: secretURL, atomically: true, encoding: .utf8)
            chmod(secretURL.path, 0o600)
            return try RestHTTPResponse.jsonValue(200, .object(["ok": .bool(true)]))
        }

        throw RestRouteError(404, .notFound, "project sub-route")
    }

    // MARK: - Certs / DNS / OIDC / Host

    private func routeCerts(method: String, rest: [String]) throws -> RestHTTPResponse {
        if rest == ["certs", "fingerprint"], method == "GET" {
            return try workerJSON(["certs", "fingerprint", "--format", "json"])
        }
        if rest == ["certs", "ca", "init"], method == "POST" {
            return try workerJSON(["certs", "ca", "init", "--format", "json"])
        }
        if rest == ["certs", "ca", "install"], method == "POST" {
            return try workerJSON(["certs", "ca", "install", "--format", "json"])
        }
        throw RestRouteError(404, .notFound, "certs route")
    }

    private func routeDNS(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest == ["dns", "status"], method == "GET" {
            return try rpcOK("dns.status")
        }
        if rest == ["dns", "bind-helper"], method == "POST" {
            var args = ["dns", "install-bind-helper", "--format", "json"]
            // osascript elevation runs as root without SUDO_UID — pin calling user.
            args += ["--allow-uid", String(getuid())]
            return try workerJSONPrivileged(args)
        }
        if rest == ["dns", "resolver"] {
            switch method {
            case "POST":
                return try workerJSONPrivileged(dnsResolverArgs(install: true, request: request))
            case "DELETE":
                return try workerJSONPrivileged(dnsResolverArgs(install: false, request: request))
            default:
                break
            }
        }
        throw RestRouteError(404, .notFound, "dns route")
    }

    /// `POST/DELETE /v1/dns/resolver` — body or query: `config` and/or `project`.
    private func dnsResolverArgs(install: Bool, request: RestHTTPRequest) throws -> [String] {
        let obj = (try? bodyObject(request)) ?? [:]
        let config: String? = {
            if case let .string(value)? = obj["config"], !value.isEmpty { return value }
            if let value = request.query["config"], !value.isEmpty { return value }
            return nil
        }()
        let project: String? = {
            if case let .string(value)? = obj["project"], !value.isEmpty { return value }
            if let value = request.query["project"], !value.isEmpty { return value }
            return nil
        }()
        guard config != nil || project != nil else {
            throw RestRouteError(
                400,
                .badRequest,
                "dns resolver requires config and/or project"
            )
        }
        var args = [
            "dns",
            install ? "install-resolver" : "uninstall-resolver",
            "--format",
            "json",
        ]
        if let config {
            args += ["--config", config]
        }
        if let project {
            args += ["--project", project]
        }
        return args
    }

    private func routeOIDC(
        request: RestHTTPRequest,
        method: String,
        rest: [String]
    ) throws -> RestHTTPResponse {
        if rest == ["oidc", "status"], method == "GET" {
            return try workerJSON(["oidc", "status", "--format", "json"])
        }
        if rest == ["oidc", "uplink"] {
            let url = stateDirectory
                .appendingPathComponent("config", isDirectory: true)
                .appendingPathComponent("oidc-uplink.yaml")
            if method == "GET" {
                if FileManager.default.fileExists(atPath: url.path) {
                    let text = try String(contentsOf: url, encoding: .utf8)
                    return RestHTTPResponse.text(200, text, contentType: "text/yaml; charset=utf-8")
                }
                return RestHTTPResponse.text(200, "", contentType: "text/yaml; charset=utf-8")
            }
            if method == "PUT" {
                let content = try bodyText(request)
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(),
                    withIntermediateDirectories: true
                )
                try content.write(to: url, atomically: true, encoding: .utf8)
                return try RestHTTPResponse.jsonValue(200, .object(["ok": .bool(true)]))
            }
        }
        throw RestRouteError(404, .notFound, "oidc route")
    }

    private func hostReboot() throws -> RestHTTPResponse {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", "tell application \"System Events\" to restart"]
        try process.run()
        process.waitUntilExit()
        if process.terminationStatus == 0 {
            return try RestHTTPResponse.jsonValue(200, .object(["ok": .bool(true)]))
        }
        throw RestRouteError(500, .internalError, "host reboot failed or cancelled")
    }

    // MARK: - Streaming

    private func streamEvents(request: RestHTTPRequest, client: Int32) {
        guard let server else {
            let response = (try? RestHTTPResponse.error(500, code: .internalError, message: "no server"))
                ?? RestHTTPResponse.text(500, "no server")
            _ = writeData(RestHTTP.encodeResponse(response), to: client)
            return
        }
        let filterExpr = request.query["filter"]
        guard let filter = try? EventFilter(filterExpr) else {
            let response = (try? RestHTTPResponse.error(400, code: .badRequest, message: "invalid filter"))
                ?? RestHTTPResponse.text(400, "invalid filter")
            _ = writeData(RestHTTP.encodeResponse(response), to: client)
            return
        }

        let preamble = Data(
            """
            HTTP/1.1 200 OK\r
            Content-Type: text/event-stream\r
            Cache-Control: no-cache\r
            Connection: keep-alive\r
            \r

            """.utf8
        )
        guard writeData(preamble, to: client) else { return }

        let handlerId = server.addEventListener(filter: filter) { envelope in
            guard let encoded = try? JSONEncoder().encode(envelope),
                  let line = String(data: encoded, encoding: .utf8)
            else { return }
            let frame = Data("data: \(line)\n\n".utf8)
            _ = writeData(frame, to: client)
        }
        defer { server.removeEventListener(handlerId) }

        var byte: UInt8 = 0
        while Darwin.read(client, &byte, 1) > 0 {}
    }

    private func streamJobLog(jobId: String, client: Int32) {
        let preamble = Data(
            """
            HTTP/1.1 200 OK\r
            Content-Type: text/event-stream\r
            Cache-Control: no-cache\r
            Connection: keep-alive\r
            \r

            """.utf8
        )
        guard writeData(preamble, to: client) else { return }

        var sent = 0
        while true {
            let lines = jobs.logLines(jobId)
            if sent < lines.count {
                for line in lines[sent...] {
                    let frame = Data("data: \(line)\n\n".utf8)
                    guard writeData(frame, to: client) else { return }
                }
                sent = lines.count
            }
            if let job = jobs.get(jobId),
               job.status == .succeeded || job.status == .failed
            {
                let done = Data("event: done\ndata: \(job.status.rawValue)\n\n".utf8)
                _ = writeData(done, to: client)
                return
            }
            Thread.sleep(forTimeInterval: 0.25)
        }
    }

    // MARK: - Helpers

    private func rpcOK(_ method: String, params: JSONValue? = nil) throws -> RestHTTPResponse {
        let result = try rpcResult(method, params: params)
        return try RestHTTPResponse.jsonValue(200, result)
    }

    private func rpcResult(_ method: String, params: JSONValue? = nil) throws -> JSONValue {
        guard let server else {
            throw RestRouteError(500, .internalError, "supervisor not ready")
        }
        let response = server.dispatchRPC(method: method, params: params)
        if let error = response.error {
            throw RestRouteError(400, .failedPrecondition, error.message)
        }
        return response.result ?? .null
    }

    private func workerJSON(_ args: [String]) throws -> RestHTTPResponse {
        let value = try workerResult(args)
        return try RestHTTPResponse.jsonValue(200, value)
    }

    /// DNS resolver / bind-helper writes need root; retry via macOS Admin dialog.
    private func workerJSONPrivileged(_ args: [String]) throws -> RestHTTPResponse {
        let value = try workerResult(args)
        if !dnsEnvelopeNeedsElevation(value) {
            return try RestHTTPResponse.jsonValue(200, value)
        }
        let elevated = try runElevatedVzctl(args)
        return try RestHTTPResponse.jsonValue(200, elevated)
    }

    private func dnsEnvelopeNeedsElevation(_ value: JSONValue) -> Bool {
        guard case let .object(obj) = value else { return false }
        if case let .string(status)? = obj["status"], status == "ok" {
            return false
        }
        if case let .number(code)? = obj["exit_code"], code == 19 {
            return true
        }
        let message: String = {
            if case let .object(summary)? = obj["summary"],
               case let .string(text)? = summary["message"]
            {
                return text
            }
            if case let .string(text)? = obj["message"] { return text }
            return String(describing: value)
        }()
        return message.contains("Permission denied")
            || message.contains("run this command with sudo")
            || message.contains("os error 13")
            || message.contains("launchctl bootstrap failed")
            || message.contains("/Library/LaunchDaemons")
    }

    private func runElevatedVzctl(_ args: [String]) throws -> JSONValue {
        let vzctl = try resolveVzctlPath()
        let shell = ([vzctl] + args).map(shellQuote).joined(separator: " ")
        let escaped = shell
            .replacingOccurrences(of: "\\", with: "\\\\")
            .replacingOccurrences(of: "\"", with: "\\\"")
        let script = "do shell script \"\(escaped)\" with administrator privileges"
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        process.arguments = ["-e", script]
        let out = Pipe()
        let err = Pipe()
        process.standardOutput = out
        process.standardError = err
        try process.run()
        process.waitUntilExit()
        let stdout = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        let stderr = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        if process.terminationStatus != 0 {
            let detail = stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            throw RestRouteError(
                500,
                .internalError,
                detail.isEmpty
                    ? "Admin elevation failed or cancelled for: \(args.joined(separator: " "))"
                    : detail
            )
        }
        let trimmed = stdout.trimmingCharacters(in: .whitespacesAndNewlines)
        if let data = trimmed.data(using: .utf8),
           let value = try? JSONDecoder().decode(JSONValue.self, from: data)
        {
            return value
        }
        // osascript may wrap output; recover JSON object if present.
        if let start = trimmed.firstIndex(of: "{"),
           let end = trimmed.lastIndex(of: "}"),
           start < end
        {
            let slice = String(trimmed[start ... end])
            if let data = slice.data(using: .utf8),
               let value = try? JSONDecoder().decode(JSONValue.self, from: data)
            {
                return value
            }
        }
        return .object([
            "apiVersion": .string("vzctl.dev/v1"),
            "status": .string("ok"),
            "exit_code": .number(0),
            "summary": .object(["message": .string(trimmed.isEmpty ? "elevated ok" : trimmed)]),
        ])
    }

    private func resolveVzctlPath() throws -> String {
        if let env = ProcessInfo.processInfo.environment["VZCTL_BIN"],
           FileManager.default.isExecutableFile(atPath: env)
        {
            return env
        }
        let sibling = URL(fileURLWithPath: CommandLine.arguments[0])
            .deletingLastPathComponent()
            .appendingPathComponent("vzctl")
            .path
        if FileManager.default.isExecutableFile(atPath: sibling) {
            return sibling
        }
        for dir in ["/Users/\(NSUserName())/.local/bin", "/opt/homebrew/bin", "/usr/local/bin"] {
            let candidate = "\(dir)/vzctl"
            if FileManager.default.isExecutableFile(atPath: candidate) {
                return candidate
            }
        }
        throw RestRouteError(500, .internalError, "vzctl binary not found (set VZCTL_BIN)")
    }

    private func shellQuote(_ value: String) -> String {
        if value.isEmpty { return "''" }
        if value.unicodeScalars.allSatisfy({ CharacterSet.alphanumerics.contains($0) || "/._-:@+=".contains(Character($0)) }) {
            return value
        }
        return "'" + value.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    private func workerResult(_ args: [String]) throws -> JSONValue {
        let result = try jobs.runSync(arguments: args)
        let trimmed = result.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
        if let data = trimmed.data(using: .utf8),
           let value = try? JSONDecoder().decode(JSONValue.self, from: data)
        {
            if result.exitCode != 0 {
                // Still return envelope so UI can inspect status/fail.
                return value
            }
            return value
        }
        if result.exitCode != 0 {
            throw RestRouteError(
                500,
                .internalError,
                result.stderr.isEmpty ? "vzctl failed (\(result.exitCode))" : result.stderr
            )
        }
        return .object(["stdout": .string(result.stdout)])
    }

    private func bodyObject(_ request: RestHTTPRequest) throws -> [String: JSONValue] {
        if request.body.isEmpty { return [:] }
        let value = try JSONDecoder().decode(JSONValue.self, from: request.body)
        guard case let .object(obj) = value else {
            throw RestRouteError(400, .badRequest, "JSON object required")
        }
        return obj
    }

    private func bodyText(_ request: RestHTTPRequest) throws -> String {
        if let obj = try? bodyObject(request), case let .string(content)? = obj["content"] {
            return content
        }
        guard let text = String(data: request.body, encoding: .utf8) else {
            throw RestRouteError(400, .badRequest, "UTF-8 body required")
        }
        return text
    }

    private func optionalBool(_ value: JSONValue?) -> Bool? {
        guard let value else { return nil }
        if case let .bool(b) = value { return b }
        return nil
    }
}

struct RestRouteError: Error {
    var status: Int
    var code: RestErrorCode
    var message: String
    var details: [String: JSONValue]?

    init(
        _ status: Int,
        _ code: RestErrorCode,
        _ message: String,
        details: [String: JSONValue]? = nil
    ) {
        self.status = status
        self.code = code
        self.message = message
        self.details = details
    }
}

@discardableResult
private func writeData(_ data: Data, to fd: Int32) -> Bool {
    data.withUnsafeBytes { raw in
        guard let base = raw.baseAddress else { return false }
        var offset = 0
        while offset < data.count {
            let written = Darwin.write(fd, base.advanced(by: offset), data.count - offset)
            if written <= 0 { return false }
            offset += written
        }
        return true
    }
}
