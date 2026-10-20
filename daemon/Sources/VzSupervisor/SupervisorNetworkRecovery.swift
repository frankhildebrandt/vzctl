import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func vzNetHealth() -> (ok: Bool, json: JSONValue) {
        let client = VzNetClient(
            socketPath: VzNetClient.defaultSocketPath(stateDirectory: stateDirectory)
        )
        do {
            let health = try client.health()
            return (
                health.ok,
                .object([
                    "ok": .bool(health.ok),
                    "version": .string(health.version),
                    "networks": .number(Double(health.networks)),
                    "socket": .string(client.socketPath),
                ])
            )
        } catch {
            return (
                false,
                .object([
                    "ok": .bool(false),
                    "socket": .string(client.socketPath),
                    "error": .string(String(describing: error)),
                ])
            )
        }
    }

    func probeNetworkRecovery() -> NetworkRecoveryResult {
        let snapshot: NetworkSnapshot
        do {
            snapshot = try networkRegistry.snapshot()
        } catch {
            return NetworkRecoveryResult(
                internalOK: false,
                hostEgress: .unknown,
                networkEgress: [:],
                conflicts: [],
                error: "network snapshot: \(error)"
            )
        }
        let activeNetworks = snapshot.networks.filter {
            $0.runtimeState == "active" && !$0.isDockerBackend
        }
        let conflicts = HostRouteScanner.conflicts(networks: activeNetworks)
        var internalOK = true
        var errors: [String] = []

        do {
            let client = VzNetClient(
                socketPath: VzNetClient.defaultSocketPath(stateDirectory: stateDirectory),
                timeoutSeconds: 5
            )
            let verifications = try client.verify()
            if verifications.contains(where: { !$0.ok }) {
                internalOK = false
                errors.append("vz-net verification failed")
            }
        } catch {
            internalOK = false
            errors.append("vz-net: \(error)")
        }

        do {
            let edgeStatus = try reconcileEdgeThrowing(reason: "host.network_changed")
            guard case let .object(values) = edgeStatus,
                  values["ok"] == .bool(true)
            else {
                throw NetworkRegistryError.runtime("vz-edge status is degraded")
            }
        } catch {
            internalOK = false
            errors.append("vz-edge: \(error)")
        }

        let running = stateLock.withLock {
            Set(helpers.values.filter { $0.state == "running" }.map(\.vmID))
        }
        var representatives: [String: (attachment: NetworkAttachmentRecord, canProbe: Bool)] = [:]
        for network in activeNetworks {
            guard let attachment = snapshot.attachments.first(where: {
                $0.networkID == network.id && running.contains($0.vmID)
            }) else { continue }
            do {
                let version = try HelperAgentClient.run(
                    method: "agent.version",
                    params: .object(["vm_id": .string(attachment.vmID)]),
                    vmID: attachment.vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 3
                )
                representatives[network.id] = (
                    attachment, Self.agentSupportsNetworkProbe(version)
                )
            } catch {
                internalOK = false
                errors.append("guest agent \(network.name): \(error)")
            }
        }

        let policies = (try? database.networkResiliencePolicies()) ?? []
        let defaultProbeURL = URL(string: "https://captive.apple.com/")!
        let enabledPolicies = policies.filter(\.probeEnabled)
        let hostProbeURL = enabledPolicies.first.flatMap { URL(string: $0.probeURL) }
            ?? (policies.isEmpty ? defaultProbeURL : nil)
        let hostEgress = hostProbeURL.map { HostEgressProber().probe(url: $0) } ?? .unknown
        var guestEgress: [String: NetworkEgressResult] = [:]
        if hostEgress.classification == "online" {
            for network in activeNetworks where network.natEgress {
                let policy = policies.first { $0.project == network.project }
                if policy?.probeEnabled == false {
                    guestEgress[network.id] = .unknown
                    continue
                }
                let probeURL = policy.flatMap { URL(string: $0.probeURL) } ?? defaultProbeURL
                guard let representative = representatives[network.id] else {
                    guestEgress[network.id] = .unknown
                    continue
                }
                let attachment = representative.attachment
                do {
                    guard representative.canProbe else {
                        // Older agents remain compatible. Their missing optional probe
                        // must not turn a healthy dataplane into a false degradation.
                        guestEgress[network.id] = .unknown
                        continue
                    }
                    let raw = try HelperAgentClient.run(
                        method: "agent.network_probe",
                        params: .object([
                            "vm_id": .string(attachment.vmID),
                            "url": .string(probeURL.absoluteString),
                            "timeout_ms": .number(5_000),
                        ]),
                        vmID: attachment.vmID,
                        stateDirectory: stateDirectory,
                        timeoutSeconds: 10
                    )
                    guestEgress[network.id] = try Self.parseEgressResult(raw)
                } catch {
                    guestEgress[network.id] = NetworkEgressResult(
                        classification: "offline",
                        phase: "agent",
                        statusCode: nil,
                        latencyMS: 0,
                        errorCode: "agent"
                    )
                    errors.append("guest egress \(network.name): \(error)")
                }
            }
        }
        return NetworkRecoveryResult(
            internalOK: internalOK,
            hostEgress: hostEgress,
            networkEgress: guestEgress,
            conflicts: conflicts,
            error: errors.isEmpty ? nil : errors.joined(separator: "; ")
        )
    }

    static func agentSupportsNetworkProbe(_ value: JSONValue) -> Bool {
        guard case let .object(values) = value,
              case let .array(capabilities)? = values["capabilities"]
        else { return false }
        return capabilities.contains(.string("network_probe"))
    }

    static func parseEgressResult(_ value: JSONValue) throws -> NetworkEgressResult {
        guard case let .object(values) = value,
              case let .string(classification)? = values["classification"],
              case let .string(phase)? = values["phase"],
              case let .number(latency)? = values["latency_ms"]
        else {
            throw RouteApplyError.guest("network_probe returned invalid result")
        }
        let statusCode: Int?
        if case let .number(value)? = values["status_code"] {
            statusCode = Int(value)
        } else {
            statusCode = nil
        }
        let errorCode: String?
        if case let .string(value)? = values["error_code"] {
            errorCode = value
        } else {
            errorCode = nil
        }
        return NetworkEgressResult(
            classification: classification,
            phase: phase,
            statusCode: statusCode,
            latencyMS: Int64(latency),
            errorCode: errorCode
        )
    }

    func attemptNetworkFallback(_ result: NetworkRecoveryResult) -> Bool {
        guard result.internalOK,
              result.hostEgress.classification == "online",
              result.conflicts.isEmpty
        else { return false }
        let failedNetworks = result.networkEgress
            .filter { $0.value.classification == "offline" }
            .map(\.key)
            .sorted()
        guard !failedNetworks.isEmpty,
              let snapshot = try? networkRegistry.snapshot(),
              let policies = try? database.networkResiliencePolicies(),
              Self.networkFallbackPolicyAllows(
                  networks: failedNetworks,
                  snapshot: snapshot,
                  policies: policies
              )
        else { return false }

        var running: [String: String] = [:]
        for name in failedNetworks {
            let attachments = snapshot.attachments.filter { $0.networkID == name }
            let records = stateLock.withLock { helpers }
            for attachment in attachments {
                if let record = records[attachment.vmID], record.state == "running" {
                    running[record.vmID] = record.bundle
                }
            }
        }
        guard !running.isEmpty else { return false }

        emit(type: "network.fallback_restart", data: [
            "networks": .array(failedNetworks.map(JSONValue.string)),
            "vms": .array(running.keys.sorted().map(JSONValue.string)),
        ])
        for vmID in running.keys.sorted().reversed() {
            let response = dispatchRPC(
                method: "vm.stop",
                params: .object(["vm_id": .string(vmID), "force": .bool(false)])
            )
            if response.error != nil { return false }
        }
        let stopDeadline = DispatchTime.now().uptimeNanoseconds + 30_000_000_000
        while DispatchTime.now().uptimeNanoseconds < stopDeadline {
            let active = stateLock.withLock {
                running.keys.contains { vmID in
                    helpers[vmID].map { $0.state == "starting" || $0.state == "running" } ?? false
                        || helperProcesses[vmID]?.isRunning == true
                }
            }
            if !active { break }
            Thread.sleep(forTimeInterval: 0.1)
        }
        let stillActive = stateLock.withLock {
            running.keys.contains { helperProcesses[$0]?.isRunning == true }
        }
        guard !stillActive else { return false }

        do {
            for name in failedNetworks {
                try networkRegistry.recreateRuntime(name: name)
            }
        } catch {
            emit(type: "network.degraded", data: [
                "state": .string("degraded"),
                "error": .string("fallback network recreate failed: \(error)"),
            ])
            return false
        }

        for vmID in running.keys.sorted() {
            guard let bundle = running[vmID] else { continue }
            let response = dispatchRPC(
                method: "vm.start",
                params: .object([
                    "vm_id": .string(vmID),
                    "bundle": .string(bundle),
                ])
            )
            if response.error != nil { return false }
        }
        let startDeadline = DispatchTime.now().uptimeNanoseconds + 30_000_000_000
        while DispatchTime.now().uptimeNanoseconds < startDeadline {
            let ready = stateLock.withLock {
                running.keys.allSatisfy { helpers[$0]?.state == "running" }
            }
            if ready { return true }
            Thread.sleep(forTimeInterval: 0.2)
        }
        return false
    }

    static func networkFallbackPolicyAllows(
        networks names: [String],
        snapshot: NetworkSnapshot,
        policies: [NetworkResiliencePolicyRecord]
    ) -> Bool {
        names.allSatisfy { name in
            guard let network = snapshot.networks.first(where: { $0.id == name }),
                  let project = network.project,
                  let stack = network.stack,
                  policies.contains(where: {
                      $0.project == project && $0.stack == stack && $0.restartVMs
                  })
            else { return false }
            let attachments = snapshot.attachments.filter { $0.networkID == name }
            return !attachments.isEmpty && attachments.allSatisfy {
                $0.project == project && $0.stack == stack
            }
        }
    }
}
