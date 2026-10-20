import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func handleVMRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
        switch request.method {
        case "vm.list":
            let records = stateLock.withLock {
                helpers.values.sorted { $0.vmID < $1.vmID }
            }
            return JSONRPCResponse(
                result: .array(records.map(\.json)),
                id: request.id ?? .null
            )
        case "vm.start":
            do {
                let params = try objectParams(request.params, context: "vm.start")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let bundle = try requiredReconcileString("bundle", from: params)
                guard stateLock.withLock({ helperStartsInFlight.insert(vmID).inserted }) else {
                    throw HelperStartupGate.StartupError("helper start already in progress for \(vmID)")
                }
                defer { _ = stateLock.withLock { helperStartsInFlight.remove(vmID) } }
                let previous = stateLock.withLock { (helpers[vmID], helperProcesses[vmID]) }
                let lockURL = HelperStartupGate.lockURL(vmID: vmID, stateDirectory: stateDirectory)
                if let current = previous.0,
                   current.state == "starting" || current.state == "running",
                   Darwin.kill(pid_t(current.pid), 0) == 0 || errno == EPERM,
                   try HelperStartupGate.isClaimed(by: pid_t(current.pid), lockURL: lockURL)
                {
                    return JSONRPCResponse(result: current.json, id: request.id ?? .null)
                }
                try HelperStartupGate.waitForRelease(lockURL: lockURL, previousProcessAlive: {
                    if let process = previous.1 { return process.isRunning }
                    guard let pid = previous.0?.pid else { return false }
                    return Darwin.kill(pid_t(pid), 0) == 0 || errno == EPERM
                }, timeout: helperStartupTimeout)
                let helperURL = URL(fileURLWithPath: CommandLine.arguments[0])
                    .deletingLastPathComponent()
                    .appendingPathComponent("vz-helper")
                guard FileManager.default.isExecutableFile(atPath: helperURL.path) else {
                    throw ReconcileRPCError.invalid("vz-helper not found at \(helperURL.path)")
                }
                let process = Process()
                process.executableURL = helperURL
                process.arguments = [
                    "run", "--vm-id", vmID, "--bundle", bundle,
                    "--supervisor-sock", socketPath,
                ]
                process.standardOutput = FileHandle.nullDevice
                process.standardError = FileHandle.standardError
                process.terminationHandler = { [weak self] process in
                    self?.helperTerminated(process, vmID: vmID, bundle: bundle)
                }
                stateLock.withLock {
                    _ = helpers.removeValue(forKey: vmID)
                    _ = helperProcesses.removeValue(forKey: vmID)
                    _ = expectedHelperStops.remove(vmID)
                }
                try process.run()
                stateLock.withLock {
                    helperProcesses[vmID] = process
                    // A fast helper.hello may already have supplied a newer state.
                    if helpers[vmID]?.pid != Int(process.processIdentifier) {
                        helpers[vmID] = HelperRecord(
                            vmID: vmID, state: "starting", pid: Int(process.processIdentifier),
                            bundle: bundle, lastError: nil
                        )
                    }
                }
                let started: HelperRecord
                do {
                    try HelperStartupGate.waitForClaim(
                        process: process, lockURL: lockURL, timeout: helperStartupTimeout
                    )
                    started = try confirmedHelperStartRecord(process, vmID: vmID)
                } catch {
                    stateLock.withLock {
                        if helperProcesses[vmID] === process {
                            helpers[vmID] = HelperRecord(
                                vmID: vmID, state: "failed", pid: Int(process.processIdentifier),
                                bundle: bundle, lastError: String(describing: error)
                            )
                        }
                    }
                    if process.isRunning { process.terminate() }
                    throw error
                }
                let result = started.json
                emit(type: "vm.state", data: [
                    "vm_id": .string(vmID),
                    "state": .string(started.state),
                    "pid": .number(Double(process.processIdentifier)),
                    "bundle": .string(bundle),
                ])
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "vm.stop":
            do {
                let params = try objectParams(request.params, context: "vm.stop")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let force = optionalBool("force", from: params) ?? false
                let signal = force ? SIGKILL : SIGTERM
                let record = stateLock.withLock { helpers[vmID] }
                if let record, record.state == "starting" || record.state == "running" {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    let killResult = Darwin.kill(pid_t(record.pid), signal)
                    let killErrno = errno
                    if killResult != 0, killErrno != ESRCH {
                        _ = stateLock.withLock { expectedHelperStops.remove(vmID) }
                        throw SupervisorError.system("stop helper \(vmID)", killErrno)
                    }
                    // Force / ESRCH / already-dead: drop bookkeeping immediately.
                    if force || killErrno == ESRCH || !helperProcessIsRunning(vmID) {
                        stateLock.withLock {
                            helperProcesses.removeValue(forKey: vmID)
                            helpers.removeValue(forKey: vmID)
                        }
                    }
                } else if let process = stateLock.withLock({ helperProcesses[vmID] }),
                          process.isRunning
                {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    if force {
                        _ = Darwin.kill(process.processIdentifier, SIGKILL)
                    } else {
                        process.terminate()
                    }
                    if force {
                        stateLock.withLock {
                            helperProcesses.removeValue(forKey: vmID)
                            helpers.removeValue(forKey: vmID)
                        }
                    }
                } else {
                    // No live helper — clear any leftover bookkeeping so vm.list drops the id.
                    stateLock.withLock {
                        helperProcesses.removeValue(forKey: vmID)
                        helpers.removeValue(forKey: vmID)
                    }
                }
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "state": .string("stopped"),
                        "force": .bool(force),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "vm.purge":
            do {
                let params = try objectParams(request.params, context: "vm.purge")
                let vmID = try requiredReconcileString("vm_id", from: params)
                // Hard-kill + clear helper bookkeeping even if the process is already gone.
                let record = stateLock.withLock { helpers[vmID] }
                if let record, record.state == "starting" || record.state == "running" {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    _ = Darwin.kill(pid_t(record.pid), SIGKILL)
                } else if let process = stateLock.withLock({ helperProcesses[vmID] }),
                          process.isRunning
                {
                    _ = stateLock.withLock { expectedHelperStops.insert(vmID) }
                    _ = Darwin.kill(process.processIdentifier, SIGKILL)
                }
                stateLock.withLock {
                    helperProcesses.removeValue(forKey: vmID)
                    helpers.removeValue(forKey: vmID)
                }

                let detached = try networkRegistry.detachAll(vmID: vmID, vmIsStopped: true)
                let portsRemoved = try database.deletePortForwards(vmID: vmID)
                try reconcileEdgeThrowing(reason: "vm.purge")
                emit(
                    type: "vm.purged",
                    data: [
                        "vm_id": .string(vmID),
                        "detached_networks": .array(detached.map(JSONValue.string)),
                        "ports_removed": .number(Double(portsRemoved)),
                    ]
                )
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "purged": .bool(true),
                        "detached_networks": .array(detached.map(JSONValue.string)),
                        "ports_removed": .number(Double(portsRemoved)),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "vm.network.ensure":
            do {
                let params = try networkParams(request.params)
                let vmID = try requiredString("vm_id", from: params)
                let selection = try networkRegistry.ensureVMNetwork(
                    vmID: vmID,
                    requestedNetwork: try optionalString("network", from: params),
                    vmIsStopped: vmIsStopped(vmID)
                )
                if selection.created {
                    reconcileEdge(reason: "vm.network.ensure")
                    emit(type: "net.attached", data: [
                        "vm_id": .string(selection.attachment.vmID),
                        "network": .string(selection.attachment.networkName),
                        "ip": .string(selection.attachment.ip),
                        "automatic": .bool(selection.automatic),
                    ])
                }
                return JSONRPCResponse(result: selection.json, id: request.id ?? .null)
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "vm.exec":
            do {
                let params = try objectParams(request.params, context: "vm.exec")
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let timeoutSeconds = agentProxyTimeoutSeconds(from: params)
                let result = try HelperAgentClient.run(
                    method: "agent.exec",
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: timeoutSeconds
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "vm.exec_tty":
            do {
                let params = try objectParams(request.params, context: "vm.exec_tty")
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let result = try HelperAgentClient.run(
                    method: "agent.exec_tty",
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 60
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case let method where Self.proxiedAgentMethods.contains(method):
            do {
                let params = try objectParams(request.params, context: method)
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let helperMethod = "agent." + String(method.dropFirst("vm.agent.".count))
                let timeout = method == "vm.agent.ca_inject" ? 65
                    : method == "vm.agent.services.stream" ? 20
                    : 35
                let result = try HelperAgentClient.run(
                    method: helperMethod,
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: timeout
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "vm.mount.list", "vm.mount.add", "vm.mount.remove":
            do {
                let params = try objectParams(request.params, context: request.method)
                let vmID = try requiredReconcileString("vm_id", from: params)
                try requireRunningHelper(vmID: vmID)
                let helperMethod = "mount." + String(request.method.dropFirst("vm.mount.".count))
                let result = try HelperAgentClient.run(
                    method: helperMethod,
                    params: request.params,
                    vmID: vmID,
                    stateDirectory: stateDirectory,
                    timeoutSeconds: 45
                )
                return JSONRPCResponse(result: result, id: request.id ?? .null)
            } catch {
                return routeErrorResponse(error, request: request)
            }
        case "helper.hello", "helper.state":
            guard let record = HelperRecord(params: request.params) else {
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32602, message: "Invalid helper params"),
                    id: request.id ?? .null
                )
            }
            let accepted = stateLock.withLock { () -> Bool in
                if let current = helperProcesses[record.vmID],
                   Int(current.processIdentifier) != record.pid {
                    return false
                }
                if let current = helpers[record.vmID],
                   current.pid != record.pid {
                    return false
                }
                helpers[record.vmID] = record
                return true
            }
            guard accepted else {
                return JSONRPCResponse(result: .object(["ok": .bool(false)]), id: request.id ?? .null)
            }
            emit(
                type: "vm.state",
                data: [
                    "vm_id": .string(record.vmID),
                    "state": .string(record.state),
                    "pid": .number(Double(record.pid)),
                    "bundle": .string(record.bundle),
                    "last_error": record.lastError.map(JSONValue.string) ?? .null,
                ]
            )
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
        case "helper.networks":
            do {
                let params = try objectParams(request.params, context: "helper.networks")
                let vmID = try requiredReconcileString("vm_id", from: params)
                let attachments = try networkRegistry.serializedAttachments(for: vmID)
                return JSONRPCResponse(
                    result: .object([
                        "vm_id": .string(vmID),
                        "attachments": .array(attachments.map(\.json)),
                    ]),
                    id: request.id ?? .null
                )
            } catch {
                return networkErrorResponse(error, request: request)
            }
        case "vm.clock_corrected":
            guard case let .object(params)? = request.params,
                  case .string? = params["vm_id"],
                  case .string? = params["reason"],
                  case .number? = params["observed_guest_unix_ms"],
                  case .number? = params["offset_ms"],
                  params["action"] == .string("stepped")
            else {
                return JSONRPCResponse(
                    error: JSONRPCError(code: -32602, message: "Invalid clock event params"),
                    id: request.id ?? .null
                )
            }
            emit(type: "vm.clock_corrected", data: params)
            return JSONRPCResponse(
                result: .object(["ok": .bool(true)]),
                id: request.id ?? .null
            )
        default: return nil
        }
    }
}

extension SupervisorServer {
    func confirmedHelperStartRecord(_ process: Process, vmID: String) throws -> HelperRecord {
        try stateLock.withLock {
            guard helperProcesses[vmID] === process, process.isRunning,
                  let record = helpers[vmID], record.pid == Int(process.processIdentifier),
                  record.state == "starting" || record.state == "running" else {
                throw HelperStartupGate.StartupError(
                    helpers[vmID]?.lastError ?? "new helper stopped or failed during startup"
                )
            }
            return record
        }
    }

    func helperTerminated(_ process: Process, vmID: String, bundle: String) {
        let termination = stateLock.withLock { () -> (String, String?)? in
            // A delayed completion from an old generation must never clear a
            // new helper's process, record or expected-stop marker.
            guard helperProcesses[vmID] === process else { return nil }
            helperProcesses.removeValue(forKey: vmID)
            let expected = expectedHelperStops.remove(vmID) != nil
            let state = Self.helperTerminationState(expected: expected, status: process.terminationStatus)
            if state == "stopped" {
                helpers.removeValue(forKey: vmID)
                return (state, nil)
            }
            if helpers[vmID]?.state == "failed" {
                return (state, helpers[vmID]?.lastError)
            }
            let message = Self.helperTerminationMessage(
                status: process.terminationStatus, reason: process.terminationReason
            )
            helpers[vmID] = HelperRecord(
                vmID: vmID, state: "failed", pid: Int(process.processIdentifier),
                bundle: bundle, lastError: message
            )
            return (state, message)
        }
        guard let termination else { return }
        emit(type: "vm.state", data: [
            "vm_id": .string(vmID), "state": .string(termination.0),
            "pid": .number(Double(process.processIdentifier)), "bundle": .string(bundle),
            "last_error": termination.1.map(JSONValue.string) ?? .null,
        ])
    }
}

struct HelperRecord: Sendable {
    let vmID: String
    let state: String
    let pid: Int
    let bundle: String
    let lastError: String?
    let updatedAt: String

    init(vmID: String, state: String, pid: Int, bundle: String, lastError: String?) {
        self.vmID = vmID
        self.state = state
        self.pid = pid
        self.bundle = bundle
        self.lastError = lastError
        updatedAt = ISO8601DateFormatter().string(from: Date())
    }

    init?(params: JSONValue?) {
        guard case let .object(values) = params,
              case let .string(vmID)? = values["vm_id"],
              case let .string(state)? = values["state"],
              case let .number(pid)? = values["pid"],
              case let .string(bundle)? = values["bundle"],
              !vmID.isEmpty,
              !bundle.isEmpty,
              pid.isFinite,
              pid >= 1,
              pid <= Double(Int.max),
              pid.rounded() == pid,
              ["starting", "running", "stopped", "failed"].contains(state)
        else {
            return nil
        }
        let lastError: String?
        if case let .string(error)? = values["error"], !error.isEmpty {
            lastError = error
        } else {
            lastError = nil
        }
        self.init(
            vmID: vmID,
            state: state,
            pid: Int(pid),
            bundle: bundle,
            lastError: lastError
        )
    }

    var json: JSONValue {
        .object([
            "vm_id": .string(vmID),
            "state": .string(state),
            "pid": .number(Double(pid)),
            "bundle": .string(bundle),
            "last_error": lastError.map(JSONValue.string) ?? .null,
            "updated_at": .string(updatedAt),
        ])
    }
}
