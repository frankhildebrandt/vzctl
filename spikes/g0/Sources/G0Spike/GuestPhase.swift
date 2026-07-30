import Darwin
import Dispatch
import Foundation
import Virtualization
import vmnet

// MARK: - Phase C: Linux guests on dual vmnet

struct GuestSpec {
    let name: String
    let diskURL: URL
    let cidataURL: URL
    let nvramURL: URL
    let serialLogURL: URL
    let mac: VZMACAddress
    let network: vmnet_network_ref
    /// Additional NICs (network + MAC), e.g. router second interface.
    let extraNICs: [(vmnet_network_ref, VZMACAddress)]

    init(
        name: String,
        diskURL: URL,
        cidataURL: URL,
        nvramURL: URL,
        serialLogURL: URL,
        mac: VZMACAddress,
        network: vmnet_network_ref,
        extraNICs: [(vmnet_network_ref, VZMACAddress)] = []
    ) {
        self.name = name
        self.diskURL = diskURL
        self.cidataURL = cidataURL
        self.nvramURL = nvramURL
        self.serialLogURL = serialLogURL
        self.mac = mac
        self.network = network
        self.extraNICs = extraNICs
    }
}

final class GuestRuntime: NSObject, VZVirtualMachineDelegate {
    let spec: GuestSpec
    let queue: DispatchQueue
    let vm: VZVirtualMachine

    init(spec: GuestSpec) throws {
        self.spec = spec
        self.queue = DispatchQueue(label: "g0.guest.\(spec.name)")
        let config = try Self.makeConfiguration(spec: spec)
        self.vm = VZVirtualMachine(configuration: config, queue: queue)
        super.init()
        self.vm.delegate = self
    }

    static func makeConfiguration(spec: GuestSpec) throws -> VZVirtualMachineConfiguration {
        let cfg = VZVirtualMachineConfiguration()
        cfg.cpuCount = 2
        cfg.memorySize = 1 * 1024 * 1024 * 1024

        let platform = VZGenericPlatformConfiguration()
        cfg.platform = platform

        let efi = VZEFIBootLoader()
        if FileManager.default.fileExists(atPath: spec.nvramURL.path) {
            efi.variableStore = VZEFIVariableStore(url: spec.nvramURL)
        } else {
            efi.variableStore = try VZEFIVariableStore(creatingVariableStoreAt: spec.nvramURL)
        }
        cfg.bootLoader = efi

        let rootAtt = try VZDiskImageStorageDeviceAttachment(
            url: spec.diskURL,
            readOnly: false,
            cachingMode: .cached,
            synchronizationMode: .fsync
        )
        let isoAtt = try VZDiskImageStorageDeviceAttachment(
            url: spec.cidataURL,
            readOnly: true
        )
        cfg.storageDevices = [
            VZVirtioBlockDeviceConfiguration(attachment: rootAtt),
            VZVirtioBlockDeviceConfiguration(attachment: isoAtt),
        ]

        let nic = VZVirtioNetworkDeviceConfiguration()
        nic.attachment = VZVmnetNetworkDeviceAttachment(network: spec.network)
        nic.macAddress = spec.mac
        var nics: [VZVirtioNetworkDeviceConfiguration] = [nic]
        for (net, mac) in spec.extraNICs {
            let extra = VZVirtioNetworkDeviceConfiguration()
            extra.attachment = VZVmnetNetworkDeviceAttachment(network: net)
            extra.macAddress = mac
            nics.append(extra)
        }
        cfg.networkDevices = nics

        // Serial → append-only log file (cloud-init markers).
        // Reading handle must be a real pipe — FileHandle.nullDevice is rejected.
        var stdinPipe: [Int32] = [0, 0]
        guard pipe(&stdinPipe) == 0 else {
            throw SpikeError.errno("serial pipe", errno)
        }
        let guestStdinRead = FileHandle(fileDescriptor: stdinPipe[0], closeOnDealloc: true)
        // Keep write end alive so the read side doesn't see EOF immediately.
        _ = FileHandle(fileDescriptor: stdinPipe[1], closeOnDealloc: false)

        FileManager.default.createFile(atPath: spec.serialLogURL.path, contents: nil)
        let writeFH = try FileHandle(forWritingTo: spec.serialLogURL)
        let console = VZVirtioConsoleDeviceConfiguration()
        let port = VZVirtioConsolePortConfiguration()
        port.isConsole = true
        port.attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: guestStdinRead,
            fileHandleForWriting: writeFH
        )
        console.ports[0] = port
        cfg.consoleDevices = [console]
        cfg.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]

        try cfg.validate()
        return cfg
    }

    func start() async throws {
        try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Void, Error>) in
            queue.async { [vm] in
                vm.start { result in
                    switch result {
                    case .success:
                        cont.resume()
                    case .failure(let error):
                        cont.resume(throwing: error)
                    }
                }
            }
        }
        print("guest \(spec.name) started → serial \(spec.serialLogURL.path)")
    }

    func stop() async {
        await withCheckedContinuation { (cont: CheckedContinuation<Void, Never>) in
            queue.async { [vm] in
                guard vm.canStop else {
                    cont.resume()
                    return
                }
                vm.stop { _ in cont.resume() }
            }
        }
    }

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        print("guest \(spec.name) stopped")
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        fputs("guest \(spec.name) error: \(error)\n", stderr)
    }
}

