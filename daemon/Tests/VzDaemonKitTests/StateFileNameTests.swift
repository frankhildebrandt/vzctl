import Testing
@testable import VzDaemonKit

@Test func stateFileComponentIsStableAndPathSafe() {
    #expect(StateFileName.component("demo/web") == "demo_web-d314c8ccfd1a9783")
    #expect(!StateFileName.component("../web").contains("/"))
}
