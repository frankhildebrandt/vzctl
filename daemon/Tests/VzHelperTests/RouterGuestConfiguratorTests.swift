import Testing
@testable import VzHelper

@Test func routerApplyUsesAtomicNftablesAndPersistentStatus() {
    let script = RouterGuestConfigurator.routerApplyScript

    #expect(script.contains("/etc/sysctl.d/90-vzctl-router.conf"))
    #expect(script.contains("sysctl -q -w net.ipv4.ip_forward=1"))
    #expect(script.contains("nft -f"))
    #expect(script.contains("/etc/vzctl/vzctl.nft"))
    #expect(script.contains("delete table inet vzctl"))
    #expect(script.contains("cmp -s"))
    #expect(!script.contains("iptables"))
    #expect(!script.contains("ssh"))
    #expect(RouterGuestConfigurator.routerStatusScript.contains("nft list table inet vzctl"))
}
