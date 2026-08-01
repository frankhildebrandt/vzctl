import Darwin
import Foundation
import Testing
import VzDaemonKit

@testable import VzSupervisor

@Test func dnsServerBindsHighGuestPortWithoutHelper() throws {
    let configuration = DNSConfiguration(
        hostAddress: "127.0.0.1",
        hostPort: 18_353,
        guestPort: 18_354,
        ttl: 15,
        upstream: "system"
    )
    let server = DNSServer(configuration: configuration)
    defer { server.shutdown() }

    let health = server.reload(
        snapshot: NetworkSnapshot(
            networks: [
                NetworkRecord(
                    name: "dmz",
                    cidr: "10.80.0.0/24",
                    project: "edge-dmz",
                    runtimeState: "active"
                ),
            ],
            attachments: []
        )
    )
    // Guest .0:18354 may fail with EADDRNOTAVAIL if bridge absent; host loopback must work.
    #expect(health.listeners.contains("127.0.0.1:18353"))
    #expect(health.upstream == "system")
}

@Test func hostServicesUseSplitHorizonAddresses() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "edge-dmz"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "web",
                networkName: "dmz",
                ip: "10.80.0.10",
                project: "edge-dmz"
            ),
        ]
    )

    let zone = DNSZoneBuilder.build(
        snapshot: snapshot,
        ttl: 15,
        hostServices: ["auth.svc.edge-dmz.vz.test", "web.svc.edge-dmz.vz.test"]
    )

    #expect(zone.addresses(for: "auth.svc.edge-dmz.vz.test", horizon: .host) == ["127.0.0.1"])
    #expect(zone.addresses(for: "auth.svc.edge-dmz.vz.test", horizon: .guest) == ["10.80.0.0"])
    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test", horizon: .guest) == ["10.80.0.10"])
}

@Test func zoneBuilderCreatesVMARecordFromActualAttachments() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "edge-dmz"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "web",
                networkName: "dmz",
                ip: "10.80.0.10"
            ),
        ]
    )

    let zone = DNSZoneBuilder.build(snapshot: snapshot, ttl: 15)

    #expect(zone.zones == ["edge-dmz.vz.test"])
    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test.", horizon: .guest) == ["10.80.0.10"])
    #expect(zone.ttl == 15)
}

@Test func zoneBuilderUsesBasenameForNamespacedVmIDs() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "edge-dmz"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "edge-dmz/web",
                networkName: "dmz",
                ip: "10.80.0.10",
                project: "edge-dmz"
            ),
        ]
    )

    let zone = DNSZoneBuilder.build(snapshot: snapshot, ttl: 15)

    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test", horizon: .guest) == ["10.80.0.10"])
    #expect(zone.addresses(for: "edge-dmz/web.dmz.edge-dmz.vz.test", horizon: .guest) == nil)
    #expect(DNSZoneBuilder.vmDNSLabel("edge-dmz/web") == "web")
    #expect(DNSZoneBuilder.vmDNSLabel("web") == "web")
}

@Test func attachmentProjectOverridesNetworkAndServicesReturnAllBackends() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "lan",
                cidr: "10.90.0.0/24",
                labels: [DNSZoneBuilder.serviceLabel: "metrics"],
                project: "fallback"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "api-1",
                networkName: "lan",
                ip: "10.90.0.10",
                labels: [DNSZoneBuilder.serviceLabel: "api, metrics"],
                project: "shop"
            ),
            NetworkAttachmentRecord(
                vmID: "api-2",
                networkName: "lan",
                ip: "10.90.0.11",
                labels: [DNSZoneBuilder.serviceLabel: "api"],
                project: "shop"
            ),
        ]
    )

    let zone = DNSZoneBuilder.build(snapshot: snapshot, ttl: 5)

    #expect(zone.addresses(for: "api-1.lan.shop.vz.test", horizon: .guest) == ["10.90.0.10"])
    #expect(zone.addresses(for: "api.svc.shop.vz.test", horizon: .guest) == [
        "10.90.0.10",
        "10.90.0.11",
    ])
    #expect(zone.addresses(for: "metrics.svc.shop.vz.test", horizon: .guest) == ["10.90.0.10"])
    #expect(zone.addresses(for: "api-1.lan.fallback.vz.test", horizon: .guest) == nil)
}

@Test func zoneBuilderSkipsOrphanedNetworksAndInvalidDNSLabels() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "edge",
                runtimeState: "orphaned"
            ),
            NetworkRecord(
                name: "bad_name",
                cidr: "10.81.0.0/24",
                project: "edge"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(vmID: "web", networkName: "dmz", ip: "10.80.0.10"),
            NetworkAttachmentRecord(
                vmID: "web",
                networkName: "bad_name",
                ip: "10.81.0.10"
            ),
        ]
    )

    let zone = DNSZoneBuilder.build(snapshot: snapshot, ttl: 1)

    #expect(zone.records.isEmpty)
    #expect(zone.ttl == 5)
}

