import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func handleNetworkRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
        switch request.method {
        case "resilience.ensure":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                let stack = try requiredString("stack", from: params)
                let probeEnabled = optionalBool("probe_enabled", from: params) ?? true
                let probeURL = try requiredString("probe_url", from: params)
                let restartVMs = optionalBool("restart_vms", from: params) ?? false
                guard let url = URL(string: probeURL),
                      ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
                      url.host != nil,
                      url.user == nil,
                      url.password == nil
                else {
                    throw NetworkRegistryError.invalid("invalid resilience probe_url")
                }
                try database.setNetworkResiliencePolicy(
                    project: project,
                    stack: stack,
                    probeEnabled: probeEnabled,
                    probeURL: probeURL,
                    restartVMs: restartVMs
                )
                return JSONRPCResponse(result: .object([
                    "project": .string(project),
                    "stack": .string(stack),
                    "probe_enabled": .bool(probeEnabled),
                    "probe_url": .string(probeURL),
                    "restart_vms": .bool(restartVMs),
                ]), id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "resilience.remove":
            do {
                let params = try networkParams(request.params)
                let project = try requiredString("project", from: params)
                try database.removeNetworkResiliencePolicy(project: project)
                return JSONRPCResponse(
                    result: .object(["removed": .bool(true), "project": .string(project)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.create":
            do {
                let params = try networkParams(request.params)
                let natEgress = optionalBool("nat_egress", from: params) ?? true
                let backend = try optionalString("backend", from: params) ?? NetworkRecord.backendVmnet
                let record = try networkRegistry.create(
                    name: try requiredString("name", from: params),
                    cidr: try requiredString("cidr", from: params),
                    mode: try optionalString("mode", from: params) ?? "shared",
                    natEgress: natEgress,
                    backend: backend,
                    labels: try labels(from: params),
                    project: try optionalString("project", from: params),
                    stack: try optionalString("stack", from: params)
                )
                reconcileEdge(reason: "net.create")
                emit(type: "net.created", data: ["network": .string(record.name)])
                return JSONRPCResponse(result: record.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.attach":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let record = try networkRegistry.attach(
                    vmID: vmID,
                    networkName: try requiredString("network", from: params),
                    ip: try requiredString("ip", from: params),
                    labels: try labels(from: params),
                    project: try optionalString("project", from: params),
                    stack: try optionalString("stack", from: params),
                    vmIsStopped: vmIsStopped(vmID)
                )
                reconcileEdge(reason: "net.attach")
                emit(
                    type: "net.attached",
                    data: [
                        "vm_id": .string(record.vmID),
                        "network": .string(record.networkName),
                        "ip": .string(record.ip),
                    ]
                )
                return JSONRPCResponse(result: record.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.detach":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let network = try requiredString("network", from: params)
                try networkRegistry.detach(
                    vmID: vmID,
                    networkName: network,
                    project: try optionalString("project", from: params),
                    vmIsStopped: vmIsStopped(vmID)
                )
                reconcileEdge(reason: "net.detach")
                emit(
                    type: "net.detached",
                    data: ["vm_id": .string(vmID), "network": .string(network)]
                )
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "network": .string(network),
                        "detached": .bool(true),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.delete":
            do {
                let params = try networkParams(request.params)
                let name = try requiredString("name", from: params)
                try networkRegistry.delete(name: name, project: try optionalString("project", from: params))
                reconcileEdge(reason: "net.delete")
                emit(type: "net.deleted", data: ["network": .string(name)])
                return JSONRPCResponse(
                    result: .object(["name": .string(name), "deleted": .bool(true)]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.list":
            do {
                return JSONRPCResponse(
                    result: try networkRegistry.snapshot().json,
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.default.show":
            do {
                let result: JSONValue
                if let (configured, network) = try networkRegistry.defaultNetwork() {
                    result = configured.json(network: network)
                } else {
                    result = .null
                }
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "net.default.set":
            do {
                let params = try networkParams(request.params)
                let configured = try networkRegistry.setDefault(
                    name: try requiredString("name", from: params),
                    cidr: try requiredString("cidr", from: params)
                )
                let network = try networkRegistry.defaultNetwork()?.1
                reconcileEdge(reason: "net.default.set")
                emit(type: "net.default.changed", data: [
                    "network": .string(configured.name),
                    "cidr": .string(configured.cidr),
                ])
                return JSONRPCResponse(
                    result: configured.json(network: network),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "route.apply", "route.plan", "route.status":
            do {
                let operation = RouterOperation(
                    rawValue: String(request.method.dropFirst("route.".count))
                )!
                let params = try request.params.map { try networkParams($0) } ?? [:]
                let requestedRouter = try optionalString("router", from: params)
                let project = try optionalString("project", from: params)
                let policies = operation == .status
                    ? []
                    : try forwardPolicies(from: params["policies"])
                let snapshot = try networkRegistry.snapshot()
                let records = stateLock.withLock {
                    helpers.values.filter { $0.state == "running" }
                }
                let routers = try routeRouterRecords(
                    records, requestedRouter: requestedRouter, project: project, operation: operation
                )
                guard !routers.isEmpty else {
                    throw RouteApplyError.invalid(
                        requestedRouter.map { "router VM \($0) is not running" }
                            ?? "no running VM with roles: [router]"
                    )
                }
                var policyMatches: [String: Int] = [:]
                let plans = try routers.sorted(by: { $0.vmID < $1.vmID }).map { router in
                    guard try vmHasRouterRole(bundle: router.bundle) else {
                        throw RouteApplyError.invalid(
                            "VM \(router.vmID) does not declare roles: [router]"
                        )
                    }
                    let topology = try RouterPlan(
                        vmID: router.vmID,
                        networkRecords: snapshot.networks,
                        attachments: snapshot.attachments
                    )
                    let attached = Set(topology.networks.map(\.name))
                    let hasNatEgress = topology.networks.contains(where: \.natEgress)
                    let selectedPolicies = policies.filter { policy in
                        if let via = policy.via,
                           !ForwardPolicy.matchesVia(vmID: router.vmID, via: via)
                        {
                            return false
                        }
                        guard attached.contains(policy.network) else { return false }
                        var hasInternet = false
                        var hasNonInternet = false
                        for allow in policy.allow {
                            if allow.to == internetPolicyTarget {
                                hasInternet = true
                                continue
                            }
                            hasNonInternet = true
                            guard attached.contains(allow.to) else { return false }
                        }
                        // Internet-only policies bind to the NAT router; mixed
                        // policies (e.g. lan→containers + lan→internet) bind to
                        // the router that owns the non-internet destinations.
                        if hasInternet, !hasNonInternet, !hasNatEgress {
                            return false
                        }
                        policyMatches[policy.name, default: 0] += 1
                        return true
                    }
                    let staticRoutes = DockerBackendRoutes.staticRoutes(
                        forRouter: router.vmID,
                        networks: snapshot.networks,
                        attachments: snapshot.attachments
                    )
                    return try RouterPlan(
                        vmID: topology.vmID,
                        networks: topology.networks,
                        policies: selectedPolicies,
                        staticRoutes: staticRoutes
                    )
                }
                for policy in policies {
                    let matches = policyMatches[policy.name, default: 0]
                    guard matches == 1 else {
                        if let via = policy.via {
                            throw RouteApplyError.invalid(
                                matches == 0
                                    ? "policy \(policy.name) via \(via) does not match a running router"
                                    : "policy \(policy.name) via \(via) matches more than one running router"
                            )
                        }
                        throw RouteApplyError.invalid(
                            matches == 0
                                ? "policy \(policy.name) does not match a running router; set policies.*.via to pin a router"
                                : "policy \(policy.name) matches more than one running router; set policies.*.via to pin a router"
                        )
                    }
                }
                var results: [JSONValue] = []
                var anyChanged = false
                for plan in plans {
                    let helperResult = try HelperRouteClient.run(
                        operation,
                        plan,
                        stateDirectory: stateDirectory
                    )
                    guard case var .object(values) = helperResult,
                          case let .bool(changed)? = values["changed"]
                    else {
                        throw RouteApplyError.guest(
                            "router helper \(plan.vmID) returned invalid status"
                        )
                    }
                    anyChanged = anyChanged || changed
                    values["vm_id"] = .string(plan.vmID)
                    values["networks"] = .array(plan.networks.map(\.json))
                    results.append(.object(values))
                    if operation == .apply {
                        emit(
                            type: "route.applied",
                            data: ["vm_id": .string(plan.vmID), "changed": .bool(changed)]
                        )
                    }
                }
                return JSONRPCResponse(
                    result: .object([
                        "changed": .bool(anyChanged),
                        "routers": .array(results),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return routeErrorResponse(error, request: request)
            }
        default: return nil
        }
    }

    func routeRouterRecords(
        _ records: [HelperRecord], requestedRouter: String?, project: String?, operation: RouterOperation
    ) throws -> [HelperRecord] {
        if let project {
            guard project.range(of: #"^[A-Za-z0-9][A-Za-z0-9._-]*$"#, options: .regularExpression)
                == project.startIndex..<project.endIndex
            else {
                throw RouteApplyError.invalid("invalid route project")
            }
            if let requestedRouter, !requestedRouter.hasPrefix(project + "/") {
                throw RouteApplyError.invalid("router \(requestedRouter) is outside project \(project)")
            }
        }
        guard operation == .status || project != nil || requestedRouter != nil else {
            throw RouteApplyError.invalid("route plan/apply requires project or explicit router")
        }
        return try records.filter { record in
            if let project, !record.vmID.hasPrefix(project + "/") { return false }
            if let requestedRouter { return record.vmID == requestedRouter }
            return try vmHasRouterRole(bundle: record.bundle)
        }
    }

    func vmHasRouterRole(bundle: String) throws -> Bool {
        let manifest = URL(fileURLWithPath: bundle, isDirectory: true)
            .appendingPathComponent("vm.json")
        let data = try Data(contentsOf: manifest)
        guard let root = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let roles = root["roles"] as? [String]
        else {
            throw RouteApplyError.invalid(
                "VM manifest \(manifest.path) has no roles array"
            )
        }
        return roles.contains("router")
    }

    func forwardPolicies(from value: JSONValue?) throws -> [ForwardPolicy] {
        guard let value else { return [] }
        guard case let .array(items) = value else {
            throw RouteApplyError.invalid("policies must be an array")
        }
        return try items.map { item in
            guard case let .object(policy) = item,
                  case let .string(name)? = policy["name"],
                  case let .string(network)? = policy["network"],
                  case let .string(forward)? = policy["forward"]
            else {
                throw RouteApplyError.invalid("invalid forward policy")
            }
            let rawAllows: [JSONValue]
            if case let .array(values)? = policy["allow"] {
                rawAllows = values
            } else {
                rawAllows = []
            }
            let allows = try rawAllows.map { item -> PolicyAllow in
                guard case let .object(allow) = item,
                      case let .string(to)? = allow["to"],
                      case let .string(proto)? = allow["proto"]
                else {
                    throw RouteApplyError.invalid("invalid allow rule in policy \(name)")
                }
                let ports: [Int]
                if case let .array(values)? = allow["ports"] {
                    ports = try values.map {
                        guard case let .number(value) = $0, value.rounded() == value else {
                            throw RouteApplyError.invalid("invalid port in policy \(name)")
                        }
                        return Int(value)
                    }
                } else {
                    ports = []
                }
                return PolicyAllow(to: to, proto: proto, ports: ports)
            }
            return ForwardPolicy(
                name: name,
                network: network,
                forward: forward,
                allow: allows,
                via: {
                    if case let .string(via)? = policy["via"] { return via }
                    return nil
                }()
            )
        }
    }
}