func runGuests(holdSeconds: Int) async throws {
    print("== G0 Phase C: Linux guests on dual shared nets ==")
    let assets = assetsDir()
    print("assets: \(assets.path)")

    let frontend = try createSharedNetwork(
        name: "frontend",
        subnet: "10.82.1.0",
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    let backend = try createSharedNetwork(
        name: "backend",
        subnet: "10.82.2.0",
        mask: "255.255.255.0",
        disableDHCP: true,
        disableDNSProxy: true
    )
    printNetworkReport(frontend)
    printNetworkReport(backend)

    // Bring up host bridges so .0 is bindable before guests ARP.
    let ifFront = try startInterface(on: frontend)
    let ifBack = try startInterface(on: backend)
    printHostInterfaces()

    var httpServer: HostProbeServer?
    do {
        httpServer = try HostProbeServer(bindAddress: "10.82.1.0", port: 18080)
        print("host probe http://10.82.1.0:18080/g0")
    } catch {
        print("warn: host HTTP probe not started: \(error) — ping-only mode")
    }

    guard let macFE = VZMACAddress(string: "52:54:00:77:01:10"),
          let macBE = VZMACAddress(string: "52:54:00:77:02:10")
    else {
        throw SpikeError.message("invalid MAC")
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
    let beSpec = GuestSpec(
        name: "backend",
        diskURL: assets.appendingPathComponent("backend.raw"),
        cidataURL: assets.appendingPathComponent("cidata-be.iso"),
        nvramURL: assets.appendingPathComponent("backend-nvram.bin"),
        serialLogURL: assets.appendingPathComponent("serial-backend.log"),
        mac: macBE,
        network: backend.network
    )

    for url in [feSpec.diskURL, beSpec.diskURL, feSpec.cidataURL, beSpec.cidataURL] {
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw SpikeError.message("missing asset: \(url.path) — run scripts/prepare-assets.sh")
        }
    }

    // Fresh serial logs
    try? FileManager.default.removeItem(at: feSpec.serialLogURL)
    try? FileManager.default.removeItem(at: beSpec.serialLogURL)

    let fe = try GuestRuntime(spec: feSpec)
    let be = try GuestRuntime(spec: beSpec)
    try await fe.start()
    try await be.start()

    print("waiting up to \(holdSeconds)s for guest IPs + SSH probes…")
    let deadline = Date().addingTimeInterval(TimeInterval(holdSeconds))
    var feIP = false
    var beIP = false
    while Date() < deadline {
        if !feIP, hostPing("10.82.1.10") {
            print("HOST_PING_OK 10.82.1.10")
            feIP = true
        }
        if !beIP, hostPing("10.82.2.10") {
            print("HOST_PING_OK 10.82.2.10")
            beIP = true
        }
        if feIP, beIP { break }
        try await Task.sleep(for: .seconds(3))
    }

    guard feIP else {
        print("frontend guest IP never came up")
        dumpTail(feSpec.serialLogURL, lines: 60)
        throw SpikeError.message("frontend IP timeout")
    }

    // SSH probes from guests (avoids hvc0/getty fighting cloud-init output)
    print("== SSH probes from frontend ==")
    let feProbe = try sshProbe(
        host: "10.82.1.10",
        script: """
          set -x
          echo '=== G0 REACHABILITY START ==='
          ip -4 addr; ip route
          for t in 10.82.1.0 10.82.1.1 10.82.2.0 10.82.2.1; do
            if ping -c 2 -W 2 "$t"; then echo "G0_PING_OK $t"; else echo "G0_PING_FAIL $t"; fi
          done
          for t in 10.82.1.0 10.82.1.1; do
            if curl -fsS --connect-timeout 2 "http://$t:18080/g0" -o /dev/null; then
              echo "G0_HTTP_OK $t"; else echo "G0_HTTP_FAIL $t"; fi
          done
          echo '=== G0 REACHABILITY END ==='
          """
    )
    print(feProbe)
    printMarkers(from: feProbe)

    if beIP {
        print("== SSH probes from backend ==")
        let beProbe = try sshProbe(
            host: "10.82.2.10",
            script: """
              set -x
              echo '=== G0 REACHABILITY START ==='
              ip -4 addr; ip route
              for t in 10.82.2.0 10.82.2.1 10.82.1.0 10.82.1.1 10.82.1.10; do
                if ping -c 2 -W 2 "$t"; then echo "G0_PING_OK $t"; else echo "G0_PING_FAIL $t"; fi
              done
              echo '=== G0 REACHABILITY END ==='
              """
        )
        print(beProbe)
        printMarkers(from: beProbe)
    } else {
        print("backend guest IP never came up — skip BE probes")
        dumpTail(beSpec.serialLogURL, lines: 40)
    }

    print("host probe hits: \(httpServer?.hitCount ?? -1)")
    await fe.stop()
    await be.stop()
    stopInterface(ifFront)
    stopInterface(ifBack)
    httpServer?.stop()
    withExtendedLifetime(frontend.network) { _ in }
    withExtendedLifetime(backend.network) { _ in }
    print("phase C done")
}

func waitSSH(_ host: String) async throws {
    for _ in 0..<60 {
        let proc = Process()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/nc")
        proc.arguments = ["-z", "-G", "2", host, "22"]
        proc.standardOutput = FileHandle.nullDevice
        proc.standardError = FileHandle.nullDevice
        try? proc.run()
        proc.waitUntilExit()
        if proc.terminationStatus == 0 {
            print("SSH_PORT_OPEN \(host)")
            return
        }
        try await Task.sleep(for: .seconds(2))
    }
    throw SpikeError.message("sshd not open on \(host)")
}

func hostPing(_ ip: String) -> Bool {
    let ping = Process()
    ping.executableURL = URL(fileURLWithPath: "/sbin/ping")
    ping.arguments = ["-c", "1", "-W", "1000", ip]
    ping.standardOutput = FileHandle.nullDevice
    ping.standardError = FileHandle.nullDevice
    try? ping.run()
    ping.waitUntilExit()
    return ping.terminationStatus == 0
}

func sshProbe(host: String, script: String) throws -> String {
    // Wait briefly for sshd after IP is up.
    var lastErr = SpikeError.message("ssh failed")
    for _ in 0..<30 {
        let proc = Process()
        let out = Pipe()
        let err = Pipe()
        proc.executableURL = URL(fileURLWithPath: "/usr/bin/ssh")
        proc.arguments = [
            "-o", "StrictHostKeyChecking=no",
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "ConnectTimeout=5",
            "-o", "PreferredAuthentications=password",
            "-o", "PubkeyAuthentication=no",
            "ubuntu@\(host)",
            "bash -s",
        ]
        proc.standardInput = Pipe()
        proc.standardOutput = out
        proc.standardError = err
        // Prefer sshpass when available.
        if let sshpass = [ "/opt/homebrew/bin/sshpass", "/usr/local/bin/sshpass" ]
            .first(where: { FileManager.default.isExecutableFile(atPath: $0) })
        {
            proc.executableURL = URL(fileURLWithPath: sshpass)
            proc.arguments = ["-p", "ubuntu", "/usr/bin/ssh"] + Array(proc.arguments!.dropFirst(0))
            // rebuild: sshpass -p ubuntu ssh <args...>
            proc.arguments = [
                "-p", "ubuntu",
                "/usr/bin/ssh",
                "-o", "StrictHostKeyChecking=no",
                "-o", "UserKnownHostsFile=/dev/null",
                "-o", "ConnectTimeout=5",
                "-o", "PreferredAuthentications=password",
                "-o", "PubkeyAuthentication=no",
                "ubuntu@\(host)",
                "bash -s",
            ]
        }
        try proc.run()
        if let stdin = proc.standardInput as? Pipe {
            stdin.fileHandleForWriting.write(script.data(using: .utf8)!)
            try? stdin.fileHandleForWriting.close()
        }
        proc.waitUntilExit()
        let stdout = String(data: out.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        let stderr = String(data: err.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
        if proc.terminationStatus == 0 {
            return stdout + stderr
        }
        lastErr = SpikeError.message("ssh \(host) exit=\(proc.terminationStatus): \(stderr)")
        Thread.sleep(forTimeInterval: 2)
    }
    throw lastErr
}

func printMarkers(from log: String) {
    for line in log.split(separator: "\n") {
        let s = String(line)
        if s.contains("G0_PING_") || s.contains("G0_HTTP_") || s.contains("G0 REACHABILITY") {
            print(s)
        }
    }
}

func dumpTail(_ url: URL, lines: Int = 40) {
    guard let data = try? Data(contentsOf: url),
          let text = String(data: data, encoding: .utf8)
    else {
        print("(no serial at \(url.path))")
        return
    }
    let all = text.split(separator: "\n")
    print("=== tail \(url.lastPathComponent) ===")
    for line in all.suffix(lines) {
        print(line)
    }
}

func assetsDir() -> URL {
    if let env = ProcessInfo.processInfo.environment["G0_ASSETS"] {
        return URL(fileURLWithPath: env)
    }
    let cwd = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
    let candidate = cwd.appendingPathComponent("assets")
    if FileManager.default.fileExists(atPath: candidate.path) {
        return candidate
    }
    return cwd
        .appendingPathComponent("spikes/g0/assets")
}

/// Minimal HTTP listener via BSD sockets (NWListener rejected bridge .0).
final class HostProbeServer: @unchecked Sendable {
    private let fd: Int32
    private let queue = DispatchQueue(label: "g0.http")
    private var source: DispatchSourceRead?
    private(set) var hitCount: Int = 0
    private let lock = NSLock()

    init(bindAddress: String, port: UInt16) throws {
        let sock = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP)
        guard sock >= 0 else { throw SpikeError.errno("http socket", errno) }
        var yes: Int32 = 1
        setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, &yes, socklen_t(MemoryLayout.size(ofValue: yes)))

        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, bindAddress, &addr.sin_addr) == 1 else {
            close(sock)
            throw SpikeError.message("bad http bind \(bindAddress)")
        }
        let br = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(sock, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard br == 0 else {
            let e = errno
            close(sock)
            throw SpikeError.errno("http bind \(bindAddress):\(port)", e)
        }
        guard listen(sock, 16) == 0 else {
            let e = errno
            close(sock)
            throw SpikeError.errno("http listen", e)
        }
        fd = sock

        let src = DispatchSource.makeReadSource(fileDescriptor: sock, queue: queue)
        src.setEventHandler { [weak self] in
            let cfd = accept(sock, nil, nil)
            guard cfd >= 0, let self else { return }
            var buf = [UInt8](repeating: 0, count: 1024)
            _ = read(cfd, &buf, buf.count)
            let body = "G0_OK\n"
            let resp =
                "HTTP/1.1 200 OK\r\nContent-Length: \(body.utf8.count)\r\nConnection: close\r\n\r\n\(body)"
            resp.withCString { ptr in
                _ = write(cfd, ptr, strlen(ptr))
            }
            close(cfd)
            self.lock.lock()
            self.hitCount += 1
            self.lock.unlock()
        }
        src.resume()
        source = src
    }

    func stop() {
        source?.cancel()
        source = nil
        close(fd)
    }
}