@Test func zoneBuilderClampsTTLToAlphaRange() {
    let empty = NetworkSnapshot(networks: [], attachments: [])

    #expect(DNSZoneBuilder.build(snapshot: empty, ttl: 4).ttl == 5)
    #expect(DNSZoneBuilder.build(snapshot: empty, ttl: 31).ttl == 30)
}

@Test func authoritativeWireResponseContainsARecordAndTTL() {
    let server = DNSServer(configuration: DNSConfiguration(
        hostAddress: "127.0.0.1",
        hostPort: 0,
        guestPort: 0,
        ttl: 12,
        upstream: "127.0.0.1:9"
    ))
    defer { server.shutdown() }
    _ = server.reload(snapshot: NetworkSnapshot(
        networks: [
            NetworkRecord(name: "dmz", cidr: "10.80.0.0/24", project: "edge"),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "web",
                networkName: "dmz",
                ip: "10.80.0.10"
            ),
        ]
    ))
    let query = dnsQuery("web.dmz.edge.vz.test")

    let response = server.response(for: query)
    let answerOffset = query.count + 6

    #expect(read16(response, 2) & 0x8400 == 0x8400)
    #expect(read16(response, 6) == 1)
    #expect(read32(response, answerOffset) == 12)
    #expect(Array(response.suffix(4)) == [10, 80, 0, 10])
}

@Test func externalQueriesAreForwardedUnchangedToConfiguredUpstream() throws {
    let upstream = try UDPFixture()
    defer { upstream.close() }
    upstream.replyOnce()
    let server = DNSServer(configuration: DNSConfiguration(
        hostAddress: "127.0.0.1",
        hostPort: 0,
        guestPort: 0,
        ttl: 15,
        upstream: "127.0.0.1:\(upstream.port)"
    ))
    defer { server.shutdown() }
    let query = dnsQuery("example.com")

    let response = server.response(for: query)

    #expect(read16(response, 0) == 0x1234)
    #expect(read16(response, 2) & 0x8080 == 0x8080)
}

@Test(.enabled(if: ProcessInfo.processInfo.environment["VZCTL_DNS_LAB"] == "1"))
func systemUpstreamLabResolvesExternalName() {
    let server = DNSServer(configuration: DNSConfiguration(
        hostAddress: "127.0.0.1",
        hostPort: 15_353,
        guestPort: 15_353,
        ttl: 15,
        upstream: "system"
    ))

    let response = server.response(for: dnsQuery("example.com"))

    #expect(read16(response, 2) & 0x800F == 0x8000)
    #expect(read16(response, 6) > 0)
}

private final class UDPFixture: @unchecked Sendable {
    let descriptor: Int32
    let port: UInt16

    init() throws {
        let socketDescriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard socketDescriptor >= 0 else { throw FixtureError.socket }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = 0
        address.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(socketDescriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bound == 0 else {
            Darwin.close(socketDescriptor)
            throw FixtureError.bind
        }
        var actual = sockaddr_in()
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let read = withUnsafeMutablePointer(to: &actual) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                getsockname(socketDescriptor, $0, &length)
            }
        }
        guard read == 0 else {
            Darwin.close(socketDescriptor)
            throw FixtureError.bind
        }
        descriptor = socketDescriptor
        port = UInt16(bigEndian: actual.sin_port)
    }

    func replyOnce() {
        DispatchQueue.global().async { [descriptor] in
            var buffer = [UInt8](repeating: 0, count: 512)
            var peer = sockaddr_storage()
            var length = socklen_t(MemoryLayout<sockaddr_storage>.size)
            let count = withUnsafeMutablePointer(to: &peer) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.recvfrom(descriptor, &buffer, buffer.count, 0, $0, &length)
                }
            }
            guard count >= 12 else { return }
            buffer[2] = 0x81
            buffer[3] = 0x80
            withUnsafePointer(to: &peer) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    _ = Darwin.sendto(descriptor, &buffer, count, 0, $0, length)
                }
            }
        }
    }

    func close() {
        Darwin.close(descriptor)
    }
}

private enum FixtureError: Error {
    case socket
    case bind
}

private func dnsQuery(_ name: String) -> Data {
    var data = Data([0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
    for label in name.split(separator: ".") {
        data.append(UInt8(label.utf8.count))
        data.append(contentsOf: label.utf8)
    }
    data.append(0)
    data.append(contentsOf: [0, 1, 0, 1])
    return data
}

private func read16(_ data: Data, _ offset: Int) -> UInt16 {
    UInt16(data[offset]) << 8 | UInt16(data[offset + 1])
}

private func read32(_ data: Data, _ offset: Int) -> UInt32 {
    UInt32(data[offset]) << 24
        | UInt32(data[offset + 1]) << 16
        | UInt32(data[offset + 2]) << 8
        | UInt32(data[offset + 3])
}
