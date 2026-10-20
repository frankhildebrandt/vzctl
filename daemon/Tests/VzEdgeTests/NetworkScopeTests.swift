import Foundation
import Testing
import VzDaemonKit
@testable import VzEdge

@Suite struct NetworkScopeTests {
    private func snapshot() throws -> NetworkSnapshot {
        try EdgeServer.parseSnapshot(.object([
            "networks": .array([
                // Existing physical IDs survive migration unchanged.
                network(id: "lan", project: "old", cidr: "10.71.0.0/24"),
                network(id: "new/lan", project: "new", cidr: "10.72.0.0/24"),
            ]),
            "attachments": .array([
                attachment(id: "lan", project: "old", ip: "10.71.0.10"),
                attachment(id: "new/lan", project: "new", ip: "10.72.0.10"),
            ]),
        ]))
    }

    @Test func sameShortNameKeepsOwnersAndDNSLabelsSeparate() throws {
        let snapshot = try snapshot()
        #expect(snapshot.networks.map(\.id) == ["lan", "new/lan"])
        #expect(snapshot.networks.map(\.name) == ["lan", "lan"])
        #expect(snapshot.attachments.map(\.networkID) == ["lan", "new/lan"])
        let zone = DNSZoneBuilder.build(snapshot: snapshot, ttl: 15)
        #expect(zone.addresses(for: "web.lan.old.vz.test", horizon: .host) == ["10.71.0.10"])
        #expect(zone.addresses(for: "web.lan.new.vz.test", horizon: .host) == ["10.72.0.10"])
        #expect(zone.addresses(for: "web.new/lan.new.vz.test", horizon: .host) == nil)
        #expect(zone.addresses(for: "web", horizon: horizon(id: "lan", project: "old")) == ["10.71.0.10"])
        #expect(zone.addresses(for: "web", horizon: horizon(id: "new/lan", project: "new")) == ["10.72.0.10"])
        #expect(zone.addresses(for: "web", horizon: horizon(id: "lan", project: "new")) == nil)
    }

    @Test func reloadUsesStableIDsForGuestLookupAndIngressOverride() throws {
        let server = DNSServer(configuration: DNSConfiguration(
            hostAddress: "127.0.0.1", hostPort: 0, guestPort: 0, ttl: 15,
            upstream: "127.0.0.1:9"
        ))
        defer { server.shutdown() }
        server.setIngressHostServices(
            hosts: ["app.svc.new.vz.test"],
            guestAddresses: ["app.svc.new.vz.test": ["new/lan": "10.72.0.2"]]
        )
        _ = server.reload(snapshot: try snapshot())
        #expect(server.lookup("web", network: "lan").guest == ["10.71.0.10"])
        #expect(server.lookup("web", network: "new/lan").guest == ["10.72.0.10"])
        #expect(server.lookup("app.svc.new.vz.test", network: "new/lan").guest == ["10.72.0.2"])
        #expect(server.lookup("app.svc.new.vz.test", network: "lan").guest == nil)
    }

