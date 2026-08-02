import Testing
@testable import VzSupervisor

@Test func supervisorRoutesCAInjectToHelpers() {
    #expect(SupervisorServer.proxiedAgentMethods.contains("vm.agent.ca_inject"))
}
