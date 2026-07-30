import Darwin
import Dispatch
import Foundation
import Virtualization
import vmnet

// MARK: - Phase C+/D: DNS transport + Router cross-net

private let g0Prefix = "10.85"
private var feNet: String { "\(g0Prefix).1.0" }
private var beNet: String { "\(g0Prefix).2.0" }
private var feHost: String { "\(g0Prefix).1.0" }
private var beHost: String { "\(g0Prefix).2.0" }
private var feGuest: String { "\(g0Prefix).1.10" }
private var beGuest: String { "\(g0Prefix).2.10" }
private var feRouter: String { "\(g0Prefix).1.2" }
private var beRouter: String { "\(g0Prefix).2.2" }

func runDnsUdp(holdSeconds: Int) async throws {
    print("== G0 DNS-UDP/TCP transport spike ==")
    print("subnet prefix \(g0Prefix).x.0/24")
    let assets = assetsDir()

    let frontend = try createSharedNetwork(
        name: "frontend",
        subnet: feNet,
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    printNetworkReport(frontend)
    let ifFront = try startInterface(on: frontend)
    printHostInterfaces()

    let udp = try UDPEchoServer(bindAddress: feHost, port: 15353)
    print("UDP echo on \(feHost):15353")
    let tcp = try TCPEchoServer(bindAddress: feHost, port: 18080)
    print("TCP echo on \(feHost):18080")

    // Also probe from host loopback path for sanity
    do {
        let fd = try bindUDP(address: "127.0.0.1", port: 15354)
        print("OK host can also bind 127.0.0.1:15354")
        close(fd)
    } catch {
        print("warn: \(error)")
    }

    guard let macFE = VZMACAddress(string: "52:54:00:83:01:10") else {
        throw SpikeError.message("bad MAC")
    }

    let feSpec = GuestSpec(
        name: "frontend",
        diskURL: assets.appendingPathComponent("frontend.raw"),
        cidataURL: assets.appendingPathComponent("cidata-fe.iso"),
        nvramURL: assets.appendingPathComponent("frontend-nvram.bin"),
        serialLogURL: assets.appendingPathComponent("serial-frontend.log"),
        mac: macFE,
        network: frontend.network
    )
    guard FileManager.default.fileExists(atPath: feSpec.diskURL.path) else {
        throw SpikeError.message("missing \(feSpec.diskURL.path)")
    }

    try? FileManager.default.removeItem(at: feSpec.serialLogURL)
    let fe = try GuestRuntime(spec: feSpec)
    try await fe.start()

    let deadline = Date().addingTimeInterval(TimeInterval(holdSeconds))
    var up = false
    while Date() < deadline {
        if hostPing(feGuest) {
            print("HOST_PING_OK \(feGuest)")
            up = true
            break
        }
        try await Task.sleep(for: .seconds(3))
    }
    guard up else { throw SpikeError.message("guest IP timeout") }

    print("== Guest transport probes ==")
    let out = try sshProbe(
        host: feGuest,
        script: """
          set -x
          echo '=== G0 TRANSPORT START ==='
          ip -4 addr; ip route
          # ICMP
          if ping -c 2 -W 2 \(feHost); then echo G0_PING_OK \(feHost); else echo G0_PING_FAIL \(feHost); fi
          if ping -c 2 -W 2 \(g0Prefix).1.1; then echo G0_PING_OK \(g0Prefix).1.1; else echo G0_PING_FAIL \(g0Prefix).1.1; fi
          # UDP DNS-like
          python3 - <<'PY'
          import socket, sys
          def probe(ip, port):
              s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
              s.settimeout(2)
              try:
                  s.sendto(b'G0DNS', (ip, port))
                  data, _ = s.recvfrom(64)
                  print(f'G0_UDP_OK {ip}:{port} reply={data!r}')
              except Exception as e:
                  print(f'G0_UDP_FAIL {ip}:{port} {e}')
              finally:
                  s.close()
          probe('\(feHost)', 15353)
          probe('\(g0Prefix).1.1', 15353)
          PY
          # TCP
          python3 - <<'PY'
          import socket
          def probe(ip, port):
              s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
              s.settimeout(2)
              try:
                  s.connect((ip, port))
                  s.sendall(b'G0TCP')
                  data = s.recv(64)
                  print(f'G0_TCP_OK {ip}:{port} reply={data!r}')
              except Exception as e:
                  print(f'G0_TCP_FAIL {ip}:{port} {e}')
              finally:
                  s.close()
          probe('\(feHost)', 18080)
          probe('\(g0Prefix).1.1', 18080)
          PY
          echo '=== G0 TRANSPORT END ==='
          """
    )
    print(out)
    printMarkers(from: out)
    print("udp_hits=\(udp.hitCount) tcp_hits=\(tcp.hitCount)")

    await fe.stop()
    stopInterface(ifFront)
    udp.stop()
    tcp.stop()
    withExtendedLifetime(frontend.network) { _ in }
    print("dnsudp done")
}

func runRouter(holdSeconds: Int) async throws {
    print("== G0 Router Cross-Net spike ==")
    let assets = assetsDir()

    let frontend = try createSharedNetwork(
        name: "frontend",
        subnet: feNet,
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    let backend = try createSharedNetwork(
        name: "backend",
        subnet: beNet,
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    printNetworkReport(frontend)
    printNetworkReport(backend)
    let ifFront = try startInterface(on: frontend)
    let ifBack = try startInterface(on: backend)

    guard let macFE = VZMACAddress(string: "52:54:00:83:01:10"),
          let macBE = VZMACAddress(string: "52:54:00:83:02:10"),
          let macR0 = VZMACAddress(string: "52:54:00:83:01:02"),
          let macR1 = VZMACAddress(string: "52:54:00:83:02:02")
    else { throw SpikeError.message("bad MAC") }

    let fe = try GuestRuntime(spec: GuestSpec(
        name: "frontend",
        diskURL: assets.appendingPathComponent("frontend.raw"),
        cidataURL: assets.appendingPathComponent("cidata-fe.iso"),
        nvramURL: assets.appendingPathComponent("frontend-nvram.bin"),
        serialLogURL: assets.appendingPathComponent("serial-frontend.log"),
        mac: macFE,
        network: frontend.network
    ))
    let be = try GuestRuntime(spec: GuestSpec(
        name: "backend",
        diskURL: assets.appendingPathComponent("backend.raw"),
        cidataURL: assets.appendingPathComponent("cidata-be.iso"),
        nvramURL: assets.appendingPathComponent("backend-nvram.bin"),
        serialLogURL: assets.appendingPathComponent("serial-backend.log"),
        mac: macBE,
        network: backend.network
    ))
    let router = try GuestRuntime(spec: GuestSpec(
        name: "router",
        diskURL: assets.appendingPathComponent("router.raw"),
        cidataURL: assets.appendingPathComponent("cidata-router.iso"),
        nvramURL: assets.appendingPathComponent("router-nvram.bin"),
        serialLogURL: assets.appendingPathComponent("serial-router.log"),
        mac: macR0,
        network: frontend.network,
        extraNICs: [(backend.network, macR1)]
    ))

    try await fe.start()
    try await be.start()
    try await router.start()

    let deadline = Date().addingTimeInterval(TimeInterval(holdSeconds))
    var feUp = false, beUp = false, rUp = false
    while Date() < deadline {
        if !feUp, hostPing(feGuest) { print("HOST_PING_OK \(feGuest)"); feUp = true }
        if !beUp, hostPing(beGuest) { print("HOST_PING_OK \(beGuest)"); beUp = true }
        if !rUp, hostPing(feRouter) { print("HOST_PING_OK \(feRouter)"); rUp = true }
        if feUp, beUp, rUp { break }
        try await Task.sleep(for: .seconds(3))
    }
    guard feUp, beUp, rUp else {
        throw SpikeError.message("not all IPs up fe=\(feUp) be=\(beUp) router=\(rUp)")
    }

    print("waiting for sshd on guests…")
    try await waitSSH(feGuest)
    try await waitSSH(beGuest)
    try await waitSSH(feRouter)

    // Ensure forwarding on router
    let routerReady = try sshProbe(
        host: feRouter,
        script: """
          sudo sysctl -w net.ipv4.ip_forward=1
          echo 1 | sudo tee /proc/sys/net/ipv4/ip_forward
          sudo iptables -P FORWARD ACCEPT || true
          ip -4 addr; ip route
          echo G0_ROUTER_READY
          """
    )
    print(routerReady)

    print("== Cross-net from frontend ==")
    let feOut = try sshProbe(
        host: feGuest,
        script: """
          set -x
          # ensure route via router
          sudo ip route replace \(g0Prefix).2.0/24 via \(feRouter) || true
          ip route
          if ping -c 3 -W 2 \(beGuest); then echo G0_XNET_OK \(beGuest); else echo G0_XNET_FAIL \(beGuest); fi
          if ping -c 2 -W 2 \(beRouter); then echo G0_XNET_OK \(beRouter); else echo G0_XNET_FAIL \(beRouter); fi
          if ping -c 2 -W 2 \(beHost); then echo G0_XNET_OK \(beHost); else echo G0_XNET_FAIL \(beHost); fi
          """
    )
    print(feOut)
    printMarkers(from: feOut)

    print("== Cross-net from backend ==")
    let beOut = try sshProbe(
        host: beGuest,
        script: """
          set -x
          sudo ip route replace \(g0Prefix).1.0/24 via \(beRouter) || true
          ip route
          if ping -c 3 -W 2 \(feGuest); then echo G0_XNET_OK \(feGuest); else echo G0_XNET_FAIL \(feGuest); fi
          """
    )
    print(beOut)
    printMarkers(from: beOut)

    await fe.stop()
    await be.stop()
    await router.stop()
    stopInterface(ifFront)
    stopInterface(ifBack)
    withExtendedLifetime(frontend.network) { _ in }
    withExtendedLifetime(backend.network) { _ in }
    print("router spike done")
}

// Re-open GuestSpec with extraNICs — edit the struct in GuestPhase instead via patch below.

final class UDPEchoServer: @unchecked Sendable {
    private let fd: Int32
    private let queue = DispatchQueue(label: "g0.udp")
    private var source: DispatchSourceRead?
    private(set) var hitCount = 0
    private let lock = NSLock()

    init(bindAddress: String, port: UInt16) throws {
        let sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard sock >= 0 else { throw SpikeError.errno("udp socket", errno) }
        var yes: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, bindAddress, &addr.sin_addr) == 1 else {
            close(sock); throw SpikeError.message("bad udp bind \(bindAddress)")
        }
        let br = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard br == 0 else {
            let e = errno; close(sock)
            throw SpikeError.errno("udp bind \(bindAddress):\(port)", e)
        }
        fd = sock
        let src = DispatchSource.makeReadSource(fileDescriptor: sock, queue: queue)
        src.setEventHandler { [weak self] in
            var buf = [UInt8](repeating: 0, count: 512)
            var peer = sockaddr_in()
            var peerLen = socklen_t(MemoryLayout<sockaddr_in>.size)
            let n = withUnsafeMutablePointer(to: &peer) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    recvfrom(sock, &buf, buf.count, 0, $0, &peerLen)
                }
            }
            guard n > 0, let self else { return }
            withUnsafePointer(to: &peer) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    _ = sendto(sock, buf, n, 0, $0, peerLen)
                }
            }
            self.lock.lock(); self.hitCount += 1; self.lock.unlock()
        }
        src.resume()
        source = src
    }

    func stop() {
        source?.cancel(); source = nil; close(fd)
    }
}

