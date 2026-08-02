import Darwin
import Foundation
import VzDaemonKit

enum EdgeServerError: Error, CustomStringConvertible {
    case system(String, Int32)
    case invalid(String)
    case conflict(String)

    var description: String {
        switch self {
        case let .system(operation, code):
            return "\(operation): \(String(cString: strerror(code)))"
        case let .invalid(message), let .conflict(message): return message
        }
    }
}

private struct EdgeManifest: Codable {
    let generation: Int64
    let digest: String
    let desired: JSONValue
}

final class EdgeServer: @unchecked Sendable {
    let socketPath: String
    private let stateDirectory: URL
    private let runtimeDirectory: URL
    private let manifestPath: URL
    private let lock = NSLock()
    private let reconcileLock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false
    private var generation: Int64 = 0
    private var digest = ""
    private var desired: JSONValue = .object([:])
    private var lastError: String?
    private let dnsConfiguration: DNSConfiguration
    private let dnsServer: DNSServer
    private let portProxy = PortForwardProxy()
    private let gatewayProxy = HostGatewayIngressProxy()
    private let processes = EmbeddedProcessManager()
    private let hostNetworking: HostNetworkReconciler

    init(
        stateDirectory: URL,
        dnsConfiguration: DNSConfiguration = .environment(),
        hostNetworking: HostNetworkReconciler = HostNetworkReconciler()
    ) throws {
        self.stateDirectory = stateDirectory
        self.dnsConfiguration = dnsConfiguration
        self.hostNetworking = hostNetworking
        dnsServer = DNSServer(configuration: dnsConfiguration)
        runtimeDirectory = stateDirectory.appendingPathComponent("runtime/edge", isDirectory: true)
        manifestPath = runtimeDirectory.appendingPathComponent("manifest.json")
        socketPath = stateDirectory.appendingPathComponent("edge.sock").path
        try FileManager.default.createDirectory(
            at: runtimeDirectory, withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        _ = chmod(stateDirectory.path, 0o700)
        _ = chmod(runtimeDirectory.path, 0o700)
        restoreManifest()
    }

    func run() throws {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw EdgeServerError.system("socket", errno) }
        lock.withLock { listener = fd }
        do {
            try prepareSocketPath()
            var address = try unixAddress(path: socketPath)
            let result = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard result == 0 else { throw EdgeServerError.system("bind", errno) }
            lock.withLock { ownsSocket = true }
            guard chmod(socketPath, 0o600) == 0 else {
                throw EdgeServerError.system("chmod edge socket", errno)
            }
            guard Darwin.listen(fd, 16) == 0 else { throw EdgeServerError.system("listen", errno) }
            while true {
                let client = Darwin.accept(fd, nil, nil)
                if client < 0 {
                    if errno == EINTR { continue }
                    if lock.withLock({ listener < 0 }) { break }
                    throw EdgeServerError.system("accept", errno)
                }
                DispatchQueue.global().async { [weak self] in self?.handle(client) }
            }
        } catch {
            stop()
            throw error
        }
    }

    func stop() {
        let state = lock.withLock { () -> (Int32, Bool) in
            let fd = listener
            listener = -1
            let unlink = ownsSocket
            ownsSocket = false
            return (fd, unlink)
        }
        if state.0 >= 0 {
            Darwin.shutdown(state.0, SHUT_RDWR)
            Darwin.close(state.0)
        }
        if state.1 { Darwin.unlink(socketPath) }
        dnsServer.shutdown()
        portProxy.shutdown()
        gatewayProxy.shutdown()
        processes.shutdown()
    }

    private func handle(_ client: Int32) {
        defer { Darwin.close(client) }
        guard peerUID(client) == geteuid() else { return }
        var pending = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while true {
            let count = Darwin.read(client, &buffer, buffer.count)
            if count <= 0 { return }
            pending.append(buffer, count: count)
            while let newline = pending.firstIndex(of: 0x0A) {
                let line = Data(pending[..<newline])
                pending.removeSubrange(...newline)
                if line.isEmpty { continue }
                let response = response(for: line)
                guard let encoded = try? JSONRPCFraming.encode(response),
                      writeAll(encoded, to: client) else { return }
            }
        }
    }

