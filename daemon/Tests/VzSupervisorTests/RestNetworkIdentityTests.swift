import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

@Test func networkRESTPreservesEncodedIdentityAndProjectThroughAttachDetachDelete() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("rest-net-\(UUID())")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let database = try StateDatabase(path: root.appendingPathComponent("state.sqlite").path)
    // Logical Docker networks require no vmnet handles or system changes.
    try database.insertNetwork(NetworkRecord(
        name: "lan", id: "lan", cidr: "10.92.0.0/24", natEgress: false,
        backend: "docker", project: "iq-store", stack: "iq-store:iq-store"
    ))
    let server = try SupervisorServer(stateDirectory: root)
    let created = server.dispatchRPC(method: "net.create", params: .object([
        "name": .string("lan"), "cidr": .string("10.123.0.0/24"),
        "backend": .string("docker"), "nat_egress": .bool(false),
        "project": .string("controller-lab"), "stack": .string("controller-lab:controller-lab"),
    ]))
    #expect(created.error == nil)
    let router = RestRouter(jobs: RestJobRunner(stateDirectory: root), database: server.database, stateDirectory: root)
    router.server = server
    func request(_ method: String, _ path: String, query: [String: String] = [:], body: String = "") -> RestHTTPResponse {
        router.handle(RestHTTPRequest(method: method, path: path, query: query, headers: [:], body: Data(body.utf8)))
    }
    let attached = request("POST", "/v1/nets/controller-lab%2Flan/attach",
                           body: #"{"vm_id":"controller-lab/client","project":"controller-lab","ip":"10.123.0.10"}"#)
    #expect(attached.status == 200)
    #expect(try server.database.attachments().first?.networkID == "controller-lab/lan")
    let ambiguous = request("DELETE", "/v1/nets/lan")
    #expect(ambiguous.status >= 400)
    #expect(try server.database.networks().count == 2)
    let detached = request("POST", "/v1/nets/controller-lab%2Flan/detach",
                           body: #"{"vm_id":"controller-lab/client","project":"controller-lab"}"#)
    #expect(detached.status == 200)
    let deleted = request("DELETE", "/v1/nets/controller-lab%2Flan", query: ["project": "controller-lab"])
    #expect(deleted.status == 200)
    #expect(try server.database.networks().map(\.id) == ["lan"])
    let legacyDelete = request("DELETE", "/v1/nets/lan", query: ["project": "iq-store"])
    #expect(legacyDelete.status == 200)
    #expect(try server.database.networks().isEmpty)
}
