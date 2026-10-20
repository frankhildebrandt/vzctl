import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func handleApplyRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
        switch request.method {
        case "stacks.list":
            do {
                let rows = try database.listStacks().map(\.json)
                return JSONRPCResponse(
                    result: .object(["stacks": .array(rows)]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.inspect":
            do {
                let params = try objectParams(request.params, context: "stack.inspect")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let state = try database.stackState(stackID: stackID)
                return JSONRPCResponse(result: state.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.begin":
            do {
                let params = try objectParams(request.params, context: "stack.begin")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let desiredHash = try requiredReconcileString("desired_hash", from: params)
                let mode = try requiredReconcileString("mode", from: params)
                guard ["up", "apply", "down", "resume"].contains(mode) else {
                    throw ReconcileRPCError.invalid("invalid reconcile mode")
                }
                let purge = optionalBool("purge", from: params) ?? false
                let payload = try jsonString([
                    "desired_hash": .string(desiredHash),
                    "mode": .string(mode),
                    "purge": .bool(purge),
                ])
                let journal = try database.beginApply(
                    stackID: stackID,
                    holder: holder,
                    desiredHash: desiredHash,
                    payload: payload,
                    resume: mode == "resume"
                )
                let common: [String: JSONValue] = [
                    "invocation_id": .string(journal.id),
                    "mode": .string(mode),
                    "stack_id": .string(stackID),
                    "generation": .number(Double(journal.generation)),
                ]
                emit(type: "apply.started", data: common)
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.heartbeat":
            do {
                let params = try objectParams(request.params, context: "stack.heartbeat")
                try database.heartbeatApply(
                    id: try requiredReconcileString("id", from: params),
                    stackID: try requiredReconcileString("stack_id", from: params),
                    holder: try requiredReconcileString("holder", from: params)
                )
                return JSONRPCResponse(result: .object(["renewed": .bool(true)]), id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.step":
            do {
                let params = try objectParams(request.params, context: "stack.step")
                let requestedStatus = try requiredReconcileString("status", from: params)
                guard ["running", "completed", "failed"].contains(requestedStatus) else {
                    throw ReconcileRPCError.invalid("invalid journal step status")
                }
                let journal = try database.advanceApply(
                    id: try requiredReconcileString("id", from: params),
                    stackID: try requiredReconcileString("stack_id", from: params),
                    holder: try requiredReconcileString("holder", from: params),
                    step: try requiredReconcileString("step", from: params),
                    status: requestedStatus == "completed" ? "running" : requestedStatus,
                    error: optionalReconcileString("error", from: params)
                )
                emit(type: "apply.step", data: [
                    "invocation_id": .string(journal.id),
                    "step": .string(journal.step),
                    "status": .string(requestedStatus == "completed" ? "done" : journal.status),
                    "error": journal.error.map(JSONValue.string) ?? .null,
                ])
                if requestedStatus == "failed" {
                    emit(type: "apply.failed", data: [
                        "invocation_id": .string(journal.id),
                        "mode": .string(journal.operationMode ?? "apply"),
                        "step": .string(journal.step),
                        "exit_code": .number(24),
                        "error": journal.error.map(JSONValue.string) ?? .string("step_failed"),
                    ])
                }
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.finish":
            do {
                let params = try objectParams(request.params, context: "stack.finish")
                let id = try requiredReconcileString("id", from: params)
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let operationMode = try database.stackState(stackID: stackID)
                    .journal?.operationMode ?? "apply"
                guard let resources = params["resources"] else {
                    throw ReconcileRPCError.invalid("missing resources")
                }
                try database.finishApply(
                    id: id,
                    stackID: stackID,
                    holder: holder,
                    resourcesJSON: try jsonString(resources)
                )
                emit(type: "apply.finished", data: [
                    "invocation_id": .string(id),
                    "mode": .string(operationMode),
                    "stack_id": .string(stackID),
                    "exit_code": .number(0),
                ])
                return JSONRPCResponse(
                    result: .object(["ok": .bool(true), "id": .string(id)]),
                    id: request.id ?? .null
                )
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        case "stack.abort":
            do {
                let params = try objectParams(request.params, context: "stack.abort")
                let stackID = try requiredReconcileString("stack_id", from: params)
                let holder = try requiredReconcileString("holder", from: params)
                let journal = try database.abortApply(stackID: stackID, holder: holder)
                emit(type: "apply.finished", data: [
                    "invocation_id": .string(journal.id),
                    "mode": .string("abort"),
                    "stack_id": .string(stackID),
                    "exit_code": .number(0),
                ])
                return JSONRPCResponse(result: journal.json, id: request.id ?? .null)
            } catch {
                return reconcileErrorResponse(error, request: request)
            }
        default: return nil
        }
    }
}

extension StackStateRecord {
    var json: JSONValue {
        .object([
            "resources": .array(resources.map(\.json)),
            "journal": journal.map(\.json) ?? .null,
            "lease": lease.map(\.json) ?? .null,
        ])
    }
}

extension StackResourceRecord {
    var json: JSONValue {
        .object([
            "kind": .string(kind),
            "name": .string(name),
            "labels": .object(labels.mapValues(JSONValue.string)),
            "state": .string(state),
        ])
    }
}

extension JournalRecord {
    var json: JSONValue {
        .object([
            "id": .string(id),
            "stack_id": .string(stackID),
            "generation": .number(Double(generation)),
            "step": .string(step),
            "status": .string(status),
            "payload": .string(payload),
            "error": error.map(JSONValue.string) ?? .null,
            "created_at": .string(createdAt),
            "updated_at": .string(updatedAt),
        ])
    }
}

extension LeaseRecord {
    var json: JSONValue {
        .object([
            "holder": .string(holder),
            "expires_at": .string(expiresAtText),
        ])
    }
}
