import Darwin
import Foundation
import Testing
import VzDaemonKit

@testable import VzEdge

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

@Test func dnsConfigurationRedirectsOnlyPrivilegedGuestPortByDefault() {
    let production = DNSConfiguration.environment(["VZCTL_DNS_GUEST_PORT": "53"])
    let development = DNSConfiguration.environment(["VZCTL_DNS_GUEST_PORT": "15353"])
    let overridden = DNSConfiguration.environment([
        "VZCTL_DNS_GUEST_PORT": "53",
        "VZCTL_DNS_GUEST_BACKEND_PORT": "16053",
    ])

    #expect(production.guestBackendPort == DnsBind.defaultGuestDNSBackendPort)
    #expect(development.guestBackendPort == 15_353)
    #expect(overridden.guestBackendPort == 16_053)
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
    #expect(zone.addresses(for: "auth.svc.edge-dmz.vz.test", horizon: guestHorizon()) == ["10.80.0.1"])
    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test", horizon: guestHorizon()) == ["10.80.0.10"])
}

@Test func hostServiceWireResponseHonorsHorizonParameter() {
    let server = DNSServer(configuration: DNSConfiguration(
        hostAddress: "127.0.0.1",
        hostPort: 0,
        guestPort: 0,
        ttl: 15,
        upstream: "127.0.0.1:9"
    ))
    defer { server.shutdown() }
    server.setHostServices(["web.svc.edge-dmz.vz.test"])
    _ = server.reload(snapshot: NetworkSnapshot(
        networks: [
            NetworkRecord(name: "dmz", cidr: "10.80.0.0/24", project: "edge-dmz"),
            NetworkRecord(name: "lan", cidr: "10.90.0.0/24", natEgress: false, project: "edge-dmz"),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "web",
                networkName: "dmz",
                ip: "10.80.0.10",
                project: "edge-dmz"
            ),
        ]
    ))
    let query = dnsQuery("web.svc.edge-dmz.vz.test")

    let hostIps = aRecords(in: server.response(for: query, horizon: .host))
    let dmzIps = aRecords(in: server.response(for: query, horizon: guestHorizon()))
    let lanIps = aRecords(in: server.response(
        for: query,
        horizon: guestHorizon(network: "lan", gateway: "10.90.0.0")
    ))
    let foreign = server.response(
        for: query,
        horizon: guestHorizon(project: "another-project")
    )

    #expect(hostIps == ["127.0.0.1"])
    #expect(dmzIps == ["10.80.0.1"])
    #expect(lanIps == ["10.90.0.1"])
    #expect(dnsResponseCode(foreign) == 3)
}

@Test func hostServicesSkipDockerBackendGateways() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "edge-dmz"
            ),
            NetworkRecord(
                name: "lan",
                cidr: "10.90.0.0/24",
                natEgress: false,
                project: "edge-dmz"
            ),
            NetworkRecord(
                name: "containers",
                cidr: "10.95.0.0/24",
                natEgress: false,
                backend: NetworkRecord.backendDocker,
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
        hostServices: ["web.svc.edge-dmz.vz.test"]
    )

    #expect(zone.addresses(for: "web.svc.edge-dmz.vz.test", horizon: .host) == [
        "127.0.0.1",
    ])
    #expect(zone.addresses(for: "web.svc.edge-dmz.vz.test", horizon: guestHorizon()) == ["10.80.0.1"])
    #expect(zone.addresses(
        for: "web.svc.edge-dmz.vz.test",
        horizon: guestHorizon(network: "lan", gateway: "10.90.0.0")
    ) == ["10.90.0.1"])
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
    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test.", horizon: guestHorizon()) == ["10.80.0.10"])
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

    #expect(zone.addresses(for: "web.dmz.edge-dmz.vz.test", horizon: guestHorizon()) == ["10.80.0.10"])
    #expect(zone.addresses(for: "edge-dmz/web.dmz.edge-dmz.vz.test", horizon: guestHorizon()) == nil)
    #expect(DNSZoneBuilder.vmDNSLabel("edge-dmz/web") == "web")
    #expect(DNSZoneBuilder.vmDNSLabel("web") == "web")
}