    @Test func legacyWireSnapshotsStillResolve() throws {
        let snapshot = try EdgeServer.parseSnapshot(.object([
            "networks": .array([.object([
                "name": .string("lan"), "cidr": .string("10.71.0.0/24"),
                "project": .string("old"),
            ])]),
            "attachments": .array([.object([
                "vm_id": .string("old/web"), "network": .string("lan"),
                "ip": .string("10.71.0.10"), "project": .string("old"),
            ])]),
        ]))
        #expect(snapshot.networks[0].id == "lan")
        #expect(snapshot.attachments[0].networkID == "lan")
        #expect(DNSZoneBuilder.build(snapshot: snapshot, ttl: 15)
            .addresses(for: "web.lan.old.vz.test", horizon: .host) == ["10.71.0.10"])
    }

    @Test func missingAttachmentIDRequiresGloballyUniqueShortName() throws {
        let legacyAttachment: JSONValue = .object([
            "vm_id": .string("old/web"), "network": .string("lan"),
            "ip": .string("10.71.0.10"), "project": .string("old"),
        ])
        #expect(throws: EdgeServerError.self) {
            try EdgeServer.parseSnapshot(.object([
                "networks": .array([
                    network(id: "lan", project: "old", cidr: "10.71.0.0/24"),
                    network(id: "new/lan", project: "new", cidr: "10.72.0.0/24"),
                ]),
                "attachments": .array([legacyAttachment]),
            ]))
        }
        let unique = try EdgeServer.parseSnapshot(.object([
            "networks": .array([network(id: "old/lan", project: "old", cidr: "10.71.0.0/24")]),
            "attachments": .array([legacyAttachment]),
        ]))
        #expect(unique.attachments[0].networkID == "old/lan")

        // An explicit legacy ID is unambiguous even across historical owners.
        let explicit = try EdgeServer.parseSnapshot(.object([
            "networks": .array([
                network(id: "lan", project: "old", cidr: "10.71.0.0/24"),
                network(id: "new/lan", project: "new", cidr: "10.72.0.0/24"),
            ]),
            "attachments": .array([attachment(id: "lan", project: "guest", ip: "10.71.0.10")]),
        ]))
        #expect(explicit.attachments[0].networkID == "lan")
        #expect(DNSZoneBuilder.build(snapshot: explicit, ttl: 15)
            .addresses(for: "web.lan.guest.vz.test", horizon: .host) == ["10.71.0.10"])
    }

    @Test func unknownOrMismatchedNetworkReferenceCannotBorrowAnotherNetwork() throws {
        let source = try snapshot()
        for (id, name) in [("missing/lan", "lan"), ("new/lan", "wrong-name"), ("lan", "lan")] {
            let bad = try EdgeServer.parseSnapshot(.object([
                "networks": .array(source.networks.map(\.json)),
                "attachments": .array([.object([
                    "network_id": .string(id), "network": .string(name),
                    "vm_id": .string("new/web"), "project": .string("new"),
                    "ip": .string("10.72.0.20"),
                ])]),
            ]))
            let zone = DNSZoneBuilder.build(snapshot: bad, ttl: 15)
            #expect(zone.addresses(for: "web.lan.new.vz.test", horizon: .host) == nil)
            #expect(zone.addresses(for: "web.lan.old.vz.test", horizon: .host) == nil)
        }
    }

    @Test func duplicateIDsAndPhysicalCIDRsFailBeforeApplying() throws {
        for second in [
            network(id: "lan", project: "new", cidr: "10.72.0.0/24"),
            network(id: "new/lan", project: "new", cidr: "10.71.0.0/24"),
        ] {
            #expect(throws: EdgeServerError.self) {
                try EdgeServer.parseSnapshot(.object([
                    "networks": .array([
                        network(id: "lan", project: "old", cidr: "10.71.0.0/24"), second,
                    ]),
                ]))
            }
        }
    }

    @Test func zoneBuilderDoesNotTrapOrPickAnOwnerForDuplicateIDs() throws {
        let source = try snapshot()
        var duplicate = source.networks[1]
        duplicate.id = source.networks[0].id
        let zone = DNSZoneBuilder.build(
            snapshot: NetworkSnapshot(
                networks: [source.networks[0], duplicate],
                attachments: source.attachments
            ), ttl: 15
        )
        #expect(zone.addresses(for: "web.lan.old.vz.test", horizon: .host) == nil)
        #expect(zone.addresses(for: "web.lan.new.vz.test", horizon: .host) == nil)
    }

    private func network(id: String, project: String, cidr: String) -> JSONValue {
        .object([
            "id": .string(id), "name": .string("lan"), "project": .string(project),
            "cidr": .string(cidr), "runtime_state": .string("active"),
        ])
    }

    private func attachment(id: String, project: String, ip: String) -> JSONValue {
        .object([
            "network_id": .string(id), "network": .string("lan"),
            "vm_id": .string("\(project)/web"), "project": .string(project), "ip": .string(ip),
        ])
    }

    private func horizon(id: String, project: String) -> DNSHorizon {
        .guest(DNSGuestContext(
            network: id, project: project, stack: project,
            gateway: "10.72.0.0", hostService: "10.72.0.1"
        ))
    }
}
