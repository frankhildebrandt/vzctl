import Darwin
import Foundation
import Testing
import VzDaemonKit

@Test func dnsBindRejectsHighPortsAndBadAddresses() throws {
    #expect(DnsBind.needsPrivilege(port: 53))
    #expect(!DnsBind.needsPrivilege(port: 15353))

    #expect(throws: DnsBind.ValidationError.portNotPrivileged(15353)) {
        try DnsBind.validate(DnsBind.BindRequest(address: "10.80.0.0", port: 15353))
    }
    #expect(throws: DnsBind.ValidationError.invalidAddress("not-an-ip")) {
        try DnsBind.validate(DnsBind.BindRequest(address: "not-an-ip", port: 53))
    }
    #expect(throws: DnsBind.ValidationError.portInvalid) {
        try DnsBind.validate(DnsBind.BindRequest(address: "10.80.0.0", port: 0))
    }

    let ok = try DnsBind.parseRequest(Data(#"{"op":"bind","address":"10.80.0.0","port":53}"#.utf8))
    #expect(ok.address == "10.80.0.0")
    #expect(ok.port == 53)
    #expect(ok.proto == DnsBind.protoUDP)

    let tcp = try DnsBind.parseRequest(
        Data(#"{"op":"bind","address":"10.80.0.0","port":443,"proto":"tcp"}"#.utf8)
    )
    #expect(tcp.proto == DnsBind.protoTCP)
    #expect(tcp.port == 443)

    #expect(throws: DnsBind.ValidationError.invalidProto("sctp")) {
        try DnsBind.validate(DnsBind.BindRequest(address: "10.80.0.0", port: 80, proto: "sctp"))
    }

    #expect(throws: DnsBind.ValidationError.invalidJSON) {
        try DnsBind.parseRequest(Data(#"{"op":"nope"}"#.utf8))
    }
    #expect(throws: DnsBind.ValidationError.unsupportedOp("listen")) {
        try DnsBind.parseRequest(Data(#"{"op":"listen","address":"10.80.0.0","port":53}"#.utf8))
    }

    let alias = try DnsBind.parseOperation(
        Data(#"{"op":"alias.ensure","cidr":"10.90.0.0/24"}"#.utf8)
    )
    #expect(alias == .alias(.init(op: DnsBind.opAliasEnsure, cidr: "10.90.0.0/24")))
    for cidr in ["10.0.0.0/8", "10.90.0.0/30"] {
        let request = try DnsBind.parseOperation(
            Data(#"{"op":"alias.ensure","cidr":"\#(cidr)"}"#.utf8)
        )
        #expect(request == .alias(.init(op: DnsBind.opAliasEnsure, cidr: cidr)))
    }
    #expect(throws: DnsBind.ValidationError.invalidCIDR("10.90.0.1/24")) {
        try DnsBind.parseOperation(
            Data(#"{"op":"alias.ensure","cidr":"10.90.0.1/24"}"#.utf8)
        )
    }

    let firewall = try DnsBind.parseOperation(Data(#"""
    {
      "op":"firewall.reconcile",
      "bindings":[{
        "cidr":"10.90.0.0/24",
        "allowed_sources":["10.90.0.0/24","10.95.0.0/24"],
        "tcp_ports":[80,443]
      }]
    }
    """#.utf8))
    #expect(firewall == .firewall(.init(bindings: [
        .init(
            cidr: "10.90.0.0/24",
            allowedSources: ["10.90.0.0/24", "10.95.0.0/24"],
            tcpPorts: [80, 443]
        ),
    ])))

    let rules = try DnsBind.firewallRules(
        bindings: [
            .init(
                cidr: "10.90.0.0/24",
                allowedSources: ["10.95.0.0/24", "10.90.0.0/24"],
                tcpPorts: [443, 80]
            ),
            .init(cidr: "10.80.0.0/24", allowedSources: ["10.80.0.0/24"], tcpPorts: []),
        ],
        interfaceByCIDR: ["10.80.0.0/24": "bridge100", "10.90.0.0/24": "bridge101"]
    )
    #expect(rules.contains("block in quick on bridge100 inet from any to 10.80.0.1"))
    #expect(rules.contains(
        "pass in quick on bridge101 inet proto tcp from { 10.90.0.0/24, 10.95.0.0/24 } to 10.90.0.1 port { 80, 443 }"
    ))
    #expect(rules.contains("block in quick on bridge101 inet from any to 10.90.0.1"))
}

@Test func unixFDPassingRoundTripsDescriptor() throws {
    var sockets = [Int32](repeating: -1, count: 2)
    guard Darwin.socketpair(AF_UNIX, SOCK_STREAM, 0, &sockets) == 0 else {
        Issue.record("socketpair failed: \(errno)")
        return
    }
    defer {
        Darwin.close(sockets[0])
        Darwin.close(sockets[1])
    }

    let bound = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
    guard bound >= 0 else {
        Issue.record("socket failed")
        return
    }
    defer { Darwin.close(bound) }

    var addr = sockaddr_in()
    addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = UInt16(0).bigEndian
    addr.sin_addr = in_addr(s_addr: inet_addr("127.0.0.1"))
    let bindResult = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            Darwin.bind(bound, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    #expect(bindResult == 0)

    let payload = Data(#"{"ok":true}"#.utf8)
    try UnixFDPassing.send(payload: payload, fileDescriptor: bound, on: sockets[0])
    let (received, fd) = try UnixFDPassing.receive(on: sockets[1])
    #expect(received == payload)
    #expect(fd != nil)
    if let fd {
        Darwin.close(fd)
    }
}
