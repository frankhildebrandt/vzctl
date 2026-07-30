import Darwin
import Dispatch
import Foundation
import vmnet
import XPC

/// G0 network spike harness (macOS 26+).
///
/// Phase A (`nets`): create two reserved shared vmnet networks with fixed
/// subnets, print host/gateway reservation, keep alive for ifconfig inspection.
///
/// Phase A2 (`activate`): also start a vmnet interface per net — needed for
/// host gateway IPs to appear / become bindable.
///
/// Phase B (`probe-dns`): bind UDP probe ports on candidate host IPs.
///
/// Phase C (`guests`): boot Ubuntu arm64 guests on both nets; collect ping markers.

@main
enum G0Spike {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        let command = args.first ?? "nets"

        do {
            switch command {
            case "nets":
                try await runNets(
                    holdSeconds: Int(args.dropFirst().first ?? "120") ?? 120,
                    activateInterfaces: false
                )
            case "activate":
                try await runNets(
                    holdSeconds: Int(args.dropFirst().first ?? "60") ?? 60,
                    activateInterfaces: true,
                    disableDHCP: true
                )
            case "activate10":
                try await runNets(
                    holdSeconds: Int(args.dropFirst().first ?? "15") ?? 15,
                    activateInterfaces: true,
                    disableDHCP: true,
                    frontendSubnet: "10.78.1.0",
                    backendSubnet: "10.78.2.0"
                )
            case "activate10-dhcp":
                try await runNets(
                    holdSeconds: Int(args.dropFirst().first ?? "15") ?? 15,
                    activateInterfaces: true,
                    disableDHCP: false,
                    frontendSubnet: "10.78.11.0",
                    backendSubnet: "10.78.12.0"
                )
            case "activate-dhcp":
                // Compare: leave DHCP enabled — does host bridge get .1?
                try await runNets(
                    holdSeconds: Int(args.dropFirst().first ?? "20") ?? 20,
                    activateInterfaces: true,
                    disableDHCP: false,
                    frontendSubnet: "192.168.110.0",
                    backendSubnet: "192.168.111.0"
                )
            case "probe-dns":
                try await runDNSProbe(addresses: Array(args.dropFirst()))
            case "guests":
                try await runGuests(holdSeconds: Int(args.dropFirst().first ?? "240") ?? 240)
            case "dnsudp":
                try await runDnsUdp(holdSeconds: Int(args.dropFirst().first ?? "300") ?? 300)
            case "router":
                try await runRouter(holdSeconds: Int(args.dropFirst().first ?? "360") ?? 360)
            case "hold-crash":
                let withGuest = args.contains("--guest")
                try await runHoldCrash(withGuest: withGuest)
            case "recreate-probe":
                try await runRecreateProbe()
            case "help", "-h", "--help":
                printUsage()
            default:
                fputs("unknown command: \(command)\n", stderr)
                printUsage()
                exit(2)
            }
        } catch {
            fputs("error: \(error)\n", stderr)
            exit(1)
        }
    }

    static func printUsage() {
        print(
            """
            G0Spike — vzctl network spike harness (macOS 26+)

            Usage:
              G0Spike nets [holdSeconds=120]
              G0Spike activate [holdSeconds=60]          # DHCP off, 10.77.1/2
              G0Spike activate10 [holdSeconds=15]        # DHCP off, 10.78.1/2
              G0Spike activate10-dhcp [holdSeconds=15]   # DHCP on,  10.78.11/12
              G0Spike activate-dhcp [holdSeconds=20]
              G0Spike probe-dns <ip> [<ip>...]
              G0Spike guests [holdSeconds=240]           # Phase C Ubuntu guests
              G0Spike dnsudp [holdSeconds=300]           # Guest→Host UDP/TCP on .0
              G0Spike router [holdSeconds=360]           # Router-VM Cross-Net
              G0Spike hold-crash [--guest]               # Phase D: hold until kill -9
              G0Spike recreate-probe                     # Phase D: after kill, recreate test
              G0Spike help
            """
        )
    }
}

// MARK: - Phase A: dual networks

struct SpikeNetwork: @unchecked Sendable {
    let name: String
    let subnetCIDR: String
    let network: vmnet_network_ref
    let subnet: in_addr
    let mask: in_addr

    var hostGateway: in_addr {
        // API contract: first = network, second = host, last = broadcast.
        let base = UInt32(bigEndian: subnet.s_addr) & UInt32(bigEndian: mask.s_addr)
        var host = in_addr()
        host.s_addr = (base + 1).bigEndian
        return host
    }

