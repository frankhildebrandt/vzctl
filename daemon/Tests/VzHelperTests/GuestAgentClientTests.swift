import Darwin
import Foundation
import Testing
@testable import VzHelper

@Test func framingAndHelloUseProtocolV1() throws {
    let pair = try SocketPair()
    let serverDone = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
        defer { serverDone.signal() }
        do {
            let request = try readObject(from: pair.server)
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
                        "capabilities": ["ping", "version", "exec", "report_ip", "health"],
                    ],
                ],
                to: pair.server
            )
        } catch {}
    }

    let client = GuestAgentClient(fileDescriptor: pair.client, ownsFileDescriptor: false)
    let hello = try client.hello(token: String(repeating: "a", count: 43), helperVersion: "test")
    #expect(hello.version == "test")
    #expect(hello.capabilities.contains("exec"))
    #expect(serverDone.wait(timeout: .now() + 1) == .success)
}

@Test func helperDeadlineSendsCancelAndCloses() throws {
    let pair = try SocketPair()
    let observed = LockedValue<String?>(nil)
    let serverDone = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
        defer { serverDone.signal() }
        do {
            let request = try readObject(from: pair.server)
            guard request["method"] as? String == "ping" else { return }
            let cancel = try readObject(from: pair.server)
            observed.set(cancel["method"] as? String)
        } catch {}
    }

    let client = GuestAgentClient(fileDescriptor: pair.client, ownsFileDescriptor: false)
    #expect(throws: GuestAgentError.self) {
        try client.ping(timeout: 0.03)
    }
    #expect(serverDone.wait(timeout: .now() + 1) == .success)
    #expect(observed.get() == "cancel")
}

@Test func execFailureKeepsStructuredExitAndStreams() throws {
    let pair = try SocketPair()
    DispatchQueue.global().async {
        do {
            let request = try readObject(from: pair.server)
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
                to: pair.server
            )
        } catch {}
    }

    let client = GuestAgentClient(fileDescriptor: pair.client, ownsFileDescriptor: false)
    do {
        _ = try client.exec(argv: ["/bin/false"])
        Issue.record("exec should fail")
    } catch let GuestAgentError.remote(code, _, details) {
        #expect(code == "exec_failed")
        #expect(details.values["exit"] as? Int == 7)
        #expect(details.values["stdout"] as? String == "out")
        #expect(details.values["stderr"] as? String == "err")
    }
}

@Test func reportIPRejectsDotZeroFromGuest() throws {
    let pair = try SocketPair()
    DispatchQueue.global().async {
        do {
            let request = try readObject(from: pair.server)
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
                to: pair.server
            )
        } catch {}
    }
    let client = GuestAgentClient(fileDescriptor: pair.client, ownsFileDescriptor: false)
    #expect(throws: GuestAgentError.self) {
        _ = try client.reportIP()
    }
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
