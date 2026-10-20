import Foundation
import Testing
@testable import VzHelper

@Test func routerApplyUsesAtomicNftablesAndPersistentStatus() throws {
    // #filePath: <repo>/daemon/Tests/VzHelperTests/…swift → four parents = <repo>
    let scriptURL = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .deletingLastPathComponent()
        .appendingPathComponent("guest-agent/scripts/router-apply")
    let script = try String(contentsOf: scriptURL, encoding: .utf8)

    #expect(script.contains("/etc/sysctl.d/90-vzctl-router.conf"))
    #expect(script.contains("sysctl -q -w net.ipv4.ip_forward=1"))
    #expect(script.contains("nft -f"))
    #expect(script.contains("/etc/vzctl/vzctl.nft"))
    #expect(script.contains("delete table inet vzctl"))
    #expect(script.contains("cmp -s"))
    // Migration may recognize the exact historical cloud-init line, but must
    // never execute iptables or change another owner's global policy.
    let legacyProvenance = #"expected = "'sh' '-c' 'command -v iptables >/dev/null && iptables -P FORWARD DROP || true'""#
    let executableLines = script.split(separator: "\n").filter { line in
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        return !trimmed.hasPrefix("#") && trimmed != legacyProvenance
    }
    #expect(!executableLines.joined(separator: "\n").contains("iptables"))
    #expect(!script.contains("ssh"))
    #expect(RouterGuestConfigurator.routerStatusScript.contains("nft list table inet vzctl"))
}
