import Foundation
import Testing
@testable import VzDaemonKit

@Test func requestRoundTripUsesNewlineFraming() throws {
    let request = JSONRPCRequest(
        method: "daemon.health",
        params: .object(["verbose": .bool(true)]),
        id: .number(7)
    )

    let encoded = try JSONRPCFraming.encode(request)
    #expect(encoded.last == 0x0A)
    #expect(try JSONRPCFraming.decode(JSONRPCRequest.self, from: encoded) == request)
}

@Test func responseRoundTripPreservesHealthPayload() throws {
    let response = JSONRPCResponse(
        result: .object(["ok": .bool(true), "db_ok": .bool(true)]),
        id: .string("health-1")
    )

    let encoded = try JSONRPCFraming.encode(response)
    #expect(try JSONRPCFraming.decode(JSONRPCResponse.self, from: encoded) == response)
}
