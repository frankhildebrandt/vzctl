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
