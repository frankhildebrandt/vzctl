import Darwin
import Foundation
import Testing
import VzDaemonKit
@testable import VzEdge

@Test func loopbackReachableDetectsOpenListener() throws {
    let (fd, port) = try bindEphemeralListener()
    defer { Darwin.close(fd) }
    #expect(
        HostGatewayIngressProxy.loopbackReachable(
            gateways: ["127.0.0.1"],
            httpPort: port,
            httpsPort: port
        )
    )
}

@Test func loopbackReachableFailsWhenNothingListens() {
    #expect(
        !HostGatewayIngressProxy.loopbackReachable(
            gateways: ["127.0.0.1"],
            httpPort: 1,
            httpsPort: 2
        )
    )
}

@Test func embeddedProcessReaperCountsNoOrphansInEmptyWorkDir() {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-reaper-\(UUID().uuidString)", isDirectory: true)
    defer { try? FileManager.default.removeItem(at: root) }
    try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    let count = EmbeddedProcessReaper.countOrphans(
        binary: "/usr/bin/caddy",
        workDir: root.path,
        exceptPID: nil
    )
    #expect(count == 0)
}

@Test func gatewayPreservesUnchangedStreamAndReconnectsAfterHelperDisconnect() throws {
    let (backendListener, backendPort) = try bindEphemeralListener()
    defer { Darwin.close(backendListener) }
    setTestSocketTimeout(backendListener)
    let helper = RestartableAcceptHelper()
    let proxy = HostGatewayIngressProxy(acceptStreamFactory: { _ in try helper.open() })
    defer {
        proxy.shutdown()
        helper.disconnect()
    }
    let binding = HostGatewayIngressProxy.Binding(
        gatewayIP: "127.0.0.1", port: 80,
        backendHost: "127.0.0.1", backendPort: backendPort
    )
    // The test factory uses a socketpair: no privileged port or live helper is touched.
    #expect(proxy.ensure([binding]).skipped.isEmpty)
    try assertForwarding(helper: helper, backendListener: backendListener)
    #expect(proxy.ensure([binding]).skipped.isEmpty)
    #expect(helper.connectionCount == 1)

    helper.disconnect()
    let deadline = Date().addingTimeInterval(3)
    while helper.connectionCount < 2 && Date() < deadline {
        Thread.sleep(forTimeInterval: 0.02)
    }
    #expect(helper.connectionCount == 2)
    try assertForwarding(helper: helper, backendListener: backendListener)
    #expect(proxy.list() == [binding])
}

private final class RestartableAcceptHelper: @unchecked Sendable {
    private let lock = NSLock()
    private var socket: Int32 = -1
    private var count = 0
    var connectionCount: Int { lock.withLock { count } }

    func open() throws -> DnsBindClient.TCPAcceptStream {
        let pair = try testSocketPair()
        return lock.withLock {
            socket = pair.1
            count += 1
            return DnsBindClient.TCPAcceptStream(socket: pair.0)
        }
    }

    func disconnect() {
        lock.withLock {
            guard socket >= 0 else { return }
            Darwin.shutdown(socket, SHUT_RDWR)
            Darwin.close(socket)
            socket = -1
        }
    }

    func client() throws -> Int32 {
        let pair = try testSocketPair()
        defer { Darwin.close(pair.1) }
        do {
            try lock.withLock {
                let response = try JSONEncoder().encode(DnsBind.BindResponse(ok: true, event: "accept"))
                try UnixFDPassing.send(payload: response, fileDescriptor: pair.1, on: socket)
            }
            return pair.0
        } catch {
            Darwin.close(pair.0)
            throw error
        }
    }
}

private func assertForwarding(helper: RestartableAcceptHelper, backendListener: Int32) throws {
    let client = try helper.client()
    defer {
        Darwin.shutdown(client, SHUT_RDWR)
        Darwin.close(client)
    }
    let backend = Darwin.accept(backendListener, nil, nil)
    try #require(backend >= 0)
    defer {
        Darwin.shutdown(backend, SHUT_RDWR)
        Darwin.close(backend)
    }
    setTestSocketTimeout(backend)
    let payload: [UInt8] = [112, 105, 110, 103]
    #expect(payload.withUnsafeBytes { Darwin.write(client, $0.baseAddress, $0.count) } == payload.count)
    var received = [UInt8](repeating: 0, count: payload.count)
    #expect(Darwin.read(backend, &received, received.count) == payload.count)
    #expect(received == payload)
}

private func testSocketPair() throws -> (Int32, Int32) {
    var descriptors: [Int32] = [-1, -1]
    guard socketpair(AF_UNIX, SOCK_STREAM, 0, &descriptors) == 0 else {
        throw POSIXError(.init(rawValue: errno)!)
    }
    for descriptor in descriptors { setTestSocketTimeout(descriptor) }
    return (descriptors[0], descriptors[1])
}

private func setTestSocketTimeout(_ descriptor: Int32) {
    var timeout = timeval(tv_sec: 2, tv_usec: 0)
    setsockopt(descriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
    setsockopt(descriptor, SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
    var yes: Int32 = 1
    setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size))
}

private func bindEphemeralListener() throws -> (Int32, UInt16) {
    let sock = Darwin.socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
    guard sock >= 0 else { throw POSIXError(.init(rawValue: errno)!) }
    var yes: Int32 = 1
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
    var address = sockaddr_in()
    address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    address.sin_family = sa_family_t(AF_INET)
    address.sin_port = 0
    address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
    let bindResult = withUnsafePointer(to: &address) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard bindResult == 0 else {
        Darwin.close(sock)
        throw POSIXError(.init(rawValue: errno)!)
    }
    guard Darwin.listen(sock, 8) == 0 else {
        Darwin.close(sock)
        throw POSIXError(.init(rawValue: errno)!)
    }
    var bound = sockaddr_in()
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    guard withUnsafeMutablePointer(to: &bound, {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            getsockname(sock, $0, &length) == 0
        }
    }) else {
        Darwin.close(sock)
        throw POSIXError(.init(rawValue: errno)!)
    }
    return (sock, UInt16(bigEndian: bound.sin_port))
}