    private func response(for data: Data) -> JSONRPCResponse {
        let request: JSONRPCRequest
        do { request = try JSONRPCFraming.decode(JSONRPCRequest.self, from: data) }
        catch {
            return JSONRPCResponse(error: JSONRPCError(code: -32700, message: "Parse error"), id: .null)
        }
        let id = request.id ?? .null
        do {
            switch request.method {
            case "edge.health", "edge.status":
                return JSONRPCResponse(result: status(), id: id)
            case "dns.lookup":
                let params = try object(request.params)
                let name = try string("name", params)
                let network = try optionalString("network", params)
                let result = dnsServer.lookup(name, network: network)
                return JSONRPCResponse(result: .object([
                    "name": .string(DNSZoneBuilder.canonicalName(name)),
                    "network": network.map(JSONValue.string) ?? .null,
                    "host": result.host.map { .array($0.map(JSONValue.string)) } ?? .null,
                    "guest": result.guest.map { .array($0.map(JSONValue.string)) } ?? .null,
                ]), id: id)
            case "edge.reconcile":
                let params = try object(request.params)
                let nextGeneration = try int64("generation", params)
                let nextDigest = try string("digest", params)
                guard let nextDesired = params["desired"] else {
                    throw EdgeServerError.invalid("desired is required")
                }
                return JSONRPCResponse(
                    result: try reconcile(
                        generation: nextGeneration, digest: nextDigest, desired: nextDesired
                    ), id: id
                )
            default:
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32601, message: "Method not found"), id: id
                )
            }
        } catch let error as EdgeServerError {
            let code = { if case .conflict = error { return -32042 }; return -32602 }()
            return JSONRPCResponse(error: JSONRPCError(code: code, message: error.description), id: id)
        } catch {
            lock.withLock { lastError = String(describing: error) }
            return JSONRPCResponse(
                error: JSONRPCError(code: -32043, message: String(describing: error)), id: id
            )
        }
    }

    func reconcile(generation next: Int64, digest nextDigest: String,
                   desired nextDesired: JSONValue) throws -> JSONValue {
        try reconcileLock.withLock {
            let current = lock.withLock { (generation, digest, desired) }
            if next < current.0 { throw EdgeServerError.conflict("stale edge generation \(next)") }
            if next == current.0 {
                guard nextDigest == current.1 else {
                    throw EdgeServerError.conflict("edge generation digest conflict")
                }
                return status()
            }
            guard !nextDigest.isEmpty else { throw EdgeServerError.invalid("digest is required") }
            do {
                try apply(nextDesired, generation: next)
                try persist(EdgeManifest(generation: next, digest: nextDigest, desired: nextDesired))
                lock.withLock {
                    generation = next
                    digest = nextDigest
                    desired = nextDesired
                    lastError = nil
                }
                return status()
            } catch {
                lock.withLock { lastError = String(describing: error) }
                if current.0 > 0 { try? apply(current.2, generation: current.0) }
                throw error
            }
        }
    }

    private func apply(_ value: JSONValue, generation: Int64) throws {
        let desired = try object(value)
        let snapshot = try parseSnapshot(desired["network_snapshot"] ?? .object([:]))
        let hostServices = try strings(desired["host_services"] ?? .array([]))
        let dnsRecords = try array(desired["dns_records"] ?? .array([])).map(parseDNSRecord)
        let ports = try array(desired["port_forwards"] ?? .array([])).map(parsePort)
        let ingress = try array(desired["ingress"] ?? .array([]))
        let oidc = try array(desired["oidc"] ?? .array([]))

        var serviceSpecs: [EmbeddedProcessManager.Spec] = []
        var gatewayBindings: [HostGatewayIngressProxy.Binding] = []
        let activeNetworks = snapshot.networks.filter {
            $0.runtimeState == "active" && !$0.isDockerBackend
        }
        let targetAliasCIDRs = Set(activeNetworks.map(\.cidr))
        var targetFirewall = Dictionary(uniqueKeysWithValues: activeNetworks.map { network in
            (
                network.cidr,
                DnsBind.FirewallBinding(
                    cidr: network.cidr,
                    allowedSources: [network.cidr],
                    tcpPorts: [],
                    dnsPort: dnsConfiguration.guestPort,
                    dnsBackendPort: dnsConfiguration.guestBackendPort
                )
            )
        })
        for raw in ingress {
            let params = try object(raw)
            let project = try safeProject(string("project", params))
            let caddyfile = try string("caddyfile", params)
            let binary = try string("binary", params)
            try requireBinary(binary, basename: "caddy")
            let workDir = runtimeDirectory.appendingPathComponent("ingress/\(project)", isDirectory: true)
            let configPath = try writeConfig(
                caddyfile, directory: workDir,
                name: "Caddyfile.\(generation)", mode: 0o600
            )
            let backendHTTP = try port("backend_http_port", params, default: try port("http_port", params, default: 80))
            serviceSpecs.append(.init(
                kind: "caddy", project: project, name: "caddy-\(project)", binary: binary,
                arguments: ["run", "--config", configPath, "--adapter", "caddyfile"],
                workDir: workDir.path,
                pidFile: workDir.appendingPathComponent("caddy.pid").path, env: [:],
                readiness: .init(host: "127.0.0.1", port: backendHTTP)
            ))
            let http = try port("http_port", params, default: 80)
            let https = try port("https_port", params, default: 443)
            let backendHTTPS = try port("backend_https_port", params, default: https)
            for gateway in try strings(params["gateways"] ?? .array([])) {
                gatewayBindings.append(.init(
                    gatewayIP: gateway, port: http,
                    backendHost: "127.0.0.1", backendPort: backendHTTP
                ))
                gatewayBindings.append(.init(
                    gatewayIP: gateway, port: https,
                    backendHost: "127.0.0.1", backendPort: backendHTTPS
                ))
                if let network = activeNetworks.first(where: {
                    IPv4CIDR.hostService(for: $0.cidr) == gateway
                }) {
                    mergeFirewallBinding(
                        cidr: network.cidr,
                        allowedSources: [network.cidr],
                        ports: [http, https],
                        into: &targetFirewall
                    )
                }
            }
            for rawBinding in try array(params["gateway_bindings"] ?? .array([])) {
                let binding = try object(rawBinding)
                let cidr = try string("cidr", binding)
                guard targetAliasCIDRs.contains(cidr) else {
                    throw EdgeServerError.invalid("ingress gateway binding requires active vmnet \(cidr)")
                }
                let sources = try strings(binding["allowed_sources"] ?? .array([.string(cidr)]))
                mergeFirewallBinding(
                    cidr: cidr,
                    allowedSources: sources,
                    ports: [http, https],
                    into: &targetFirewall
                )
            }
        }
        for raw in oidc {
            let params = try object(raw)
            let project = try safeProject(string("project", params))
            let processName = try string("processName", params)
            let kind = processName.hasPrefix("oidc-simple-") ? "oidc-simple" : "dex"
            let binary = try string("binary", params)
            try requireBinary(binary, basename: kind == "dex" ? "dex" : "vzctl-oidc-simple")
            let configName = try optionalString("configName", params) ?? (kind == "dex" ? "config.yaml" : "config.json")
            let ext = (configName as NSString).pathExtension
            let workDir = runtimeDirectory.appendingPathComponent("oidc/\(project)", isDirectory: true)
            let configPath = try writeConfig(
                try string("config", params), directory: workDir,
                name: "config.\(generation)\(ext.isEmpty ? "" : ".\(ext)")", mode: 0o600
            )
            let templates = try strings(params["arguments"] ?? .array([.string("serve"), .string("{config}")]))
            let arguments = templates.map { $0.replacingOccurrences(of: "{config}", with: configPath) }
            let expectedArguments = kind == "dex" ? ["serve", configPath] : ["--config", configPath]
            guard arguments == expectedArguments else {
                throw EdgeServerError.invalid("unsupported \(kind) arguments")
            }
            let readiness = try loopbackEndpoint(string("listen", params))
            serviceSpecs.append(.init(
                kind: kind, project: project, name: processName,
                binary: binary, arguments: arguments,
                workDir: workDir.path,
                pidFile: workDir.appendingPathComponent("oidc.pid").path,
                env: [:], readiness: .init(host: readiness.0, port: readiness.1)
            ))
        }

        try hostNetworking.prepare(
            targetCIDRs: targetAliasCIDRs,
            targetFirewall: targetFirewall
        )
        _ = try processes.reconcile(serviceSpecs)
        _ = try portProxy.ensure(ports)
        let ingressResult = gatewayProxy.ensure(gatewayBindings)
        if let failure = ingressResult.skipped.first {
            throw EdgeServerError.invalid(
                "ingress listener \(failure.0.gatewayIP):\(failure.0.port): \(failure.1)"
            )
        }
        dnsServer.setHostServices(hostServices)
        dnsServer.setRuntimeRecords(dnsRecords)
        let dns = dnsServer.reload(snapshot: snapshot)
        guard dns.ok else { throw EdgeServerError.invalid(dns.lastError ?? "DNS reconcile failed") }
        try hostNetworking.finish(
            targetCIDRs: targetAliasCIDRs,
            targetFirewall: targetFirewall
        )
    }

    func status() -> JSONValue {
        let state = lock.withLock { (generation, digest, lastError) }
        let dns = dnsServer.health()
        let services = processes.statusAll()
        let servicesOK = services.allSatisfy { $0["running"] == .bool(true) }
        return .object([
            "ok": .bool(dns.ok && servicesOK && state.2 == nil),
            "version": .string(VzDaemonKit.version),
            "generation": .number(Double(state.0)),
            "digest": .string(state.1),
            "socket": .string(socketPath),
            "dns": dns.json,
            "port_forwards": .array(portProxy.list().map(\.json)),
            "ingress_bindings": .number(Double(gatewayProxy.list().count)),
            "host_aliases": .number(Double(hostNetworking.aliasCount)),
            "services": .array(services.map(JSONValue.object)),
            "last_error": state.2.map(JSONValue.string) ?? .null,
        ])
    }

    private func restoreManifest() {
        guard FileManager.default.fileExists(atPath: manifestPath.path) else { return }
        do {
            let manifest = try JSONDecoder().decode(
                EdgeManifest.self, from: Data(contentsOf: manifestPath)
            )
            try apply(manifest.desired, generation: manifest.generation)
            lock.withLock {
                generation = manifest.generation
                digest = manifest.digest
                desired = manifest.desired
                lastError = nil
            }
        } catch {
            lock.withLock { lastError = "manifest recovery: \(error)" }
        }
    }

    private func persist(_ manifest: EdgeManifest) throws {
        let temporary = manifestPath.appendingPathExtension("tmp")
        let data = try JSONEncoder().encode(manifest)
        try data.write(to: temporary, options: .atomic)
        _ = chmod(temporary.path, 0o600)
        if FileManager.default.fileExists(atPath: manifestPath.path) {
            try FileManager.default.removeItem(at: manifestPath)
        }
        try FileManager.default.moveItem(at: temporary, to: manifestPath)
    }

    private func writeConfig(_ value: String, directory: URL, name: String,
                             mode: mode_t) throws -> String {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        _ = chmod(directory.path, 0o700)
        let target = directory.appendingPathComponent(name)
        try Data(value.utf8).write(to: target, options: .atomic)
        _ = chmod(target.path, mode)
        return target.path
    }

    private func parseSnapshot(_ value: JSONValue) throws -> NetworkSnapshot {
        let values = try object(value)
        let networks = try array(values["networks"] ?? .array([])).map { raw -> NetworkRecord in
            let item = try object(raw)
            return NetworkRecord(
                name: try string("name", item), cidr: try string("cidr", item),
                mode: try optionalString("mode", item) ?? "shared",
                natEgress: bool("nat_egress", item, default: true),
                backend: try optionalString("backend", item) ?? NetworkRecord.backendVmnet,
                labels: try labels(item["labels"]), project: try optionalString("project", item),
                stack: try optionalString("stack", item),
                runtimeState: try optionalString("runtime_state", item) ?? "active",
                lastError: try optionalString("last_error", item),
                updatedAt: try optionalString("updated_at", item) ?? ""
            )
        }
        let attachments = try array(values["attachments"] ?? .array([])).map { raw -> NetworkAttachmentRecord in
            let item = try object(raw)
            return NetworkAttachmentRecord(
                vmID: try string("vm_id", item), networkName: try string("network", item),
                ip: try string("ip", item), labels: try labels(item["labels"]),
                project: try optionalString("project", item), stack: try optionalString("stack", item),
                updatedAt: try optionalString("updated_at", item) ?? ""
            )
        }
        return NetworkSnapshot(networks: networks, attachments: attachments)
    }

    private func parseDNSRecord(_ value: JSONValue) throws -> DNSRuntimeRecord {
        let item = try object(value)
        return DNSRuntimeRecord(
            name: try string("name", item),
            network: try string("network", item),
            listenerNetwork: try string("listener_network", item),
            stack: try string("stack", item),
            project: try string("project", item),
            ip: try string("ip", item)
        )
    }

    private func parsePort(_ raw: JSONValue) throws -> PortForwardRecord {
        let item = try object(raw)
        return PortForwardRecord(
            bind: try string("bind", item), hostPort: try port("host_port", item),
            guestIP: try string("guest_ip", item), guestPort: try port("guest_port", item),
            vmID: try string("vm_id", item), source: try string("source", item),
            project: try string("project", item), stack: try string("stack", item),
            state: try optionalString("state", item) ?? "active",
            updatedAt: try optionalString("updated_at", item) ?? ""
        )
    }

    private func prepareSocketPath() throws {
        if FileManager.default.fileExists(atPath: socketPath) {
            let probe = VzEdgeClient(socketPath: socketPath, timeoutSeconds: 1)
            if (try? probe.health()) != nil { throw EdgeServerError.conflict("vz-edge already running") }
            Darwin.unlink(socketPath)
        }
    }
}

