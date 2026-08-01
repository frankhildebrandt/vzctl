import Foundation
import Testing
import VzDaemonKit
@testable import VzHelper

@Test func fetchAttachmentsMissingSupervisorReturnsEmpty() throws {
    // Keep paths short: macOS AF_UNIX sun_path is ~104 bytes.
    let directory = URL(fileURLWithPath: "/tmp/vzvm-\(String(UUID().uuidString.prefix(8)))", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }

    let nics = try HelperVmnetClient.fetchAttachments(
        vmID: "vzb-test",
        socketPath: directory.appendingPathComponent("missing.sock").path,
        bundleURL: directory
    )
    #expect(nics.isEmpty)
}

@Test func fetchAttachmentsReachableSupervisorRPCErrorThrows() throws {
    let directory = URL(fileURLWithPath: "/tmp/vzvm-\(String(UUID().uuidString.prefix(8)))", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }

    let socketPath = directory.appendingPathComponent("vz.sock").path
    let listener = try MockSupervisorSocket(
        path: socketPath,
        response: JSONRPCResponse(
            error: JSONRPCError(code: -32000, message: "networks unavailable"),
            id: .number(1)
        )
    )
    defer { listener.stop() }

    #expect(throws: HelperError.self) {
        _ = try HelperVmnetClient.fetchAttachments(
            vmID: "web",
            socketPath: socketPath,
            bundleURL: directory
        )
    }
}

/// One-shot Unix JSON-RPC responder for helper.networks tests.
private final class MockSupervisorSocket: @unchecked Sendable {
    private let listenFD: Int32
    private let path: String
    private var acceptTask: Task<Void, Never>?

    init(path: String, response: JSONRPCResponse) throws {
        self.path = path
        unlink(path)

        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("socket", errno) }
        self.listenFD = fd

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(fd)
            throw HelperError.invalid("supervisor socket path is too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            Darwin.close(fd)
            throw HelperError.system("bind \(path)", errno)
        }
        guard Darwin.listen(fd, 1) == 0 else {
            Darwin.close(fd)
            throw HelperError.system("listen", errno)
        }

        let encoded = try JSONRPCFraming.encode(response)
        acceptTask = Task.detached {
            let client = Darwin.accept(fd, nil, nil)
            guard client >= 0 else { return }
            defer { Darwin.close(client) }
            // Drain the request line so the peer can finish writing.
            var byte: UInt8 = 0
            while Darwin.read(client, &byte, 1) == 1 {
                if byte == 0x0A { break }
            }
            _ = encoded.withUnsafeBytes { raw -> Bool in
                guard let base = raw.baseAddress else { return true }
                var offset = 0
                while offset < raw.count {
                    let count = Darwin.write(client, base.advanced(by: offset), raw.count - offset)
                    if count <= 0 { return false }
                    offset += count
                }
                return true
            }
        }
    }

    func stop() {
        acceptTask?.cancel()
        Darwin.close(listenFD)
        unlink(path)
    }
}
