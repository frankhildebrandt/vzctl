import Foundation
import Testing
@testable import VzEdge

@Test func planSharesCaddyWhenBinaryAndBackendPortsMatch() {
    let intents = [
        caddyIntent(project: "neti"),
        caddyIntent(project: "iq-store"),
    ]
    let children = CaddyProcessPlan.children(from: intents)
    #expect(children.count == 1)
    #expect(children[0].name == "caddy-shared")
    #expect(children[0].project == "shared")
    #expect(children[0].shared)
    #expect(children[0].backendHTTP == 18_080)
    #expect(children[0].caddyfile.contains("https://neti.example"))
    #expect(children[0].caddyfile.contains("https://store.example"))
    #expect(children[0].reapWorkDirs == intents.map(\.workDir))
}

@Test func planKeepsPerProjectCaddyWhenBackendPortsDiffer() {
    let intents = [
        caddyIntent(project: "neti", backendHTTP: 18_080, backendHTTPS: 18_443),
        caddyIntent(project: "custom", backendHTTP: 9_443, backendHTTPS: 9_443),
    ]
    let children = CaddyProcessPlan.children(from: intents)
    #expect(children.map(\.name) == ["caddy-neti", "caddy-custom"])
    #expect(children.allSatisfy { !$0.shared })
    #expect(children.map(\.project) == ["neti", "custom"])
    #expect(children[0].configPath == intents[0].configPath)
    #expect(children[1].configPath == intents[1].configPath)
    #expect(children[0].caddyfile.contains("https://neti.example"))
    #expect(!children[0].caddyfile.contains("https://custom.example"))
    #expect(children.allSatisfy { $0.reapWorkDirs.isEmpty })
}

@Test func planDoesNotShareWhenBinariesDiffer() {
    let intents = [
        caddyIntent(project: "neti", binary: "/opt/caddy"),
        caddyIntent(project: "other", binary: "/usr/local/caddy"),
    ]
    let children = CaddyProcessPlan.children(from: intents)
    #expect(children.map(\.name) == ["caddy-neti", "caddy-other"])
}

@Test func planReturnsEmptyWhenThereAreNoIntents() {
    #expect(CaddyProcessPlan.children(from: []).isEmpty)
}

@Test func uniqueGatewayBindingsDropsDuplicateLoopbackKeys() {
    let http = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1", port: 80,
        backendHost: "127.0.0.1", backendPort: 18_080
    )
    let https = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1", port: 443,
        backendHost: "127.0.0.1", backendPort: 18_443
    )
    let duplicateHTTP = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1", port: 80,
        backendHost: "127.0.0.1", backendPort: 18_080
    )
    let guest = HostGatewayIngressProxy.Binding(
        gatewayIP: "10.90.0.1", port: 443,
        backendHost: "127.0.0.1", backendPort: 18_443
    )
    let unique = HostGatewayIngressProxy.uniqueBindings([http, https, duplicateHTTP, https, guest])
    #expect(unique.map(\.key) == ["127.0.0.1:80", "127.0.0.1:443", "10.90.0.1:443"])
}

private func caddyIntent(
    project: String,
    backendHTTP: UInt16 = 18_080,
    backendHTTPS: UInt16 = 18_443,
    binary: String = "/opt/caddy"
) -> CaddyIntent {
    CaddyIntent(
        project: project,
        caddyfile: """
        {
        \tauto_https off
        \thttp_port \(backendHTTP)
        \thttps_port \(backendHTTPS)
        }

        https://\(project == "iq-store" ? "store" : project).example {
        \treverse_proxy 10.0.0.2:80
        }
        """,
        binary: binary,
        backendHTTP: backendHTTP,
        backendHTTPS: backendHTTPS,
        workDir: URL(fileURLWithPath: "/tmp/ingress/\(project)"),
        configPath: "/tmp/ingress/\(project)/Caddyfile.1"
    )
}
