import Testing
@testable import VzDaemonKit

@Test func routerPlanRequiresTwoDotTwoAttachments() throws {
    let networks = [
        NetworkRecord(name: "dmz", cidr: "10.80.0.0/24"),
        NetworkRecord(name: "lan", cidr: "10.90.0.0/24"),
    ]
    let attachments = [
        NetworkAttachmentRecord(vmID: "router", networkName: "dmz", ip: "10.80.0.2"),
        NetworkAttachmentRecord(vmID: "router", networkName: "lan", ip: "10.90.0.2"),
    ]

    let plan = try RouterPlan(
        vmID: "router",
        networkRecords: networks,
        attachments: attachments
    )

    #expect(plan.networks.map(\.address) == ["10.80.0.2", "10.90.0.2"])
    #expect(IPv4CIDR.gateway(for: networks[0].cidr) == "10.80.0.0")
    #expect(IPv4CIDR.router(for: networks[0].cidr) == "10.80.0.2")
}

@Test func routerPlanRejectsGuestRangeAddressAndSingleNIC() {
    let networks = [
        NetworkRecord(name: "dmz", cidr: "10.80.0.0/24"),
        NetworkRecord(name: "lan", cidr: "10.90.0.0/24"),
    ]
    #expect(throws: RouteApplyError.self) {
        try RouterPlan(
            vmID: "router",
            networkRecords: networks,
            attachments: [
                NetworkAttachmentRecord(
                    vmID: "router",
                    networkName: "dmz",
                    ip: "10.80.0.10"
                ),
                NetworkAttachmentRecord(
                    vmID: "router",
                    networkName: "lan",
                    ip: "10.90.0.2"
                ),
            ]
        )
    }
    #expect(throws: RouteApplyError.self) {
        try RouterPlan(
            vmID: "router",
            networkRecords: networks,
            attachments: [
                NetworkAttachmentRecord(
                    vmID: "router",
                    networkName: "dmz",
                    ip: "10.80.0.2"
                ),
            ]
        )
    }
}

@Test func routerPlanRendersDefaultDenyAndExplicitAllows() throws {
    let plan = try RouterPlan(
        vmID: "router",
        networks: [
            RouterNetwork(name: "dmz", cidr: "10.80.0.0/24", address: "10.80.0.2"),
            RouterNetwork(name: "lan", cidr: "10.90.0.0/24", address: "10.90.0.2"),
        ],
        policies: [
            ForwardPolicy(
                name: "dmz-default",
                network: "dmz",
                forward: "deny-all",
                allow: [
                    PolicyAllow(to: "lan", proto: "tcp", ports: [5432]),
                    PolicyAllow(to: "dmz", proto: "icmp"),
                ]
            ),
        ]
    )

    #expect(plan.nftables.contains("policy drop"))
    #expect(plan.nftables.contains("ct state established,related accept"))
    #expect(plan.nftables.contains("tcp dport 5432 accept"))
    #expect(plan.nftables.contains("ip protocol icmp accept"))
    #expect(plan.rules.count == 2)
}

@Test func routerPlanRendersInternetAllowAndMasquerade() throws {
    let plan = try RouterPlan(
        vmID: "router",
        networks: [
            RouterNetwork(
                name: "lan",
                cidr: "10.90.0.0/24",
                address: "10.90.0.2",
                natEgress: false
            ),
            RouterNetwork(
                name: "wan",
                cidr: "10.80.0.0/24",
                address: "10.80.0.2",
                natEgress: true
            ),
        ],
        policies: [
            ForwardPolicy(
                name: "lan-out",
                network: "lan",
                forward: "deny-all",
                allow: [PolicyAllow(to: "internet", proto: "tcp", ports: [443])]
            ),
        ]
    )
    #expect(plan.nftables.contains("ip daddr != { 10.0.0.0/8"))
    #expect(plan.nftables.contains("tcp dport 443 accept"))
    #expect(plan.nftables.contains("masquerade comment \"vzctl:internet\""))
    #expect(plan.rules.first?.to == "internet")
}

@Test func routerPlanRejectsInternetWithoutNatEgress() {
    #expect(throws: RouteApplyError.self) {
        try RouterPlan(
            vmID: "router",
            networks: [
                RouterNetwork(
                    name: "lan",
                    cidr: "10.90.0.0/24",
                    address: "10.90.0.2",
                    natEgress: false
                ),
                RouterNetwork(
                    name: "dmz",
                    cidr: "10.80.0.0/24",
                    address: "10.80.0.2",
                    natEgress: false
                ),
            ],
            policies: [
                ForwardPolicy(
                    name: "bad",
                    network: "lan",
                    forward: "deny-all",
                    allow: [PolicyAllow(to: "internet", proto: "tcp", ports: [80])]
                ),
            ]
        )
    }
}

@Test func routerPlanRejectsUnknownNetworksAndInvalidPorts() {
    #expect(throws: RouteApplyError.self) {
        try RouterPlan(
            vmID: "router",
            networks: [
                RouterNetwork(name: "dmz", cidr: "10.80.0.0/24", address: "10.80.0.2"),
                RouterNetwork(name: "lan", cidr: "10.90.0.0/24", address: "10.90.0.2"),
            ],
            policies: [
                ForwardPolicy(
                    name: "bad",
                    network: "dmz",
                    forward: "deny-all",
                    allow: [PolicyAllow(to: "missing", proto: "tcp", ports: [0])]
                ),
            ]
        )
    }
}
