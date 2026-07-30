import Foundation
import Testing
@testable import VzDaemonKit

@Test func eventEnvelopeEncodesAsOneNDJSONLine() throws {
    let event = EventEnvelope(
        ts: "2026-07-30T08:30:00.000Z",
        type: "vm.state",
        data: ["vm_id": .string("web"), "state": .string("running")]
    )

    let encoded = try JSONRPCFraming.encode(event)
    #expect(encoded.last == 0x0A)
    #expect(try JSONRPCFraming.decode(EventEnvelope.self, from: encoded) == event)
}

@Test func eventFilterSupportsCommaSeparatedExactAndPrefixPatterns() throws {
    let filter = try EventFilter("vm.*, apply.failed")

    #expect(filter.matches("vm.state"))
    #expect(filter.matches("vm.clock_corrected"))
    #expect(filter.matches("apply.failed"))
    #expect(!filter.matches("apply.started"))
    #expect(!filter.matches("dns.reloaded"))
}

@Test func eventFilterRejectsNonSuffixWildcards() {
    #expect(throws: EventFilterError.invalidPattern("vm.*.failed")) {
        _ = try EventFilter("vm.*.failed")
    }
}