final class TCPEchoServer: @unchecked Sendable {
    private let fd: Int32
    private let queue = DispatchQueue(label: "g0.tcp")
    private var source: DispatchSourceRead?
    private(set) var hitCount = 0
    private let lock = NSLock()

    init(bindAddress: String, port: UInt16) throws {
        let sock = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard sock >= 0 else { throw SpikeError.errno("tcp socket", errno) }
        var yes: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, bindAddress, &addr.sin_addr) == 1 else {
            close(sock); throw SpikeError.message("bad tcp bind \(bindAddress)")
        }
        let br = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard br == 0 else {
            let e = errno; close(sock)
            throw SpikeError.errno("tcp bind \(bindAddress):\(port)", e)
        }
        guard listen(sock, 16) == 0 else {
            let e = errno; close(sock)
            throw SpikeError.errno("tcp listen", e)
        }
        fd = sock
        let src = DispatchSource.makeReadSource(fileDescriptor: sock, queue: queue)
        src.setEventHandler { [weak self] in
            let cfd = accept(sock, nil, nil)
            guard cfd >= 0, let self else { return }
            var buf = [UInt8](repeating: 0, count: 256)
            let n = read(cfd, &buf, buf.count)
            if n > 0 { _ = write(cfd, buf, n) }
            close(cfd)
            self.lock.lock(); self.hitCount += 1; self.lock.unlock()
        }
        src.resume()
        source = src
    }

    func stop() {
        source?.cancel(); source = nil; close(fd)
    }
}