private func object(_ value: JSONValue?) throws -> [String: JSONValue] {
    guard case let .object(values)? = value else { throw EdgeServerError.invalid("object required") }
    return values
}

private func array(_ value: JSONValue) throws -> [JSONValue] {
    guard case let .array(values) = value else { throw EdgeServerError.invalid("array required") }
    return values
}

private func string(_ key: String, _ values: [String: JSONValue]) throws -> String {
    guard case let .string(value)? = values[key], !value.isEmpty else {
        throw EdgeServerError.invalid("\(key) is required")
    }
    return value
}

private func optionalString(_ key: String, _ values: [String: JSONValue]) throws -> String? {
    guard let raw = values[key], raw != .null else { return nil }
    guard case let .string(value) = raw else { throw EdgeServerError.invalid("\(key) must be string") }
    return value
}

private func strings(_ value: JSONValue) throws -> [String] {
    try array(value).map {
        guard case let .string(item) = $0 else { throw EdgeServerError.invalid("string array required") }
        return item
    }
}

private func mergeFirewallBinding(
    cidr: String,
    allowedSources: [String],
    ports: [UInt16],
    into bindings: inout [String: DnsBind.FirewallBinding]
) {
    let existing = bindings[cidr]
    bindings[cidr] = DnsBind.FirewallBinding(
        cidr: cidr,
        allowedSources: Array(Set((existing?.allowedSources ?? []) + allowedSources)).sorted(),
        tcpPorts: Array(Set((existing?.tcpPorts ?? []) + ports)).sorted(),
        dnsPort: existing?.dnsPort,
        dnsBackendPort: existing?.dnsBackendPort
    )
}

