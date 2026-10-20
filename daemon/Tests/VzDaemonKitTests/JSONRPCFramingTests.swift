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

@Test func foundationConversionPreservesNumbersAndBooleans() throws {
    let data = Data(#"{"zero":0,"one":1,"two":2,"fraction":0.25,"yes":true,"no":false,"nested":[0,1,true,null]}"#.utf8)
    let foundation = try JSONSerialization.jsonObject(with: data)
    let expected = try JSONDecoder().decode(JSONValue.self, from: data)
    #expect(JSONValue.fromAny(foundation) == expected)
    #expect(JSONValue.fromAny(0) == .number(0))
    #expect(JSONValue.fromAny(1) == .number(1))
    #expect(JSONValue.fromAny(true) == .bool(true))
    #expect(JSONValue.fromAny(false) == .bool(false))
}
