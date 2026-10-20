import Darwin
import Foundation
import VzDaemonKit

extension SupervisorServer {
    func subscribe(_ client: Int32, request: JSONRPCRequest) {
        let expression: String?
        if request.params == nil {
            expression = nil
        } else if case let .object(params)? = request.params {
            if params["filter"] == nil || params["filter"] == .null {
                expression = nil
            } else if case let .string(filter)? = params["filter"] {
                expression = filter
            } else {
                writeSubscriptionError(client, request: request)
                return
            }
        } else {
            writeSubscriptionError(client, request: request)
            return
        }

        guard let filter = try? EventFilter(expression) else {
            writeSubscriptionError(client, request: request)
            return
        }

        let id = UUID()
        let response = JSONRPCResponse(
            result: .object(["ok": .bool(true), "v": .number(1)]),
            id: request.id ?? .null
        )
        guard let encoded = try? JSONRPCFraming.encode(response) else { return }
        let registered = stateLock.withLock { () -> Bool in
            guard writeAll(encoded, to: client) else { return false }
            subscribers[id] = EventSubscriber(fd: client, filter: filter)
            return true
        }
        guard registered else { return }
        defer {
            _ = stateLock.withLock {
                subscribers.removeValue(forKey: id)
            }
        }

        var byte: UInt8 = 0
        while Darwin.read(client, &byte, 1) > 0 {}
    }

    func writeSubscriptionError(_ client: Int32, request: JSONRPCRequest) {
        let response = JSONRPCResponse(
            error: JSONRPCError(code: -32602, message: "Invalid event filter"),
            id: request.id ?? .null
        )
        if let encoded = try? JSONRPCFraming.encode(response) {
            _ = writeAll(encoded, to: client)
        }
    }

    func emit(type: String, data: [String: JSONValue]) {
        let event = EventEnvelope(type: type, data: data)
        guard let encoded = try? JSONRPCFraming.encode(event) else { return }
        let listeners: [EventListener] = stateLock.withLock {
            let failed = subscribers.compactMap { id, subscriber -> UUID? in
                guard subscriber.filter.matches(type) else { return nil }
                return writeAll(encoded, to: subscriber.fd) ? nil : id
            }
            for id in failed {
                subscribers.removeValue(forKey: id)
            }
            return Array(eventListeners.values)
        }
        for listener in listeners where listener.filter.matches(type) {
            listener.handler(event)
        }
    }
}

struct EventSubscriber: Sendable {
    let fd: Int32
    let filter: EventFilter
}

struct EventListener: Sendable {
    let filter: EventFilter
    let handler: @Sendable (EventEnvelope) -> Void
}
