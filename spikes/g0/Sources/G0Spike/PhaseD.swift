import Darwin
import Dispatch
import Foundation
import Virtualization
import vmnet

/// Phase D — crash / recreate semantics (monolith harness).
///
/// `hold-crash`: create net (+ optional guest), print CRASH_READY, sleep until killed.
/// `recreate-probe`: try same subnet as last hold, then a fresh subnet.

private let crashStatePath = "/tmp/g0-crash-state.json"

struct CrashState: Codable {
    var pid: Int32
    var subnet: String
    var guestIP: String?
    var startedAt: String
    var withGuest: Bool
}

func runHoldCrash(withGuest: Bool) async throws {
    print("== G0 Phase D: hold-crash ==")
    let subnet = "10.93.1.0"
    let guestIP = "10.93.1.10"

    let net = try createSharedNetwork(
        name: "crashnet",
        subnet: subnet,
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    printNetworkReport(net)
    let iface = try startInterface(on: net)
    print("bridge should show inet \(subnet)")

    var guest: GuestRuntime?
    if withGuest {
        let assets = assetsDir()
        // Expect cidata-crash.iso + crash.raw prepared by script
        guard let mac = VZMACAddress(string: "52:54:00:90:01:10") else {
            throw SpikeError.message("bad mac")
        }
        let spec = GuestSpec(
            name: "crash",
            diskURL: assets.appendingPathComponent("crash.raw"),
            cidataURL: assets.appendingPathComponent("cidata-crash.iso"),
            nvramURL: assets.appendingPathComponent("crash-nvram.bin"),
            serialLogURL: assets.appendingPathComponent("serial-crash.log"),
            mac: mac,
            network: net.network
        )
        for u in [spec.diskURL, spec.cidataURL] {
            guard FileManager.default.fileExists(atPath: u.path) else {
                throw SpikeError.message("missing \(u.lastPathComponent) — run scripts/phase-d-prepare.sh")
            }
        }
        try? FileManager.default.removeItem(at: spec.serialLogURL)
        let g = try GuestRuntime(spec: spec)
        try await g.start()
        guest = g
        let deadline = Date().addingTimeInterval(180)
        var up = false
        while Date() < deadline {
            if hostPing(guestIP) {
                print("HOST_PING_OK \(guestIP)")
                up = true
                break
            }
            try await Task.sleep(for: .seconds(2))
        }
        guard up else { throw SpikeError.message("guest IP timeout") }
    }

    let state = CrashState(
        pid: ProcessInfo.processInfo.processIdentifier,
        subnet: subnet,
        guestIP: withGuest ? guestIP : nil,
        startedAt: ISO8601DateFormatter().string(from: Date()),
        withGuest: withGuest
    )
    let data = try JSONEncoder().encode(state)
    try data.write(to: URL(fileURLWithPath: crashStatePath))
    print("CRASH_READY pid=\(state.pid) subnet=\(subnet) guest=\(withGuest) state=\(crashStatePath)")
    print("waiting to be killed (kill -9 \(state.pid)) — NO cleanup on purpose")

    // Keep refs alive; ignore signals for clean SIGKILL testing
    signal(SIGTERM, SIG_IGN)
    withExtendedLifetime(net.network) { _ in }
    withExtendedLifetime(iface) { _ in }
    withExtendedLifetime(guest) { _ in }
    while true {
        try await Task.sleep(for: .seconds(3600))
    }
}

func runRecreateProbe() async throws {
    print("== G0 Phase D: recreate-probe ==")
    let url = URL(fileURLWithPath: crashStatePath)
    guard let data = try? Data(contentsOf: url),
          let state = try? JSONDecoder().decode(CrashState.self, from: data)
    else {
        throw SpikeError.message("missing \(crashStatePath) — run hold-crash + kill first")
    }
    print("last state: \(state)")

    if let g = state.guestIP {
        let alive = hostPing(g)
        print(alive ? "GUEST_STILL_ALIVE \(g)" : "GUEST_DEAD \(g)")
    }

    // Same subnet as crashed process
    do {
        let net = try createSharedNetwork(
            name: "recreate-same",
            subnet: state.subnet,
            mask: "255.255.255.0",
            disableDHCP: true,
            disableDNSProxy: true
        )
        print("RECREATE_SAME_OK \(state.subnet)")
        let iface = try startInterface(on: net)
        stopInterface(iface)
        withExtendedLifetime(net.network) { _ in }
    } catch {
        print("RECREATE_SAME_FAIL \(state.subnet) — \(error)")
    }

    // Fresh subnet
    let fresh = "10.94.1.0"
    do {
        let net = try createSharedNetwork(
            name: "recreate-fresh",
            subnet: fresh,
            mask: "255.255.255.0",
            disableDHCP: true,
            disableDNSProxy: true
        )
        print("RECREATE_FRESH_OK \(fresh)")
        let iface = try startInterface(on: net)
        stopInterface(iface)
        withExtendedLifetime(net.network) { _ in }
    } catch {
        print("RECREATE_FRESH_FAIL \(fresh) — \(error)")
    }

    // Contrast: stopInterface alone is NOT enough while network_ref retained.
    let clean = "10.95.1.0"
    do {
        let n1 = try createSharedNetwork(
            name: "clean1",
            subnet: clean,
            mask: "255.255.255.0",
            disableDHCP: true,
            disableDNSProxy: true
        )
        let i1 = try startInterface(on: n1)
        stopInterface(i1)
        // Intentionally keep n1.network retained — reservation should still block.
        do {
            _ = try createSharedNetwork(
                name: "clean2",
                subnet: clean,
                mask: "255.255.255.0",
                disableDHCP: true,
                disableDNSProxy: true
            )
            print("CLEAN_STOP_RECREATE_OK \(clean) (unexpected while ref held)")
        } catch {
            print("CLEAN_STOP_RECREATE_BLOCKED \(clean) — stopInterface alone insufficient while network_ref retained: \(error)")
        }
        withExtendedLifetime(n1.network) { _ in }
    } catch {
        print("CLEAN_SETUP_FAIL \(error)")
    }
    print("recreate-probe done")
}
