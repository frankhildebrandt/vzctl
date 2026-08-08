import Testing
@testable import VzDaemonKit

@Test func stateFileComponentIsStableAndPathSafe() {
    #expect(StateFileName.component("demo/web") == "demo_web-d314c8ccfd1a9783")
    #expect(!StateFileName.component("../web").contains("/"))
}

@Test func socketComponentIsStableAndKeepsDefaultHelperPathsShort() {
    let component = StateFileName.socketComponent("monitos/monitos-main")
    #expect(component == "44da7bb0f51beebe")
    #expect(!component.contains("/"))

    let path = "/Users/example/Library/Application Support/vzctl/helpers/"
        + component + ".console.sock"
    #expect(path.utf8.count < 104)
}
