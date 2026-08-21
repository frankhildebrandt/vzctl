import Darwin
import Foundation
import Testing
@testable import VzHelper

@Test func framingAndHelloUseProtocolV1() throws {
    try withMockServer { server in
        let request = try readObject(from: server)
        guard
            request["method"] as? String == "hello",
            let id = request["id"] as? String
        else { return }
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": true,
                "result": [
                    "v": 1,
                    "agent_version": "test",
                    "capabilities": [
                        "ping", "version", "exec", "report_ip", "health", "time_hint",
                    ],
                ],
            ],
            to: server
        )
    } client: { client in
        let hello = try client.hello(token: String(repeating: "a", count: 43), helperVersion: "test")
        #expect(hello.version == "test")
        #expect(hello.capabilities.contains("exec"))
    }
}

@Test func timeHintSendsHostTimeAndDecodesCorrection() throws {
    let observedReason = LockedValue<String?>(nil)
    try withMockServer { server in
        let request = try readObject(from: server)
        guard
            let id = request["id"] as? String,
            let params = request["params"] as? [String: Any]
        else { return }
        observedReason.set(params["reason"] as? String)
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": true,
                "result": [
                    "observed_guest_unix_ms": 1_785_387_590_000,
                    "offset_ms": 10_000,
                    "action": "stepped",
                ],
            ],
            to: server
        )
    } client: { client in
        let result = try client.timeHint(
            hostUnixMS: 1_785_387_600_000,
            reason: .wake
        )
        #expect(observedReason.get() == "wake")
        #expect(result.observedGuestUnixMS == 1_785_387_590_000)
        #expect(result.offsetMS == 10_000)
        #expect(result.action == .stepped)
    }
}

@Test func helperDeadlineSendsCancelAndCloses() throws {
    let observed = LockedValue<String?>(nil)
    try withMockServer { server in
        let request = try readObject(from: server)
        guard request["method"] as? String == "ping" else { return }
        let cancel = try readObject(from: server)
        observed.set(cancel["method"] as? String)
    } client: { client in
        #expect(throws: GuestAgentError.self) {
            try client.ping(timeout: 0.05)
        }
    }
    #expect(observed.get() == "cancel")
}

@Test func statsDecodesCpuMemoryAndDisk() throws {
    try withMockServer { server in
        let request = try readObject(from: server)
        guard
            request["method"] as? String == "stats",
            let id = request["id"] as? String
        else { return }
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": true,
                "result": [
                    "cpu": ["percent": 12.5],
                    "memory": [
                        "used_mib": 512,
                        "total_mib": 1024,
                        "percent": 50.0,
                    ],
                    "disk": [
                        "read_iops": 3.2,
                        "write_iops": 1.1,
                    ],
                ],
            ],
            to: server
        )
    } client: { client in
        let stats = try client.stats()
        let cpu = stats["cpu"] as? [String: Any]
        let memory = stats["memory"] as? [String: Any]
        let disk = stats["disk"] as? [String: Any]
        #expect((cpu?["percent"] as? NSNumber)?.doubleValue == 12.5)
        #expect((memory?["used_mib"] as? NSNumber)?.intValue == 512)
        #expect((memory?["total_mib"] as? NSNumber)?.intValue == 1024)
        #expect((memory?["percent"] as? NSNumber)?.doubleValue == 50)
        #expect((disk?["read_iops"] as? NSNumber)?.doubleValue == 3.2)
        #expect((disk?["write_iops"] as? NSNumber)?.doubleValue == 1.1)
    }
}

@Test func execFailureKeepsStructuredExitAndStreams() throws {
    // Wire contract: exec_failed becomes AgentExecResult for callers that expect
    // exit/stdout/stderr rather than a thrown remote error.
    try withMockServer { server in
        let request = try readObject(from: server)
        guard let id = request["id"] as? String else { return }
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": false,
                "error": [
                    "code": "exec_failed",
                    "message": "exit status 7",
                    "details": [
                        "exit": 7,
                        "stdout": "out",
                        "stderr": "err",
                        "truncated": false,
                    ],
                ],
            ],
            to: server
        )
    } client: { client in
        let result = try client.exec(argv: ["/bin/false"])
        #expect(result.exit == 7)
        #expect(result.stdout == "out")
        #expect(result.stderr == "err")
        #expect(result.truncated == false)
    }
}

@Test func reportIPRejectsDotZeroFromGuest() throws {
    try withMockServer { server in
        let request = try readObject(from: server)
        guard let id = request["id"] as? String else { return }
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": true,
                "result": [
                    "interfaces": [[
                        "name": "enp0s1",
                        "mac": "02:00:00:00:00:10",
                        "addresses": ["10.90.1.0/24"],
                    ]],
                ],
            ],
            to: server
        )
    } client: { client in
        #expect(throws: GuestAgentError.self) {
            _ = try client.reportIP()
        }
    }
}