@Test func vmAndContainerDNSProvidesFQDNWildcardShortAndPTR() {
    let snapshot = NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz",
                cidr: "10.80.0.0/24",
                project: "shop",
                stack: "platform:shop"
            ),
            NetworkRecord(
                name: "lan",
                cidr: "10.90.0.0/24",
                project: "shop",
                stack: "platform:shop"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "shop/web",
                networkName: "dmz",
                ip: "10.80.0.10",
                project: "shop",
                stack: "platform:shop"
            ),
        ]
    )
    let zone = DNSZoneBuilder.build(
        snapshot: snapshot,
        ttl: 15,
        runtimeRecords: [
            DNSRuntimeRecord(
                name: "api",
                network: "containers",
                listenerNetwork: "dmz",
                stack: "shop",
                project: "shop",
                ip: "10.95.0.10"
            ),
        ]
    )
    let dmz = DNSHorizon.guest(DNSGuestContext(
        network: "dmz", project: "shop", stack: "shop",
        gateway: "10.80.0.0", hostService: "10.80.0.1"
    ))
    let lan = DNSHorizon.guest(DNSGuestContext(
        network: "lan", project: "shop", stack: "shop",
        gateway: "10.90.0.0", hostService: "10.90.0.1"
    ))

    #expect(zone.addresses(for: "web.dmz.shop.vz.test", horizon: .host) == ["10.80.0.10"])
    #expect(zone.addresses(for: "metrics.web.dmz.shop.vz.test", horizon: .host) == ["10.80.0.10"])
    #expect(zone.addresses(for: "web", horizon: dmz) == ["10.80.0.10"])
    #expect(zone.addresses(for: "web", horizon: .host) == nil)
    #expect(zone.addresses(for: "web", horizon: lan) == nil)
    #expect(zone.addresses(for: "api.containers.shop.vz.test", horizon: .host) == ["10.95.0.10"])
    #expect(zone.addresses(for: "x.api.containers.shop.vz.test", horizon: dmz) == ["10.95.0.10"])
    #expect(zone.addresses(for: "api", horizon: dmz) == ["10.95.0.10"])
    #expect(zone.ptrNames(for: "10.0.80.10.in-addr.arpa") == ["web.dmz.shop.vz.test"])
    #expect(zone.ptrNames(for: "10.0.95.10.in-addr.arpa") == ["api.containers.shop.vz.test"])
}

@Test func ptrWireResponseContainsCanonicalMachineName() {
    let server = DNSServer(configuration: DNSConfiguration(
        hostAddress: "127.0.0.1", hostPort: 0, guestPort: 0,
        ttl: 15, upstream: "127.0.0.1:9"
    ))
    defer { server.shutdown() }
    _ = server.reload(snapshot: NetworkSnapshot(
        networks: [
            NetworkRecord(
                name: "dmz", cidr: "10.80.0.0/24", project: "shop",
                stack: "platform:shop"
            ),
        ],
        attachments: [
            NetworkAttachmentRecord(
                vmID: "shop/web", networkName: "dmz", ip: "10.80.0.10",
                project: "shop", stack: "platform:shop"
            ),
        ]
    ))

    let response = server.response(for: dnsQuery("10.0.80.10.in-addr.arpa", type: 12))

    #expect(read16(response, 6) == 1)
    #expect(response.suffix(encodedDNSName("web.dmz.shop.vz.test").count)
        == encodedDNSName("web.dmz.shop.vz.test"))
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

    #expect(zone.addresses(for: "api-1.lan.shop.vz.test", horizon: guestHorizon(project: "shop")) == ["10.90.0.10"])
    #expect(zone.addresses(for: "api.svc.shop.vz.test", horizon: guestHorizon(project: "shop")) == [
        "10.90.0.10",
        "10.90.0.11",
    ])
    #expect(zone.addresses(for: "metrics.svc.shop.vz.test", horizon: guestHorizon(project: "shop")) == ["10.90.0.10"])
    #expect(zone.addresses(for: "api-1.lan.fallback.vz.test", horizon: guestHorizon()) == nil)
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

