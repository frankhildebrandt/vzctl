import Testing
@testable import VzDaemonKit

@Test func cidrRequiresCanonicalNetworkAndGuestRangeStartsAtTen() throws {
    let cidr = try IPv4CIDR("10.80.0.0/24")
    #expect(cidr.canonical == "10.80.0.0/24")
    #expect(!cidr.containsGuest("10.80.0.2"))
    #expect(cidr.containsAttachment("10.80.0.2"))
    #expect(cidr.containsGuest("10.80.0.10"))
    #expect(cidr.containsGuest("10.80.0.254"))
    #expect(cidr.guestAddress(offset: 10) == "10.80.0.10")
    #expect(cidr.guestAddress(offset: 255) == nil)
    #expect(!cidr.containsGuest("10.80.1.10"))
    #expect(throws: NetworkValidationError.self) {
        try IPv4CIDR("10.80.0.1/24")
    }
}

@Test func cidrOverlapAndContainmentAreIndependentOfLocale() throws {
    let hypernetwork = try IPv4CIDR("10.80.0.0/24")
    #expect(hypernetwork.overlaps(try IPv4CIDR("10.80.0.128/25")))
    #expect(!hypernetwork.overlaps(try IPv4CIDR("10.81.0.0/24")))
    #expect(hypernetwork.contains("10.80.0.10"))
    #expect(!hypernetwork.contains("10.81.0.10"))
}
