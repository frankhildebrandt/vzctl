import Foundation
import Testing
@testable import VzDaemonKit
@testable import VzNet

@Test func netRuntimeRejectsInvalidCIDRBeforeVmnet() throws {
    let store = NetRuntimeStore()
    defer { store.shutdown() }
    #expect(throws: NetRuntimeError.self) {
        try store.acquire(
            name: "bad",
            cidr: "10.80.0.1/24",
            mode: "shared",
            natEgress: true
        )
    }
}

@Test func netRuntimeRejectsBridgedMode() throws {
    let store = NetRuntimeStore()
    defer { store.shutdown() }
    #expect(throws: NetRuntimeError.self) {
        try store.acquire(
            name: "br",
            cidr: "10.80.0.0/24",
            mode: "bridged",
            natEgress: true
        )
    }
}

@Test func vzNetNetworkInfoRoundTripsJSON() throws {
    let info = VzNetNetworkInfo(
        name: "dmz",
        cidr: "10.80.0.0/24",
        mode: "shared",
        natEgress: true,
        gateway: "10.80.0.0"
    )
    let parsed = try VzNetNetworkInfo(json: info.json)
    #expect(parsed == info)
}

@Test func vzNetVerificationParsesHealthWithoutMutation() throws {
    let verification = try VzNetVerification(json: .object([
        "name": .string("dmz"),
        "cidr": .string("10.80.0.0/24"),
        "ref_ok": .bool(true),
        "serialization_ok": .bool(true),
        "bridge_ok": .bool(false),
        "error": .string("bridge missing"),
    ]))
    #expect(verification.name == "dmz")
    #expect(!verification.ok)
    #expect(verification.error == "bridge missing")
}