@Test func caInjectUsesGuestAgentWireMethod() throws {
    let observedMethod = LockedValue<String?>(nil)
    let observedName = LockedValue<String?>(nil)
    try withMockServer { server in
        let request = try readObject(from: server)
        guard
            let id = request["id"] as? String,
            let params = request["params"] as? [String: Any]
        else { return }
        observedMethod.set(request["method"] as? String)
        observedName.set(params["name"] as? String)
        try writeObject(
            [
                "v": 1,
                "id": id,
                "ok": true,
                "result": [
                    "installed": true,
                    "fingerprint": String(repeating: "a", count: 64),
                    "name": "vzctl-local",
                    "path": "/usr/local/share/ca-certificates/vzctl-local.crt",
                ],
            ],
            to: server
        )
    } client: { client in
        let result = try client.caInject(
            pem: "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----\n",
            fingerprint: String(repeating: "a", count: 64)
        )
        #expect(result["installed"] as? Bool == true)
    }
    #expect(observedMethod.get() == "ca_inject")
    #expect(observedName.get() == "vzctl-local")
}

@Test func hostTokenRequiresMode0600() throws {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-token-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("agent.token")
    let token = Data(repeating: 1, count: 32).base64EncodedString()
        .replacingOccurrences(of: "+", with: "-")
        .replacingOccurrences(of: "/", with: "_")
        .replacingOccurrences(of: "=", with: "")
    try token.write(to: url, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
    #expect(try AgentToken.load(from: url) == token)
    try FileManager.default.setAttributes([.posixPermissions: 0o640], ofItemAtPath: url.path)
    #expect(throws: HelperError.self) {
        _ = try AgentToken.load(from: url)
    }
}

/// Runs `handler` on a dedicated thread that is parked in `read` before the client starts.
/// Avoids DispatchQueue.global starvation under parallel Swift Testing on CI.
private func withMockServer(
    _ handler: @escaping @Sendable (Int32) throws -> Void,
    client: (GuestAgentClient) throws -> Void
) throws {
    let pair = try SocketPair()
    let started = DispatchSemaphore(value: 0)
    let finished = DispatchSemaphore(value: 0)
    let errorBox = LockedValue<String?>(nil)

    let thread = Thread {
        started.signal()
        do {
            try handler(pair.server)
        } catch {
            errorBox.set(String(describing: error))
        }
        finished.signal()
    }
    thread.qualityOfService = .userInitiated
    thread.start()
    #expect(started.wait(timeout: .now() + 1) == .success)

    // Tiny yield so the thread can enter its blocking read before we write.
    Thread.sleep(forTimeInterval: 0.01)

    let agent = GuestAgentClient(fileDescriptor: pair.client, ownsFileDescriptor: false)
    defer { agent.close() }
    try client(agent)

    #expect(finished.wait(timeout: .now() + 2) == .success)
    if let serverError = errorBox.get() {
        Issue.record("mock server error: \(serverError)")
    }
}

private final class SocketPair: @unchecked Sendable {
    let client: Int32
    let server: Int32

    init() throws {
        var descriptors: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &descriptors) == 0 else {
            throw POSIXError(.ENOTSOCK)
        }
        client = descriptors[0]
        server = descriptors[1]
    }

    deinit {
        Darwin.close(client)
        Darwin.close(server)
    }
}

private final class LockedValue<Value>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Value

    init(_ value: Value) {
        self.value = value
    }

    func set(_ value: Value) {
        lock.lock()
        self.value = value
        lock.unlock()
    }

    func get() -> Value {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

private func readObject(from descriptor: Int32) throws -> [String: Any] {
    let prefix = try readExactly(4, from: descriptor)
    let length = prefix.withUnsafeBytes {
        UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))
    }
    let payload = try readExactly(Int(length), from: descriptor)
    guard let object = try JSONSerialization.jsonObject(with: payload) as? [String: Any] else {
        throw POSIXError(.EBADMSG)
    }
    return object
}

private func writeObject(_ object: [String: Any], to descriptor: Int32) throws {
    let payload = try JSONSerialization.data(withJSONObject: object)
    var length = UInt32(payload.count).littleEndian
    let prefix = withUnsafeBytes(of: &length) { Data($0) }
    try writeAll(prefix + payload, to: descriptor)
}

private func readExactly(_ count: Int, from descriptor: Int32) throws -> Data {
    var data = Data(count: count)
    var offset = 0
    try data.withUnsafeMutableBytes { buffer in
        while offset < count {
            let received = Darwin.read(
                descriptor,
                buffer.baseAddress!.advanced(by: offset),
                count - offset
            )
            guard received > 0 else { throw POSIXError(.ECONNRESET) }
            offset += received
        }
    }
    return data
}

private func writeAll(_ data: Data, to descriptor: Int32) throws {
    try data.withUnsafeBytes { buffer in
        var offset = 0
        while offset < buffer.count {
            let written = Darwin.write(
                descriptor,
                buffer.baseAddress!.advanced(by: offset),
                buffer.count - offset
            )
            guard written > 0 else { throw POSIXError(.EPIPE) }
            offset += written
        }
    }
}
