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

    @Test func parseRequestKeepsEncodedSlashInPath() {
        let raw = Data("GET /v1/vms/edge-dmz%2Fdocker/mounts HTTP/1.1\r\nHost: x\r\n\r\n".utf8)
        let request = RestHTTP.parseRequest(from: raw)
        #expect(request?.path == "/v1/vms/edge-dmz%2Fdocker/mounts")
        let segments = RestHTTP.pathSegments(request!.path)
        #expect(segments == ["v1", "vms", "edge-dmz/docker", "mounts"])
    }

    @Test func parseRequestDecodesQueryOnly() {
        let raw = Data(
            "GET /v1/vms/proj%2Fweb?force=true&path=%2Ftmp%2Fstack HTTP/1.1\r\nHost: x\r\n\r\n"
                .utf8
        )
        let request = RestHTTP.parseRequest(from: raw)
        #expect(request?.path == "/v1/vms/proj%2Fweb")
        #expect(request?.query["force"] == "true")
        #expect(request?.query["path"] == "/tmp/stack")
        #expect(RestHTTP.pathSegments(request!.path) == ["v1", "vms", "proj/web"])
    }
}