private func port(_ key: String, _ values: [String: JSONValue], default fallback: UInt16? = nil) throws -> UInt16 {
    guard let raw = values[key] else {
        if let fallback { return fallback }
        throw EdgeServerError.invalid("\(key) is required")
    }
    guard case let .number(number) = raw, number >= 1, number <= 65_535 else {
        throw EdgeServerError.invalid("invalid \(key)")
    }
    return UInt16(number)
}

private func int64(_ key: String, _ values: [String: JSONValue]) throws -> Int64 {
    guard case let .number(value)? = values[key], value >= 0 else {
        throw EdgeServerError.invalid("invalid \(key)")
    }
    return Int64(value)
}

private func bool(_ key: String, _ values: [String: JSONValue], default fallback: Bool) -> Bool {
    guard case let .bool(value)? = values[key] else { return fallback }
    return value
}

private func labels(_ value: JSONValue?) throws -> [String: String] {
    guard let value else { return [:] }
    let raw = try object(value)
    var result: [String: String] = [:]
    for (key, value) in raw {
        guard case let .string(item) = value else { throw EdgeServerError.invalid("label must be string") }
        result[key] = item
    }
    return result
}

private func safeProject(_ value: String) throws -> String {
    guard !value.isEmpty, value.utf8.count <= 63,
          value.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" || $0 == "_" }) else {
        throw EdgeServerError.invalid("invalid project")
    }
    return value
}

