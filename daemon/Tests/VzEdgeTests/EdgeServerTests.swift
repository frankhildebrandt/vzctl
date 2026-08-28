import Foundation
import Testing
import VzDaemonKit
@testable import VzEdge

@Test func edgePersistsAndRestoresLastKnownGoodManifest() throws {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-edge-test-\(UUID().uuidString)", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let configuration = DNSConfiguration(
        hostAddress: "127.0.0.1", hostPort: 25_353,
        guestPort: 25_354, ttl: 15, upstream: "system"
    )
    let desired: JSONValue = .object([
        "network_snapshot": .object(["networks": .array([]), "attachments": .array([])]),
        "host_services": .array([]),
        "port_forwards": .array([]),
        "ingress": .array([]),
        "oidc": .array([]),
    ])

    let first = try EdgeServer(stateDirectory: root, dnsConfiguration: configuration)
    let applied = try first.reconcile(generation: 1, digest: "one", desired: desired)
    #expect(applied["generation"] == .number(1))
    first.stop()

    let restored = try EdgeServer(stateDirectory: root, dnsConfiguration: configuration)
    defer { restored.stop() }
    #expect(restored.status()["generation"] == .number(1))
    #expect(restored.status()["digest"] == .string("one"))
    let attributes = try FileManager.default.attributesOfItem(
        atPath: root.appendingPathComponent("runtime/edge/manifest.json").path
    )
    #expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600)
}

@Test func edgeRejectsStaleAndConflictingGenerations() throws {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-edge-generation-\(UUID().uuidString)", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let server = try EdgeServer(
        stateDirectory: root,
        dnsConfiguration: DNSConfiguration(
            hostAddress: "127.0.0.1", hostPort: 25_355,
            guestPort: 25_356, ttl: 15, upstream: "system"
        )
    )
    defer { server.stop() }
    let desired: JSONValue = .object([
        "network_snapshot": .object(["networks": .array([]), "attachments": .array([])]),
    ])
    _ = try server.reconcile(generation: 2, digest: "two", desired: desired)
    #expect(throws: EdgeServerError.self) {
        try server.reconcile(generation: 1, digest: "old", desired: desired)
    }
    #expect(throws: EdgeServerError.self) {
        try server.reconcile(generation: 2, digest: "different", desired: desired)
    }
}

@Test func ingressGuestListenerSkipsDegradeWhenLoopbackIsActive() throws {
    let loopbackHTTP = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1",
        port: 80,
        backendHost: "127.0.0.1",
        backendPort: 18_080
    )
    let loopbackHTTPS = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1",
        port: 443,
        backendHost: "127.0.0.1",
        backendPort: 18_443
    )
    let guest = HostGatewayIngressProxy.Binding(
        gatewayIP: "10.90.0.1",
        port: 443,
        backendHost: "127.0.0.1",
        backendPort: 18_443
    )
    let result = HostGatewayIngressProxy.EnsureResult(
        active: [loopbackHTTP, loopbackHTTPS],
        skipped: [(guest, "dns-bind: busy")]
    )
    let warning = try evaluateIngressGatewayEnsure(
        result,
        loopbackGateways: ["127.0.0.1"],
        httpPort: 80,
        httpsPort: 443
    )
    #expect(warning?.contains("10.90.0.1:443") == true)

    #expect(throws: EdgeServerError.self) {
        _ = try evaluateIngressGatewayEnsure(
            HostGatewayIngressProxy.EnsureResult(
                active: [],
                skipped: [(loopbackHTTPS, "bind failed")]
            ),
            loopbackGateways: ["127.0.0.1"],
            httpPort: 80,
            httpsPort: 443
        )
    }
}

private extension JSONValue {
    subscript(_ key: String) -> JSONValue? {
        guard case let .object(values) = self else { return nil }
        return values[key]
    }
}
