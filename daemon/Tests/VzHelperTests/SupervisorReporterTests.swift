import Darwin
import Foundation
import Testing
import VzDaemonKit
@testable import VzHelper

@Test func failedStateIncludesConcreteError() throws {
    let directory = URL(
        fileURLWithPath: "/tmp/vzrp-\(String(UUID().uuidString.prefix(8)))",
        isDirectory: true
    )
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }

    let socket = try ReporterCaptureSocket(path: directory.appendingPathComponent("vz.sock").path)
    defer { socket.stop() }
    let reporter = SupervisorReporter(
        vmID: "monitos/monitos-main",
        bundle: "/tmp/bundle",
        socketPath: socket.path
    )

    reporter.report(.failed, error: "console socket path is too long")

    let request = try #require(socket.request)
    guard case let .object(params)? = request.params else {
        Issue.record("expected object params")
        return
    }
    #expect(params["state"] == .string("failed"))
    #expect(params["error"] == .string("console socket path is too long"))
}

@Test func runArgumentFailureIsReportedBeforeRunStarts() throws {
    let directory = URL(
        fileURLWithPath: "/tmp/vzrp-\(String(UUID().uuidString.prefix(8)))",
        isDirectory: true
    )
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let socket = try ReporterCaptureSocket(path: directory.appendingPathComponent("vz.sock").path)
    defer { socket.stop() }

    VzHelperMain.reportRunFailure(
        arguments: [
            "run", "--vm-id", "monitos/monitos-main", "--bundle", "/tmp/bundle",
            "--supervisor-sock", socket.path,
        ],
        error: HelperError.invalid("invalid VM manifest resources.memory_mib")
    )

    let request = try #require(socket.request)
    guard case let .object(params)? = request.params else {
        Issue.record("expected object params")
        return
    }
    #expect(params["state"] == .string("failed"))
    #expect(params["error"] == .string("invalid VM manifest resources.memory_mib"))
}

private final class ReporterCaptureSocket: @unchecked Sendable {
    let path: String
    private let listener: Int32
    private let lock = NSLock()
    private var captured: JSONRPCRequest?
    private var task: Task<Void, Never>?

    var request: JSONRPCRequest? { lock.withLock { captured } }

    init(path: String) throws {
        self.path = path
        unlink(path)
        listener = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard listener >= 0 else { throw HelperError.system("socket", errno) }

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0, Darwin.listen(listener, 1) == 0 else {
            throw HelperError.system("listen", errno)
        }

        task = Task.detached { [weak self] in
            guard let self else { return }
            let client = Darwin.accept(self.listener, nil, nil)
            guard client >= 0 else { return }
            defer { Darwin.close(client) }
            var data = Data()
            var byte: UInt8 = 0
            while Darwin.read(client, &byte, 1) == 1 {
                data.append(byte)
                if byte == 0x0A { break }
            }
            if let request = try? JSONRPCFraming.decode(JSONRPCRequest.self, from: data) {
                self.lock.withLock { self.captured = request }
                let response = JSONRPCResponse(result: .object(["ok": .bool(true)]), id: request.id)
                if let encoded = try? JSONRPCFraming.encode(response) {
                    _ = encoded.withUnsafeBytes { raw in
                        Darwin.write(client, raw.baseAddress, raw.count)
                    }
                }
            }
        }
    }

    func stop() {
        task?.cancel()
        Darwin.close(listener)
        unlink(path)
    }
}
