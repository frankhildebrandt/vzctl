import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func helperProcessIsRunning(_ vmID: String) -> Bool {
        stateLock.withLock {
            helperProcesses[vmID]?.isRunning == true
        }
    }

    func requireRunningHelper(vmID: String) throws {
        let running = stateLock.withLock {
            helpers[vmID]?.state == "running"
        }
        guard running else {
            throw RouteApplyError.unavailable("VM \(vmID) is not running")
        }
    }

    func agentProxyTimeoutSeconds(from params: [String: JSONValue]) -> Int {
        guard case let .number(value)? = params["timeout_ms"],
              value.rounded() == value,
              value > 0
        else {
            return 35
        }
        return max(35, Int(value / 1_000) + 5)
    }

    func vmIsStopped(_ vmID: String) -> Bool {
        let reportsRunning = stateLock.withLock {
            guard let helper = helpers[vmID] else { return false }
            return helper.state == "starting" || helper.state == "running"
        }
        guard !reportsRunning else { return false }

        let path = stateDirectory
            .appendingPathComponent("helpers", isDirectory: true)
            .appendingPathComponent("\(StateFileName.component(vmID)).lock")
            .path
        let descriptor = Darwin.open(path, O_RDONLY)
        if descriptor < 0 {
            return errno == ENOENT
        }
        defer { Darwin.close(descriptor) }
        if flock(descriptor, LOCK_EX | LOCK_NB) == 0 {
            flock(descriptor, LOCK_UN)
            return true
        }
        return false
    }

    func networkParams(_ value: JSONValue?) throws -> [String: JSONValue] {
        guard case let .object(params)? = value else {
            throw NetworkRegistryError.invalid("network params must be an object")
        }
        return params
    }

    func requiredString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String {
        guard case let .string(value)? = params[key], !value.isEmpty else {
            throw NetworkRegistryError.invalid("missing or invalid \(key)")
        }
        return value
    }

    func optionalString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String? {
        guard let raw = params[key], raw != .null else { return nil }
        guard case let .string(value) = raw, !value.isEmpty else {
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
        return value
    }

    func optionalStringArray(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> [String]? {
        guard let raw = params[key], raw != .null else { return nil }
        guard case let .array(items) = raw else {
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
        return try items.map { item in
            guard case let .string(value) = item else {
                throw NetworkRegistryError.invalid("invalid \(key) entry")
            }
            return value
        }
    }

    func requiredPort(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> UInt16 {
        guard let raw = params[key] else {
            throw NetworkRegistryError.invalid("missing \(key)")
        }
        switch raw {
        case let .number(value):
            guard value >= 1, value <= 65535, value.rounded() == value else {
                throw NetworkRegistryError.invalid("invalid \(key)")
            }
            return UInt16(value)
        case let .string(value):
            guard let parsed = UInt16(value), parsed > 0 else {
                throw NetworkRegistryError.invalid("invalid \(key)")
            }
            return parsed
        default:
            throw NetworkRegistryError.invalid("invalid \(key)")
        }
    }

    func optionalPort(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> UInt16? {
        guard let raw = params[key], raw != .null else { return nil }
        return try requiredPort(key, from: params)
    }

    func labels(from params: [String: JSONValue]) throws -> [String: String] {
        guard let raw = params["labels"], raw != .null else { return [:] }
        guard case let .object(values) = raw else {
            throw NetworkRegistryError.invalid("labels must be an object")
        }
        var labels: [String: String] = [:]
        for (key, rawValue) in values {
            guard case let .string(value) = rawValue else {
                throw NetworkRegistryError.invalid("label \(key) must be a string")
            }
            labels[key] = value
        }
        return labels
    }

    func networkErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let networkError: NetworkRegistryError
        if let error = error as? NetworkRegistryError {
            networkError = error
        } else if let error = error as? NetworkValidationError {
            networkError = .invalid(error.description)
        } else {
            networkError = .runtime(String(describing: error))
        }
        return JSONRPCResponse(
            error: JSONRPCError(code: networkError.rpcCode, message: networkError.description),
            id: request.id ?? .null
        )
    }

    func routeErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let routeError = error as? RouteApplyError
            ?? .guest(String(describing: error))
        return JSONRPCResponse(
            error: JSONRPCError(code: routeError.rpcCode, message: routeError.description),
            id: request.id ?? .null
        )
    }

    func reconcileErrorResponse(
        _ error: Error,
        request: JSONRPCRequest
    ) -> JSONRPCResponse {
        let code: Int
        let message: String
        switch error {
        case let ReconcileDatabaseError.incomplete(journal):
            code = 5
            message = "incomplete journal \(journal.id) at \(journal.step); "
                + "run `vzctl apply --resume` or `vzctl apply --abort`"
        case let ReconcileDatabaseError.leaseHeld(lease):
            code = 6
            message = "stack lease held by \(lease.holder) until \(lease.expiresAtText)"
        case ReconcileDatabaseError.generationChanged:
            code = 5
            message = "desired config changed; run `vzctl apply --abort` before applying"
        case ReconcileDatabaseError.noIncomplete:
            code = 5
            message = "no incomplete journal"
        case ReconcileDatabaseError.leaseLost:
            code = 6
            message = "stack lease was lost"
        case let ReconcileRPCError.invalid(value):
            code = -32602
            message = value
        default:
            code = -32010
            message = String(describing: error)
        }
        return JSONRPCResponse(
            error: JSONRPCError(code: code, message: message),
            id: request.id ?? .null
        )
    }

    func objectParams(
        _ value: JSONValue?,
        context: String
    ) throws -> [String: JSONValue] {
        guard case let .object(params)? = value else {
            throw ReconcileRPCError.invalid("\(context) params must be an object")
        }
        return params
    }

    func requiredReconcileString(
        _ key: String,
        from params: [String: JSONValue]
    ) throws -> String {
        guard case let .string(value)? = params[key], !value.isEmpty else {
            throw ReconcileRPCError.invalid("missing or invalid \(key)")
        }
        return value
    }

    func optionalReconcileString(
        _ key: String,
        from params: [String: JSONValue]
    ) -> String? {
        guard case let .string(value)? = params[key] else { return nil }
        return value
    }

    func optionalBool(
        _ key: String,
        from params: [String: JSONValue]
    ) -> Bool? {
        guard case let .bool(value)? = params[key] else { return nil }
        return value
    }

    func jsonString(_ value: JSONValue) throws -> String {
        String(decoding: try JSONEncoder().encode(value), as: UTF8.self)
    }

    func jsonString(_ value: [String: JSONValue]) throws -> String {
        try jsonString(.object(value))
    }
}

enum ReconcileRPCError: Error {
    case invalid(String)
}
