import Testing
import Foundation
import VzDaemonKit
@testable import VzSupervisor

@Test func supervisorRoutesCAInjectToHelpers() {
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.ca_inject"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.stats"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.services.list"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.services.http"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.services.stream"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.systemd.status"))
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.systemd.list"))
}

@Test func failedHelperStateIsVisibleWithConcreteError() throws {
    let state = URL(
        fileURLWithPath: "/tmp/vzsup-\(String(UUID().uuidString.prefix(8)))",
        isDirectory: true
    )
    defer { try? FileManager.default.removeItem(at: state) }
    let server = try SupervisorServer(stateDirectory: state)
    defer { server.stop() }

    let report = server.dispatchRPC(
        method: "helper.state",
        params: .object([
            "vm_id": .string("monitos/monitos-main"),
            "state": .string("failed"),
            "pid": .number(42),
            "bundle": .string("/tmp/bundle"),
            "error": .string("console socket path is too long"),
        ])
    )
    #expect(report.error == nil)

    let listed = server.dispatchRPC(method: "vm.list")
    guard case let .array(records)? = listed.result,
          case let .object(record)? = records.first
    else {
        Issue.record("expected one vm.list record")
        return
    }
    #expect(record["state"] == .string("failed"))
    #expect(record["last_error"] == .string("console socket path is too long"))
}

@Test func incompleteJournalErrorNamesTheApplyRecoveryCommands() throws {
    let state = URL(
        fileURLWithPath: "/tmp/vzsup-\(String(UUID().uuidString.prefix(8)))",
        isDirectory: true
    )
    defer { try? FileManager.default.removeItem(at: state) }
    let server = try SupervisorServer(stateDirectory: state)
    defer { server.stop() }
    let params: JSONValue = .object([
        "stack_id": .string("monitos:monitos"),
        "holder": .string("test:42"),
        "desired_hash": .string("abc"),
        "mode": .string("up"),
    ])

    #expect(server.dispatchRPC(method: "stack.begin", params: params).error == nil)
    let blocked = server.dispatchRPC(method: "stack.begin", params: params)

    #expect(blocked.error?.code == 5)
    #expect(blocked.error?.message.contains("vzctl apply --resume") == true)
    #expect(blocked.error?.message.contains("vzctl apply --abort") == true)
}

@Test func unexpectedHelperTerminationHasImmediateDiagnostic() {
    #expect(
        SupervisorServer.helperTerminationMessage(status: 1, reason: .exit)
            == "helper exited with status 1 before reporting an error"
    )
    #expect(
        SupervisorServer.helperTerminationMessage(status: 9, reason: .uncaughtSignal)
            == "helper terminated by signal 9 before reporting an error"
    )
}

@Test func expectedHelperTerminationIsReportedAsStopped() {
    #expect(SupervisorServer.helperTerminationState(expected: true, status: 9) == "stopped")
    #expect(SupervisorServer.helperTerminationState(expected: false, status: 0) == "stopped")
    #expect(SupervisorServer.helperTerminationState(expected: false, status: 1) == "failed")
}