@Test func systemResolverResponsePreservesQuestionAndRecordTTL() {
    let query = dnsQuery("example.com")
    let response = SystemDNSResolver.response(
        request: query,
        question: DNSQuestion(
            name: "example.com",
            type: 1,
            dnsClass: 1,
            endOffset: query.count
        ),
        records: [SystemDNSRecord(
            type: 1,
            dnsClass: 1,
            ttl: 42,
            rdata: Data([192, 0, 2, 1])
        )]
    )
    #expect(read16(response, 4) == 1)
    #expect(read16(response, 6) == 1)
    #expect(response.suffix(4) == Data([192, 0, 2, 1]))
}

@Test func systemResolverSelectionCanChangeWithoutDNSRestart() {
    let fake = SwitchingSystemResolver()
    let server = DNSServer(
        configuration: DNSConfiguration(
            hostAddress: "127.0.0.1", hostPort: 0, guestPort: 0,
            ttl: 15, upstream: "system"
        ),
        systemResolve: { request, question in fake.resolve(request, question) }
    )
    defer { server.shutdown() }
    let query = dnsQuery("corp.example")
    #expect(aRecords(in: server.response(for: query)) == ["192.0.2.10"])

    fake.useVPNResolver()
    #expect(aRecords(in: server.response(for: query)) == ["10.20.30.40"])
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

private final class SwitchingSystemResolver: @unchecked Sendable {
    private let lock = NSLock()
    private var address = Data([192, 0, 2, 10])

    func useVPNResolver() { lock.withLock { address = Data([10, 20, 30, 40]) } }

    func resolve(_ request: Data, _ question: DNSQuestion) -> Data {
        let rdata = lock.withLock { address }
        return SystemDNSResolver.response(
            request: request,
            question: question,
            records: [SystemDNSRecord(type: 1, dnsClass: 1, ttl: 15, rdata: rdata)]
        )
    }
}

private enum FixtureError: Error {
    case socket
    case bind
}

private func guestHorizon(
    network: String = "dmz",
    project: String? = "edge-dmz",
    gateway: String = "10.80.0.0"
) -> DNSHorizon {
    let prefix = gateway.split(separator: ".").dropLast().joined(separator: ".")
    return .guest(DNSGuestContext(
        network: network,
        project: project,
        stack: project,
        gateway: gateway,
        hostService: "\(prefix).1"
    ))
}

private func dnsResponseCode(_ response: Data) -> UInt8 {
    response.count >= 4 ? response[3] & 0x0F : 0xFF
}

private func dnsQuery(_ name: String, type: UInt16 = 1) -> Data {
    var data = Data([0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
    for label in name.split(separator: ".") {
        data.append(UInt8(label.utf8.count))
        data.append(contentsOf: label.utf8)
    }
    data.append(0)
    data.append(UInt8((type >> 8) & 0xFF))
    data.append(UInt8(type & 0xFF))
    data.append(contentsOf: [0, 1])
    return data
}

private func encodedDNSName(_ name: String) -> Data {
    var data = Data()
    for label in name.split(separator: ".") {
        data.append(UInt8(label.utf8.count))
        data.append(contentsOf: label.utf8)
    }
    data.append(0)
    return data
}

/// Extract A RDATA from an authoritative response (skips the question section).
private func aRecords(in response: Data) -> [String] {
    guard response.count >= 12 else { return [] }
    let questionCount = Int(read16(response, 4))
    let answerCount = Int(read16(response, 6))
    var offset = 12
    for _ in 0..<questionCount {
        while offset < response.count, response[offset] != 0 {
            let length = Int(response[offset])
            guard length < 0xC0 else {
                offset += 2
                break
            }
            offset += 1 + length
        }
        if offset < response.count, response[offset] == 0 { offset += 1 }
        offset += 4 // type + class
    }
    var ips: [String] = []
    for _ in 0..<answerCount {
        guard offset + 12 <= response.count else { break }
        if response[offset] & 0xC0 == 0xC0 {
            offset += 2
        } else {
            while offset < response.count, response[offset] != 0 {
                let length = Int(response[offset])
                offset += 1 + length
            }
            offset += 1
        }
        guard offset + 10 <= response.count else { break }
        let type = read16(response, offset)
        let rdlen = Int(read16(response, offset + 8))
        offset += 10
        guard offset + rdlen <= response.count else { break }
        if type == 1, rdlen == 4 {
            let bytes = Array(response[offset..<(offset + 4)])
            ips.append(bytes.map(String.init).joined(separator: "."))
        }
        offset += rdlen
    }
    return ips
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