    var firstGuest: in_addr {
        let base = UInt32(bigEndian: subnet.s_addr) & UInt32(bigEndian: mask.s_addr)
        var guest = in_addr()
        guest.s_addr = (base + 2).bigEndian
        return guest
    }

    var broadcast: in_addr {
        let base = UInt32(bigEndian: subnet.s_addr) & UInt32(bigEndian: mask.s_addr)
        let inv = ~UInt32(bigEndian: mask.s_addr)
        var bcast = in_addr()
        bcast.s_addr = (base | inv).bigEndian
        return bcast
    }
}

func runNets(
    holdSeconds: Int,
    activateInterfaces: Bool,
    disableDHCP: Bool = true,
    frontendSubnet: String = "10.77.1.0",
    backendSubnet: String = "10.77.2.0"
) async throws {
    print("== G0 Phase A: dual shared vmnet networks ==")
    print("host: \(ProcessInfo.processInfo.operatingSystemVersionString)")
    print("time: \(ISO8601DateFormatter().string(from: Date()))")
    print("activateInterfaces: \(activateInterfaces)  disableDHCP: \(disableDHCP)")
    print("subnets: \(frontendSubnet)/24 , \(backendSubnet)/24")
    print()

    // Fixed private /24s outside common LAN ranges. Prefer stopInterface cleanup —
    // unclean exit can leave subnet reservations that make recreate fail.
    let frontend = try createSharedNetwork(
        name: "frontend",
        subnet: frontendSubnet,
        mask: "255.255.255.0",
        disableDHCP: disableDHCP,
        disableDNSProxy: true
    )
    let backend = try createSharedNetwork(
        name: "backend",
        subnet: backendSubnet,
        mask: "255.255.255.0",
        disableDHCP: disableDHCP,
        disableDNSProxy: true
    )

    for net in [frontend, backend] {
        printNetworkReport(net)
    }

    printProposedIPConvention(frontend: frontend, backend: backend)

    var ifaces: [interface_ref] = []
    if activateInterfaces {
        print("== activating vmnet interfaces (host bridge inet should appear) ==")
        for net in [frontend, backend] {
            let iface = try startInterface(on: net)
            ifaces.append(iface)
        }
        print()
    }

    printHostInterfaces()
    print()
    print("== bind probe on bridge inet (.0) and API-reserved host (.1) ==")
    let candidates = [
        ipv4String(frontend.subnet),
        ipv4String(frontend.hostGateway),
        ipv4String(backend.subnet),
        ipv4String(backend.hostGateway),
        "127.0.0.1",
    ]
    for ip in candidates {
        do {
            let fd = try bindUDP(address: ip, port: 15353)
            print("OK   bind \(ip):15353")
            close(fd)
        } catch {
            print("FAIL bind \(ip):15353  \(error)")
        }
    }

    print()
    print("holding networks for \(holdSeconds)s — inspect with: ifconfig | rg 'bridge|vmenet|10\\.77'")
    print("Ctrl-C to exit early; interfaces will be stopped on normal exit.")
    try await Task.sleep(for: .seconds(holdSeconds))

    for iface in ifaces {
        stopInterface(iface)
    }
    withExtendedLifetime(frontend.network) { _ in }
    withExtendedLifetime(backend.network) { _ in }
    print("stopped interfaces / released networks")
}

func startInterface(on net: SpikeNetwork) throws -> interface_ref {
    let desc = xpc_dictionary_create(nil, nil, 0)
    xpc_dictionary_set_bool(desc, vmnet_allocate_mac_address_key, true)

    let queue = DispatchQueue(label: "g0.vmnet.\(net.name)")
    let sem = DispatchSemaphore(value: 0)
    var completionStatus: vmnet_return_t = .VMNET_FAILURE
    var completionParams: xpc_object_t?

    guard let iface = vmnet_interface_start_with_network(net.network, desc, queue, { status, params in
        completionStatus = status
        completionParams = params
        sem.signal()
    }) else {
        throw SpikeError.vmnet("interface_start_with_network(\(net.name)) returned nil", .VMNET_FAILURE)
    }

    sem.wait()
    guard completionStatus == .VMNET_SUCCESS else {
        throw SpikeError.vmnet("interface_start(\(net.name))", completionStatus)
    }

    print("--- interface \(net.name) started ---")
    if let params = completionParams {
        if let mac = xpc_dictionary_get_string(params, vmnet_mac_address_key) {
            print("mac: \(String(cString: mac))")
        }
        // Dump a few known keys if present.
        for key in [vmnet_mtu_key, vmnet_max_packet_size_key] {
            if xpc_dictionary_get_value(params, key) != nil {
                let v = xpc_dictionary_get_uint64(params, key)
                print("\(String(cString: key)): \(v)")
            }
        }
    }
    return iface
}

