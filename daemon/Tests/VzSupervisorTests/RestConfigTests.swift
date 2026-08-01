import Foundation
import Testing
@testable import VzSupervisor

struct RestConfigTests {
    @Test func parseUnixListen() throws {
        let spec = try RestListenSpec.parse("unix:/tmp/vz-api.sock")
        #expect(spec == .unix(path: "/tmp/vz-api.sock"))
    }

    @Test func parseTcpLoopback() throws {
        let spec = try RestListenSpec.parse("tcp:127.0.0.1:17800")
        #expect(spec == .tcp(host: "127.0.0.1", port: 17_800))
    }

    @Test func rejectNonLoopbackTcp() {
        #expect(throws: RestConfigError.self) {
            try RestListenSpec.parse("tcp:0.0.0.0:17800")
        }
    }

    @Test func defaultListenUsesStateDir() throws {
        let dir = URL(fileURLWithPath: "/tmp/vzctl-test-state", isDirectory: true)
        let spec = try RestConfig.resolve(stateDirectory: dir, flagValue: nil, environment: [:])
        #expect(spec == .unix(path: "/tmp/vzctl-test-state/api.sock"))
    }

    @Test func flagOverridesEnv() throws {
        let dir = URL(fileURLWithPath: "/tmp/vzctl-test-state", isDirectory: true)
        let spec = try RestConfig.resolve(
            stateDirectory: dir,
            flagValue: "tcp:127.0.0.1:19000",
            environment: ["VZCTL_API_LISTEN": "unix:/tmp/from-env.sock"]
        )
        #expect(spec == .tcp(host: "127.0.0.1", port: 19_000))
    }

    @Test func parseServeArgs() throws {
        let parsed = try RestConfig.parseServeArgs([
            "--api-listen", "unix:/tmp/a.sock", "extra",
        ])
        #expect(parsed.apiListen == "unix:/tmp/a.sock")
        #expect(parsed.remaining == ["extra"])
    }

    @Test func pathSegmentsDecodeSlash() {
        let segments = RestHTTP.pathSegments("/v1/vms/proj%2Fweb")
        #expect(segments == ["v1", "vms", "proj/web"])
    }
}
