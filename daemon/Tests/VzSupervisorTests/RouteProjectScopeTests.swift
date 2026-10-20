import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

@Test func routeProjectScopeNeverSelectsAnotherProjectOrPrefixSibling() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("route-scope-\(UUID())")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let bundle = root.appendingPathComponent("router")
    try FileManager.default.createDirectory(at: bundle, withIntermediateDirectories: true)
    try Data(#"{"roles":["router"]}"#.utf8).write(to: bundle.appendingPathComponent("vm.json"))
    let server = try SupervisorServer(stateDirectory: root)
    let records = ["lab/ops", "lab-extra/ops", "neti/main-node", "manual"].map {
        HelperRecord(vmID: $0, state: "running", pid: 1, bundle: bundle.path, lastError: nil)
    }
    for operation in [RouterOperation.plan, .apply] {
        let selected = try server.routeRouterRecords(records, requestedRouter: nil, project: "lab", operation: operation)
        #expect(selected.map(\.vmID) == ["lab/ops"])
        #expect(throws: (any Error).self) {
            try server.routeRouterRecords(records, requestedRouter: "neti/main-node", project: "lab", operation: operation)
        }
        #expect(throws: (any Error).self) {
            try server.routeRouterRecords(records, requestedRouter: nil, project: nil, operation: operation)
        }
        #expect(try server.routeRouterRecords(records, requestedRouter: "manual", project: nil, operation: operation).map(\.vmID) == ["manual"])
    }
    #expect(try server.routeRouterRecords(records, requestedRouter: nil, project: nil, operation: .status).count == 4)
}

@Test func routeProjectScopeRejectsMalformedProjectBeforeReadingAnyVM() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("route-invalid-\(UUID())")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let server = try SupervisorServer(stateDirectory: root)
    for project in ["", "/lab", "lab/other", ".", "-lab", "lab space", "lab\n"] {
        #expect(throws: (any Error).self) {
            try server.routeRouterRecords([], requestedRouter: nil, project: project, operation: .apply)
        }
    }
    #expect(try server.routeRouterRecords([], requestedRouter: nil, project: "lab.dev-1_ok", operation: .plan).isEmpty)
    for method in ["route.plan", "route.apply"] {
        let unscoped = server.dispatchRPC(method: method, params: .object(["policies": .array([])]))
        #expect(unscoped.error?.message.contains("project or explicit router") == true)
        let foreign = server.dispatchRPC(method: method, params: .object([
            "project": .string("lab"), "router": .string("neti/main-node"), "policies": .array([]),
        ]))
        #expect(foreign.error?.message.contains("outside project") == true)
    }
}