func stopInterface(_ iface: interface_ref) {
    let queue = DispatchQueue(label: "g0.vmnet.stop")
    let sem = DispatchSemaphore(value: 0)
    let status = vmnet_stop_interface(iface, queue) { _ in
        sem.signal()
    }
    if status == .VMNET_SUCCESS {
        sem.wait()
        print("stopped interface")
    } else {
        print("warn: vmnet_stop_interface schedule failed: \(vmnetStatusName(status))")
    }
}

func createSharedNetwork(
    name: String,
    subnet: String,
    mask: String,
    disableDHCP: Bool,
    disableDNSProxy: Bool
) throws -> SpikeNetwork {
    var status: vmnet_return_t = .VMNET_SUCCESS
    guard let config = vmnet_network_configuration_create(.VMNET_SHARED_MODE, &status) else {
        throw SpikeError.vmnet("configuration_create(\(name))", status)
    }
    // Config is CF-retained; process exit is fine for this short-lived spike harness.

    var subnetAddr = try parseIPv4(subnet)
    var maskAddr = try parseIPv4(mask)
    status = vmnet_network_configuration_set_ipv4_subnet(config, &subnetAddr, &maskAddr)
    guard status == .VMNET_SUCCESS else {
        throw SpikeError.vmnet("set_ipv4_subnet(\(name))", status)
    }

    if disableDHCP {
        vmnet_network_configuration_disable_dhcp(config)
    }
    if disableDNSProxy {
        vmnet_network_configuration_disable_dns_proxy(config)
    }

    guard let network = vmnet_network_create(config, &status) else {
        throw SpikeError.vmnet("network_create(\(name))", status)
    }

    var gotSubnet = in_addr()
    var gotMask = in_addr()
    vmnet_network_get_ipv4_subnet(network, &gotSubnet, &gotMask)

    return SpikeNetwork(
        name: name,
        subnetCIDR: "\(subnet)/24",
        network: network,
        subnet: gotSubnet,
        mask: gotMask
    )
}

func printNetworkReport(_ net: SpikeNetwork) {
    print("--- network \(net.name) ---")
    print("requested/got subnet: \(ipv4String(net.subnet))/\(prefixLength(net.mask))  mask=\(ipv4String(net.mask))")
    print("reserved host/gateway (2nd addr): \(ipv4String(net.hostGateway))")
    print("first assignable guest:           \(ipv4String(net.firstGuest))")
    print("broadcast (last):                 \(ipv4String(net.broadcast))")
    print("vmnet_network_ref retained:       yes (process-local)")
    print()
}

func printProposedIPConvention(frontend: SpikeNetwork, backend: SpikeNetwork) {
    print("== proposed IP convention (draft for #4 / #5) ==")
    print(
        """
        API docs: 1st/2nd/last reserved; 2nd = host.
        Empirical (DHCP off + interface started): bridge inet = .0 (bindable);
        .1 is NOT present on host → bind EADDRNOTAVAIL. Guest gateway still TBD (need VM).

        | role                      | \(frontend.name)           | \(backend.name)            |
        |---------------------------|-----------------------|-----------------------|
        | subnet                    | \(frontend.subnetCIDR)          | \(backend.subnetCIDR)           |
        | host bridge inet / bind   | \(ipv4String(frontend.subnet))            | \(ipv4String(backend.subnet))             |
        | API-reserved 2nd (.1)     | \(ipv4String(frontend.hostGateway))            | \(ipv4String(backend.hostGateway))             |
        | router-vm nic (draft)     | \(ipv4String(offset(frontend.subnet, 2)))            | \(ipv4String(offset(backend.subnet, 2)))             |
        | guest pool (draft)        | .10–.250              | .10–.250              |

        Do not assign VMs .0 or .1 until guest-path verified.
        """
    )
}

