import Testing
@testable import VzHelper

@Test func routerApplyUsesPersistentSysctlAndDefaultDrop() {
    let script = RouterGuestConfigurator.routerApplyScript

    #expect(script.contains("/etc/sysctl.d/90-vzctl-router.conf"))
    #expect(script.contains("sysctl -q -w net.ipv4.ip_forward=1"))
    #expect(script.contains("iptables -P FORWARD DROP"))
    #expect(script.contains("policy drop"))
    #expect(script.contains("cmp -s"))
    #expect(!script.contains("ssh"))
}
