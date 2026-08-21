import Foundation
import Testing
import VzDaemonKit
@testable import VzHelper

@Test func agentExecParsesRequiredAndOptionalParams() throws {
    let params = JSONValue.object([
        "vm_id": .string("demo/web"),
        "cmd": .array([.string("/bin/echo"), .string("hi")]),
        "cwd": .string("/tmp"),
        "env": .object(["FOO": .string("bar")]),
        "timeout_ms": .number(1_500),
        "stdin_b64": .string(Data("hello".utf8).base64EncodedString()),
    ])
    let parsed = try HelperAgentRequest.parseExec(params)
    #expect(parsed.cmd == ["/bin/echo", "hi"])
    #expect(parsed.cwd == "/tmp")
    #expect(parsed.env == ["FOO": "bar"])
    #expect(parsed.timeoutMilliseconds == 1_500)
    #expect(parsed.stdin == Data("hello".utf8))
}

@Test func agentExecDefaultsOptionalFields() throws {
    let parsed = try HelperAgentRequest.parseExec(
        .object([
            "cmd": .array([.string("true")]),
        ])
    )
    #expect(parsed.cmd == ["true"])
    #expect(parsed.cwd == nil)
    #expect(parsed.env.isEmpty)
    #expect(parsed.timeoutMilliseconds == 30_000)
    #expect(parsed.stdin == nil)
}

@Test func agentExecRejectsInvalidShapes() {
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseExec(nil)
    }
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseExec(.object(["cmd": .array([])]))
    }
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseExec(
            .object(["cmd": .array([.number(1)])])
        )
    }
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseExec(
            .object([
                "cmd": .array([.string("true")]),
                "stdin_b64": .string("@@@"),
            ])
        )
    }
}

@Test func agentProxyExposesKnownMethods() {
    #expect(HelperAgentProxy.methods.contains("agent.exec"))
    #expect(HelperAgentProxy.methods.contains("agent.exec_tty"))
    #expect(HelperAgentProxy.methods.contains("agent.health"))
    #expect(HelperAgentProxy.methods.contains("agent.version"))
    #expect(HelperAgentProxy.methods.contains("agent.report_ip"))
    #expect(HelperAgentProxy.methods.contains("agent.ping"))
    #expect(HelperAgentProxy.methods.contains("agent.ca_inject"))
    #expect(HelperAgentProxy.methods.contains("agent.stats"))
}

@Test func agentCAInjectParsesRequiredParamsAndDefaultName() throws {
    let parsed = try HelperAgentRequest.parseCAInject(
        .object([
            "pem": .string("-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n"),
            "fingerprint": .string(String(repeating: "a", count: 64)),
        ])
    )
    #expect(parsed.name == "vzctl-local")
    #expect(parsed.fingerprint == String(repeating: "a", count: 64))
}

@Test func agentCAInjectRejectsMissingPEM() {
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseCAInject(
            .object(["fingerprint": .string(String(repeating: "a", count: 64))])
        )
    }
}

@Test func agentExecTTYParsesColsRows() throws {
    let parsed = try HelperAgentRequest.parseExecTTY(
        .object([
            "cmd": .array([.string("/bin/bash")]),
            "cols": .number(120),
            "rows": .number(40),
        ])
    )
    #expect(parsed.cmd == ["/bin/bash"])
    #expect(parsed.cols == 120)
    #expect(parsed.rows == 40)
}

@Test func agentExecTTYRejectsStdin() {
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseExecTTY(
            .object([
                "cmd": .array([.string("bash")]),
                "stdin_b64": .string(Data("x".utf8).base64EncodedString()),
            ])
        )
    }
}

@Test func agentNetworkProbeParsesTargetMode() throws {
    let parsed = try HelperAgentRequest.parseNetworkProbe(
        .object([
            "vm_id": .string("neti/neti-home"),
            "target": .string("main-node.core.neti.vz.test:4222"),
            "via": .string("both"),
            "connect_ip": .string("10.90.0.2"),
            "timeout_ms": .number(1_500),
        ])
    )
    #expect(parsed["target"] as? String == "main-node.core.neti.vz.test:4222")
    #expect(parsed["via"] as? String == "both")
    #expect(parsed["connect_ip"] as? String == "10.90.0.2")
    #expect(parsed["timeout_ms"] as? Int == 1_500)
}

@Test func agentNetworkProbeRejectsUrlAndTargetTogether() {
    #expect(throws: RouteApplyError.self) {
        try HelperAgentRequest.parseNetworkProbe(
            .object([
                "url": .string("https://captive.apple.com/"),
                "target": .string("10.90.0.2:4222"),
            ])
        )
    }
}
@Test func muxEncodeDecodeRoundTrip() throws {
    let payload = Data("pty-out".utf8)
    let encoded = try GuestAgentMux.encode(type: .stdout, payload: payload)
    let (type, decoded) = try GuestAgentMux.decode(encoded)
    #expect(type == .stdout)
    #expect(decoded == payload)
    let resize = GuestAgentMux.resizePayload(cols: 80, rows: 24)
    #expect(resize.count == 4)
    #expect(try GuestAgentMux.exitStatus(from: Data([1, 0, 0, 0])) == 1)
}
