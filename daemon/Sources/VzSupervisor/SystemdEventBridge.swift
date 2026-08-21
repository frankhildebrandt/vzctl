import Foundation
import VzDaemonKit

/// Polls guest systemd event buffers when clients subscribe to `vm.systemd.*`.
final class SystemdEventBridge: @unchecked Sendable {
    typealias RunningVMs = () -> [String]
    typealias EmitEvent = (_ type: String, _ data: [String: JSONValue]) -> Void
    typealias HasSubscriber = () -> Bool

    private let runningVMs: RunningVMs
    private let hasSubscriber: HasSubscriber
    private let emit: EmitEvent
    private let stateDirectory: URL
    private let queue = DispatchQueue(label: "com.vzctl.systemd-events")
    private var timer: DispatchSourceTimer?
    private var cursors: [String: String] = [:]

    init(
        stateDirectory: URL,
        runningVMs: @escaping RunningVMs,
        hasSubscriber: @escaping HasSubscriber,
        emit: @escaping EmitEvent
    ) {
        self.stateDirectory = stateDirectory
        self.runningVMs = runningVMs
        self.hasSubscriber = hasSubscriber
        self.emit = emit
    }

    func start() {
        queue.async {
            let timer = DispatchSource.makeTimerSource(queue: self.queue)
            timer.schedule(deadline: .now() + 2, repeating: 2)
            timer.setEventHandler { [weak self] in self?.poll() }
            timer.resume()
            self.timer = timer
        }
    }

    func stop() {
        queue.sync {
            timer?.cancel()
            timer = nil
            cursors.removeAll()
        }
    }

    private func poll() {
        guard hasSubscriber() else { return }
        for vmID in runningVMs() {
            pollVM(vmID)
        }
    }

    private func pollVM(_ vmID: String) {
        let since = queue.sync { cursors[vmID] }
        var params: [String: JSONValue] = ["vm_id": .string(vmID), "limit": .number(100)]
        if let since, !since.isEmpty {
            params["since"] = .string(since)
        }
        let result: JSONValue
        do {
            result = try HelperAgentClient.run(
                method: "agent.systemd.events",
                params: .object(params),
                vmID: vmID,
                stateDirectory: stateDirectory,
                timeoutSeconds: 10
            )
        } catch {
            return
        }
        guard case let .object(payload) = result,
              case let .array(events)? = payload["events"]
        else {
            return
        }
        if case let .string(cursor)? = payload["cursor"], !cursor.isEmpty {
            queue.sync { cursors[vmID] = cursor }
        }
        for event in events {
            guard case let .object(record) = event else { continue }
            emit(
                "vm.systemd.unit",
                systemdEventData(vmID: vmID, record: record)
            )
        }
    }

    private func systemdEventData(vmID: String, record: [String: JSONValue]) -> [String: JSONValue] {
        var data: [String: JSONValue] = ["vm_id": .string(vmID)]
        for key in ["unit", "unit_type", "load", "active", "sub", "reason"] {
            if case let .string(value)? = record[key], !value.isEmpty {
                data[key] = .string(value)
            }
        }
        return data
    }
}
