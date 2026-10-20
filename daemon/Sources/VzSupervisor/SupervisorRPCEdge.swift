import CryptoKit
import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func handleEdgeRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
        switch request.method {
        case "dns.status":
            return JSONRPCResponse(
                result: vzEdgeHealth().dns,
                id: request.id ?? .null
            )
        case "dns.lookup":
            do {
                let params = try networkParams(request.params)
                let name = try requiredString("name", from: params)
                return JSONRPCResponse(result: try edgeClient.lookup(name: name), id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                let portsValue = params["ports"] ?? .array([])
                guard case let .array(items) = portsValue else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "ports must be an array"),
                        id: request.id ?? .null
                    )
                }
                var records: [PortForwardRecord] = []
                for item in items {
                    guard case let .object(object) = item else { continue }
                    let bind = try requiredString("bind", from: object)
                    let hostPort = try requiredPort("host_port", from: object)
                    let guestIP = try requiredString("guest_ip", from: object)
                    let guestPort = try requiredPort("guest_port", from: object)
                    let vmID = try requiredString("vm_id", from: object)
                    let source = try optionalString("source", from: object) ?? "\(bind):\(hostPort)"
                    records.append(
                        PortForwardRecord(
                            bind: bind,
                            hostPort: hostPort,
                            guestIP: guestIP,
                            guestPort: guestPort,
                            vmID: vmID,
                            source: source,
                            project: project,
                            stack: stack
                        )
                    )
                }
                try database.replacePortForwards(project: project, stack: stack, records: records)
                try reconcileEdgeThrowing(reason: "port.ensure")
                emit(
                    type: "port.ensured",
                    data: [
                        "project": .string(project),
                        "stack": .string(stack),
                        "count": .number(Double(records.count)),
                    ]
                )
                return JSONRPCResponse(
                    result: .object(["ports": .array(records.map(\.json))]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.list":
            do {
                let params = (try? networkParams(request.params)) ?? [:]
                let project = try optionalString("project", from: params)
                let stack = try optionalString("stack", from: params)
                let records = try database.portForwards(project: project, stack: stack)
                return JSONRPCResponse(
                    result: .object(["ports": .array(records.map(\.json))]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "port.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                try database.deletePortForwards(project: project, stack: stack)
                try reconcileEdgeThrowing(reason: "port.purge")
                emit(
                    type: "port.purged",
                    data: ["project": .string(project), "stack": .string(stack)]
                )
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "stack": .string(stack),
                        "purged": .bool(true),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.records.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let records = params["records"] ?? .array([])
                guard case .array = records else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "records must be an array"),
                        id: request.id ?? .null
                    )
                }
                try database.setEdgeDNSRecords(project: project, records: records)
                let edge = try reconcileEdgeThrowing(reason: "dns.records.ensure")
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "records": records,
                        "dns": edgeDNS(from: edge),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.records.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeDNSRecords(project: project, records: .array([]))
                try reconcileEdgeThrowing(reason: "dns.records.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "dns.host_services.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let hostsValue = params["hosts"] ?? .array([])
                guard case .array = hostsValue else {
                    return JSONRPCResponse(
                        error: JSONRPCError(code: -32602, message: "hosts must be an array"),
                        id: request.id ?? .null
                    )
                }
                try database.setEdgeHostServices(project: project, hosts: hostsValue)
                let edge = try reconcileEdgeThrowing(reason: "dns.host_services.ensure")
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "hosts": hostsValue,
                        "dns": edgeDNS(from: edge),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "ingress.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                _ = try requiredString("caddyfile", from: params)
                let snapshot = try edgeProjectMutationSnapshot(project: project)
                var stored = params
                if stored["binary"] == nil {
                    stored["binary"] = .string(stateDirectory.appendingPathComponent("bin/caddy").path)
                }
                let hostsValue = stored.removeValue(forKey: "hosts")
                if let hostsValue {
                    guard case .array = hostsValue else {
                        return JSONRPCResponse(
                            error: JSONRPCError(code: -32602, message: "hosts must be an array"),
                            id: request.id ?? .null
                        )
                    }
                    try database.setEdgeHostServices(project: project, hosts: hostsValue)
                }
                try database.setEdgeIngress(project: project, value: .object(stored))
                let edge: JSONValue
                do {
                    edge = try reconcileEdgeThrowing(reason: "ingress.ensure")
                } catch {
                    try restoreEdgeProjectMutation(project: project, snapshot: snapshot)
                    _ = try? reconcileEdgeThrowing(reason: "ingress.ensure.rollback")
                    throw error
                }

                emit(
                    type: "ingress.ensured",
                    data: [
                        "project": .string(project),
                        "edge": edge,
                    ]
                )
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "edge": edge,
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "ingress.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeIngress(project: project, value: nil)
                try database.setEdgeHostServices(project: project, hosts: .array([]))
                try reconcileEdgeThrowing(reason: "ingress.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "oidc.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                _ = try requiredString("config", from: params)
                var stored = params
                if stored["binary"] == nil {
                    stored["binary"] = .string(stateDirectory.appendingPathComponent("bin/dex").path)
                }
                if stored["processName"] == nil { stored["processName"] = .string("dex-\(project)") }
                try database.setEdgeOIDC(project: project, value: .object(stored))
                let edge = try reconcileEdgeThrowing(reason: "oidc.ensure")
                let processName = try requiredString("processName", from: stored)
                emit(type: "oidc.ensured", data: [
                    "project": .string(project),
                    "process": .string(processName),
                ])
                return JSONRPCResponse(
                    result: .object([
                        "project": .string(project),
                        "process": .string(processName),
                        "edge": edge,
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "oidc.purge":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.setEdgeOIDC(project: project, value: nil)
                try reconcileEdgeThrowing(reason: "oidc.purge")
                return JSONRPCResponse(
                    result: .object(["project": .string(project), "purged": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        default: return nil
        }
    }

    func reconcileEdge(reason: String) {
        do {
            let result = try reconcileEdgeThrowing(reason: reason)
            emit(type: "edge.reconciled", data: [
                "reason": .string(reason),
                "status": result,
            ])
        } catch {
            emit(type: "edge.reconcile_failed", data: [
                "reason": .string(reason),
                "error": .string(String(describing: error)),
            ])
        }
    }

    struct EdgeProjectMutationSnapshot {
        let ingress: JSONValue?
        let hostServices: JSONValue
    }

    func edgeProjectMutationSnapshot(project: String) throws -> EdgeProjectMutationSnapshot {
        guard let record = try database.edgeProjects().first(where: { $0.project == project }) else {
            return EdgeProjectMutationSnapshot(ingress: nil, hostServices: .array([]))
        }
        return EdgeProjectMutationSnapshot(
            ingress: record.ingress,
            hostServices: record.hostServices
        )
    }

    func restoreEdgeProjectMutation(
        project: String,
        snapshot: EdgeProjectMutationSnapshot
    ) throws {
        try database.setEdgeIngress(project: project, value: snapshot.ingress)
        try database.setEdgeHostServices(project: project, hosts: snapshot.hostServices)
    }

    @discardableResult
    func reconcileEdgeThrowing(reason: String) throws -> JSONValue {
        try edgeReconcileLock.withLock {
            let projects = try database.edgeProjects()
            let hostServices = try projects.flatMap { record -> [JSONValue] in
                guard case let .array(values) = record.hostServices else {
                    throw SupervisorError.database("edge host services must be an array")
                }
                return values
            }
            let dnsRecords = try projects.flatMap { record -> [JSONValue] in
                guard case let .array(values) = record.dnsRecords else {
                    throw SupervisorError.database("edge DNS records must be an array")
                }
                return values
            }
            let ingress = projects.compactMap(\.ingress)
            let oidc = projects.compactMap(\.oidc)
            let ports = try database.portForwards()
            let desired: JSONValue = .object([
                "network_snapshot": try networkRegistry.snapshot().json,
                "host_services": .array(hostServices),
                "dns_records": .array(dnsRecords),
                "port_forwards": .array(ports.map(\.json)),
                "ingress": .array(ingress),
                "oidc": .array(oidc),
            ])
            let generation = try database.nextEdgeGeneration()
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            let data = try encoder.encode(desired)
            let digest = SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
            return try edgeClient.reconcile(
                generation: generation, digest: digest, desired: desired
            )
        }
    }

    func vzEdgeHealth() -> (ok: Bool, json: JSONValue, dns: JSONValue) {
        do {
            let health = try edgeClient.health()
            maybeRepairIngressFromEdgeHealth(health)
            guard case let .object(values) = health else {
                return (false, health, .object(["ok": .bool(false)]))
            }
            let ok = values["ok"] == .bool(true)
            return (ok, health, values["dns"] ?? .object(["ok": .bool(false)]))
        } catch {
            let failure: JSONValue = .object([
                "ok": .bool(false),
                "socket": .string(edgeClient.socketPath),
                "error": .string(String(describing: error)),
            ])
            return (false, failure, failure)
        }
    }

    func maybeRepairIngressFromEdgeHealth(_ health: JSONValue) {
        guard case let .object(values) = health else { return }
        let loopbackBroken = values["ingress_loopback_ok"] == .bool(false)
        let guestWarning = values["ingress_guest_warning"].map { $0 != .null } ?? false
        guard loopbackBroken || guestWarning else { return }
        let now = Date()
        guard now.timeIntervalSince(lastIngressWatchdogReconcile) >= 60 else { return }
        lastIngressWatchdogReconcile = now
        reconcileEdge(reason: "ingress.watchdog")
    }

    func edgeDNS(from status: JSONValue) -> JSONValue {
        guard case let .object(values) = status else { return .null }
        return values["dns"] ?? .null
    }
}