func printHostInterfaces() {
    print("== host interfaces (snapshot) ==")
    let task = Process()
    task.executableURL = URL(fileURLWithPath: "/sbin/ifconfig")
    task.arguments = ["-a"]
    let pipe = Pipe()
    task.standardOutput = pipe
    try? task.run()
    task.waitUntilExit()
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    guard let text = String(data: data, encoding: .utf8) else { return }
    for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
        let s = String(line)
        if s.range(of: #"^(bridge|vmenet|en\d|lo0)"#, options: .regularExpression) != nil
            || s.contains("inet ")
            || s.contains("member:")
        {
            print(s)
        }
    }
}

// MARK: - Phase B: DNS bind probe

func runDNSProbe(addresses: [String]) async throws {
    let targets = addresses.isEmpty ? ["127.0.0.1", "192.168.100.1", "192.168.101.1"] : addresses
    print("== G0 Phase B: DNS bind probe ==")
    print("binding UDP :15353 on candidates (does not prove guest reachability yet)")
    print()

    var sockets: [Int32] = []
    defer {
        for fd in sockets { close(fd) }
    }

    for ip in targets {
        do {
            let fd = try bindUDP(address: ip, port: 15353)
            sockets.append(fd)
            print("OK   bind \(ip):15353  fd=\(fd)")
        } catch {
            print("FAIL bind \(ip):15353  \(error)")
        }
    }

    if sockets.isEmpty {
        throw SpikeError.message("no binds succeeded")
    }

    print()
    print("holding \(sockets.count) listeners for 30s…")
    try await Task.sleep(for: .seconds(30))
}

func bindUDP(address: String, port: UInt16) throws -> Int32 {
    let fd = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
    guard fd >= 0 else { throw SpikeError.errno("socket", errno) }

    var yes: Int32 = 1
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))

    var addr = sockaddr_in()
    addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
    addr.sin_family = sa_family_t(AF_INET)
    addr.sin_port = port.bigEndian
    let parsed = inet_pton(AF_INET, address, &addr.sin_addr)
    guard parsed == 1 else {
        close(fd)
        throw SpikeError.message("inet_pton failed for \(address)")
    }

    let bindResult = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
            bind(fd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
        }
    }
    guard bindResult == 0 else {
        let e = errno
        close(fd)
        throw SpikeError.errno("bind \(address):\(port)", e)
    }
    return fd
}

// MARK: - Helpers

enum SpikeError: Error, CustomStringConvertible {
    case vmnet(String, vmnet_return_t)
    case errno(String, Int32)
    case message(String)

    var description: String {
        switch self {
        case .vmnet(let op, let status):
            "vmnet \(op) failed: \(vmnetStatusName(status)) (\(status))"
        case .errno(let op, let code):
            "\(op) failed: \(String(cString: strerror(code))) (\(code))"
        case .message(let m):
            m
        }
    }
}

func vmnetStatusName(_ status: vmnet_return_t) -> String {
    switch status {
    case .VMNET_SUCCESS: "SUCCESS"
    case .VMNET_FAILURE: "FAILURE"
    case .VMNET_MEM_FAILURE: "MEM_FAILURE"
    case .VMNET_INVALID_ARGUMENT: "INVALID_ARGUMENT"
    case .VMNET_SETUP_INCOMPLETE: "SETUP_INCOMPLETE"
    case .VMNET_INVALID_ACCESS: "INVALID_ACCESS"
    case .VMNET_PACKET_TOO_BIG: "PACKET_TOO_BIG"
    case .VMNET_BUFFER_EXHAUSTED: "BUFFER_EXHAUSTED"
    case .VMNET_TOO_MANY_PACKETS: "TOO_MANY_PACKETS"
    case .VMNET_SHARING_SERVICE_BUSY: "SHARING_SERVICE_BUSY"
    default: "UNKNOWN"
    }
}

func parseIPv4(_ s: String) throws -> in_addr {
    var addr = in_addr()
    guard inet_pton(AF_INET, s, &addr) == 1 else {
        throw SpikeError.message("invalid IPv4: \(s)")
    }
    return addr
}

func ipv4String(_ addr: in_addr) -> String {
    var a = addr
    var buf = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
    inet_ntop(AF_INET, &a, &buf, socklen_t(INET_ADDRSTRLEN))
    return String(decoding: buf.prefix { $0 != 0 }.map(UInt8.init), as: UTF8.self)
}

func prefixLength(_ mask: in_addr) -> Int {
    let m = UInt32(bigEndian: mask.s_addr)
    return m.nonzeroBitCount
}

func offset(_ subnet: in_addr, _ n: UInt32) -> in_addr {
    let base = UInt32(bigEndian: subnet.s_addr)
    var out = in_addr()
    out.s_addr = (base + n).bigEndian
    return out
}