private func requireBinary(_ path: String, basename: String) throws {
    guard URL(fileURLWithPath: path).lastPathComponent == basename else {
        throw EdgeServerError.invalid("edge service binary must be \(basename)")
    }
}

private func loopbackEndpoint(_ value: String) throws -> (String, UInt16) {
    guard let separator = value.lastIndex(of: ":"),
          String(value[..<separator]) == "127.0.0.1",
          let port = UInt16(value[value.index(after: separator)...]), port > 0 else {
        throw EdgeServerError.invalid("service listen must be 127.0.0.1:port")
    }
    return ("127.0.0.1", port)
}

private func unixAddress(path: String) throws -> sockaddr_un {
    var address = sockaddr_un()
    address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
    address.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
        throw EdgeServerError.invalid("edge socket path is too long")
    }
    withUnsafeMutableBytes(of: &address.sun_path) { raw in
        raw.copyBytes(from: bytes)
        raw[bytes.count] = 0
    }
    return address
}

private func peerUID(_ fd: Int32) -> uid_t? {
    var uid: uid_t = 0
    var gid: gid_t = 0
    return getpeereid(fd, &uid, &gid) == 0 ? uid : nil
}

private func writeAll(_ data: Data, to fd: Int32) -> Bool {
    data.withUnsafeBytes { raw in
        guard let base = raw.baseAddress else { return true }
        var offset = 0
        while offset < raw.count {
            let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
            if count <= 0 { return false }
            offset += count
        }
        return true
    }
}
