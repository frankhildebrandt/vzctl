import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

@Test func statusCacheExpiresAndInvalidatesOnYAMLOrMutation() throws {
    var clock = Date()
    var calls = 0
    let cache = RestStatusCache(lifetime: 15, now: { clock })
    func collect(_ config: String = "a") throws -> JSONValue {
        try cache.get(path: "/stack", config: Data(config.utf8)) {
            calls += 1
            return .number(Double(calls))
        }
    }
    #expect(try collect() == .number(1))
    #expect(try collect() == .number(1))
    #expect(try collect("b") == .number(2))
    clock = clock.addingTimeInterval(16)
    #expect(try collect("b") == .number(3))
    cache.invalidate()
    #expect(try collect("b") == .number(4))
}
