import Darwin
import Foundation
import Testing
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
