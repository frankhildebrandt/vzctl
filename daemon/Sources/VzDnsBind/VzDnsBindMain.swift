import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzDnsBindMain {
    private static let networkLock = NSLock()
    private static let aliasStatePath = "/var/run/vzctl/dns-bind-aliases.json"
    private static let pfTokenPath = "/var/run/vzctl/dns-bind-pf.token"
    private static let pfAnchor = "com.apple/vzctl"

    static func main() {
        let args = Array(CommandLine.arguments.dropFirst())
        switch args.first {
        case "version", nil:
            print("vz-dns-bind \(VzDaemonKit.version)")
            print("privilege: UDP/TCP bind helper for guest DNS :53 and ingress :80/:443 (ADR 0002)")
        case "help", "-h", "--help":
            print(
                """
                vz-dns-bind — privileged UDP/TCP bind helper

                Commands:
                  version
                  serve --allow-uid <uid> [--socket <path>]
                  cleanup
                  help

                UDP: binds SOCK_DGRAM on privileged ports and returns the FD via SCM_RIGHTS.
                TCP: binds+listens SOCK_STREAM, then streams accepted client FDs via SCM_RIGHTS
                on the same UDS connection (macOS cannot reliably accept on handed-off listeners).
                No DNS or proxy logic.
                """
            )
        case "cleanup":
            guard geteuid() == 0 else {
                fputs("error: cleanup requires root\n", stderr)
                exit(1)
            }
            do {
                try networkLock.withLock { try cleanupManagedNetworking() }
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        case "serve":
            do {
                let options = try ServeOptions.parse(Array(args.dropFirst()))
                try serve(options)
            } catch {
                fputs("error: \(error)\n", stderr)
                exit(1)
            }
        default:
            fputs("unknown: \(args.first!)\n", stderr)
            exit(VzExit.usage.rawValue)
        }
    }

    private struct ServeOptions {
        var allowUID: uid_t
        var socketPath: String

        static func parse(_ args: [String]) throws -> ServeOptions {
            var allowUID: uid_t?
            var socketPath = DnsBind.socketPath()
            var index = args.startIndex
            while index < args.endIndex {
                let argument = args[index]
                switch argument {
                case "--allow-uid":
                    index = args.index(after: index)
                    guard index < args.endIndex, let value = uid_t(args[index]) else {
                        throw ServeError.usage("--allow-uid requires a numeric uid")
                    }
                    allowUID = value
                case "--socket":
                    index = args.index(after: index)
                    guard index < args.endIndex else {
                        throw ServeError.usage("--socket requires a path")
                    }
                    socketPath = args[index]
                default:
                    throw ServeError.usage("unknown option: \(argument)")
                }
                index = args.index(after: index)
            }
            guard let allowUID else {
                throw ServeError.usage("serve requires --allow-uid <uid>")
            }
            return ServeOptions(allowUID: allowUID, socketPath: socketPath)
        }
    }

    private enum ServeError: Error, CustomStringConvertible {
        case usage(String)
        case system(String, Int32)

        var description: String {
            switch self {
            case let .usage(message):
                return message
            case let .system(operation, code):
                return "\(operation): \(String(cString: strerror(code)))"
            }
        }
    }

    private static func serve(_ options: ServeOptions) throws {
        try prepareSocketDirectory(options.socketPath)
        unlink(options.socketPath)

        let listener = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard listener >= 0 else { throw ServeError.system("socket", errno) }
        defer { Darwin.close(listener) }

        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        let pathBytes = options.socketPath.utf8CString
        guard pathBytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
            throw ServeError.usage("socket path too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in
            pathBytes.withUnsafeBytes { source in
                buffer.copyMemory(from: source)
            }
        }
        let bindResult = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bindResult == 0 else {
            throw ServeError.system("bind \(options.socketPath)", errno)
        }
        chmod(options.socketPath, 0o666)
        guard Darwin.listen(listener, 16) == 0 else {
            throw ServeError.system("listen", errno)
        }

        print("listening: \(options.socketPath) allow-uid=\(options.allowUID)")

        while true {
            let client = Darwin.accept(listener, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                throw ServeError.system("accept", errno)
            }
            // One request per connection; TCP accept streams hold the connection open.
            DispatchQueue.global(qos: .userInitiated).async {
                handle(client: client, allowUID: options.allowUID)
            }
        }
    }

    private static func prepareSocketDirectory(_ socketPath: String) throws {
        let directory = URL(fileURLWithPath: socketPath).deletingLastPathComponent().path
        if directory.isEmpty || directory == "/" { return }
        do {
            try FileManager.default.createDirectory(
                atPath: directory,
                withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o755]
            )
        } catch {
            throw ServeError.usage("cannot create socket directory \(directory): \(error)")
        }
    }

    private static func handle(client: Int32, allowUID: uid_t) {
        defer { Darwin.close(client) }
        do {
            var peerUID: uid_t = 0
            var peerGID: gid_t = 0
            guard getpeereid(client, &peerUID, &peerGID) == 0 else {
                try sendFailure(on: client, "getpeereid failed")
                return
            }
            guard peerUID == allowUID else {
                try sendFailure(on: client, "peer uid \(peerUID) not allowed")
                return
            }

            let operation = try DnsBind.parseOperation(readRequest(on: client))
            guard case let .bind(request) = operation else {
                try networkLock.withLock {
                    switch operation {
                    case let .alias(request):
                        if request.op == DnsBind.opAliasEnsure {
                            try ensureHostServiceAlias(cidr: request.cidr)
                            try updateAliasState(cidr: request.cidr, present: true)
                        } else {
                            try removeHostServiceAlias(cidr: request.cidr)
                            try updateAliasState(cidr: request.cidr, present: false)
                        }
                    case let .firewall(request):
                        try reconcileFirewall(request.bindings)
                    case .cleanup:
                        try cleanupManagedNetworking()
                    case .bind:
                        break
                    }
                }
                let response = try JSONEncoder().encode(DnsBind.BindResponse(ok: true))
                var framed = response
                framed.append(0x0A)
                try UnixFDPassing.send(payload: framed, fileDescriptor: nil, on: client)
                return
            }
            let proto = request.proto.lowercased()
            let bound = try bindSocket(address: request.address, port: request.port, proto: proto)

            if proto == DnsBind.protoTCP {
                // Keep listening FD in-helper; stream accepted clients over this UDS.
                defer { Darwin.close(bound) }
                let listening = try JSONEncoder().encode(
                    DnsBind.BindResponse(ok: true, event: "listening")
                )
                var framed = listening
                framed.append(0x0A)
                try UnixFDPassing.send(payload: framed, fileDescriptor: nil, on: client)

                // Poll listen FD + UDS so peer hangup releases the port promptly
                // (plain accept() alone leaves EADDRINUSE orphans on rebind).
                while true {
                    var pollFds = [
                        pollfd(fd: bound, events: Int16(POLLIN), revents: 0),
                        pollfd(fd: client, events: Int16(POLLIN | POLLHUP), revents: 0),
                    ]
                    let primed = poll(&pollFds, nfds_t(pollFds.count), -1)
                    if primed < 0 {
                        if errno == EINTR { continue }
                        return
                    }
                    let clientEvents = Int32(pollFds[1].revents)
                    if clientEvents & (POLLHUP | POLLERR | POLLNVAL) != 0 {
                        return
                    }
                    if clientEvents & POLLIN != 0 {
                        var byte: UInt8 = 0
                        let n = recv(client, &byte, 1, MSG_DONTWAIT)
                        if n <= 0 { return }
                    }
                    if Int32(pollFds[0].revents) & (POLLIN | POLLERR | POLLNVAL) == 0 {
                        continue
                    }
                    if Int32(pollFds[0].revents) & (POLLERR | POLLNVAL) != 0 {
                        return
                    }
                    let accepted = Darwin.accept(bound, nil, nil)
                    if accepted < 0 {
                        if errno == EINTR { continue }
                        return
                    }
                    var noSigPipe: Int32 = 1
                    setsockopt(
                        accepted,
                        SOL_SOCKET,
                        SO_NOSIGPIPE,
                        &noSigPipe,
                        socklen_t(MemoryLayout<Int32>.size)
                    )
                    let payload = try JSONEncoder().encode(
                        DnsBind.BindResponse(ok: true, event: "accept")
                    )
                    var acceptFrame = payload
                    acceptFrame.append(0x0A)
                    do {
                        try UnixFDPassing.send(
                            payload: acceptFrame,
                            fileDescriptor: accepted,
                            on: client
                        )
                    } catch {
                        Darwin.close(accepted)
                        return
                    }
                    Darwin.close(accepted)
                }
            } else {
                defer { Darwin.close(bound) }
                let response = try JSONEncoder().encode(DnsBind.BindResponse(ok: true))
                var framed = response
                framed.append(0x0A)
                try UnixFDPassing.send(payload: framed, fileDescriptor: bound, on: client)
            }
        } catch {
            try? sendFailure(on: client, "\(error)")
        }
    }

    private static func sendFailure(on client: Int32, _ message: String) throws {
        let response = try JSONEncoder().encode(DnsBind.BindResponse(ok: false, error: message))
        var framed = response
        framed.append(0x0A)
        try UnixFDPassing.send(payload: framed, fileDescriptor: nil, on: client)
    }

    private static func readRequest(on client: Int32) throws -> Data {
        var request = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while request.count <= 65_536 {
            let count = Darwin.recv(client, &buffer, buffer.count, 0)
            guard count > 0 else {
                throw ServeError.usage(request.isEmpty ? "empty request" : "unterminated request")
            }
            request.append(contentsOf: buffer.prefix(Int(count)))
            if let newline = request.firstIndex(of: 0x0A) {
                return Data(request[..<newline])
            }
        }
        throw ServeError.usage("request exceeds 64 KiB")
    }

    private static func bindSocket(address: String, port: UInt16, proto: String) throws -> Int32 {
        let isTCP = proto == DnsBind.protoTCP
        if isTCP {
            try ensureHostServiceAlias(address: address)
        }
        let descriptor = Darwin.socket(
            AF_INET,
            isTCP ? SOCK_STREAM : SOCK_DGRAM,
            isTCP ? IPPROTO_TCP : IPPROTO_UDP
        )
        guard descriptor >= 0 else { throw ServeError.system("socket", errno) }
        // SO_REUSEADDR is required to bind UDP :53 alongside mDNSResponder's *:53.
        // Guest answers must not rely on winning that race — ingress *.svc names use
        // split horizon (host → 127.0.0.1, guest → host-service `.1`).
        var reuse: Int32 = 1
        setsockopt(
            descriptor,
            SOL_SOCKET,
            SO_REUSEADDR,
            &reuse,
            socklen_t(MemoryLayout<Int32>.size)
        )
        var addr = sockaddr_in()
        addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        addr.sin_family = sa_family_t(AF_INET)
        addr.sin_port = port.bigEndian
        guard inet_pton(AF_INET, address, &addr.sin_addr) == 1 else {
            Darwin.close(descriptor)
            throw DnsBind.ValidationError.invalidAddress(address)
        }
        let result = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard result == 0 else {
            let code = errno
            Darwin.close(descriptor)
            throw ServeError.system("bind \(address):\(port)", code)
        }
        if isTCP {
            guard Darwin.listen(descriptor, 32) == 0 else {
                let code = errno
                Darwin.close(descriptor)
                throw ServeError.system("listen \(address):\(port)", code)
            }
        }
        return descriptor
    }

    /// Backward-compatible safety net for TCP callers. The normal lifecycle uses
    /// explicit alias.ensure before any ingress listener is opened.
    private static func ensureHostServiceAlias(address: String) throws {
        if address == "127.0.0.1" || address == "0.0.0.0" { return }
        guard let cidr = cidrForHostServiceAddress(address) else {
            throw ServeError.usage("no vmnet bridge found for host-service alias \(address)")
        }
        try ensureHostServiceAlias(cidr: cidr)
        try updateAliasState(cidr: cidr, present: true)
    }

    private static func ensureHostServiceAlias(cidr: String) throws {
        let parsed = try IPv4CIDR(cidr)
        let gateway = IPv4CIDR.gateway(for: cidr)
        let address = IPv4CIDR.hostService(for: cidr)
        let mask = ipv4String(parsed.mask)
        guard !gateway.isEmpty, !address.isEmpty, !mask.isEmpty else {
            throw ServeError.usage("invalid host-service CIDR \(cidr)")
        }

        let interfaces = ipv4Interfaces()
        if let alias = interfaces.first(where: { $0.address == address }) {
            guard interfaces.contains(where: {
                $0.name == alias.name && $0.address == gateway && $0.netmask == mask
            }) else {
                throw ServeError.usage(
                    "refusing to reuse alias \(address): matching vmnet bridge \(gateway)/\(parsed.prefix) missing"
                )
            }
            return
        }
        guard let bridge = interfaces.first(where: {
            $0.address == gateway && $0.netmask == mask
        }) else {
            throw ServeError.usage("no vmnet bridge \(gateway)/\(parsed.prefix) for alias \(address)")
        }
        try runIfconfig([bridge.name, "alias", address, "netmask", mask])
    }

    private static func removeHostServiceAlias(cidr: String) throws {
        let gateway = IPv4CIDR.gateway(for: cidr)
        let address = IPv4CIDR.hostService(for: cidr)
        let interfaces = ipv4Interfaces()
        guard let alias = interfaces.first(where: { $0.address == address }) else { return }
        guard interfaces.contains(where: { $0.name == alias.name && $0.address == gateway }) else {
            throw ServeError.usage("refusing to remove alias \(address): matching vmnet gateway missing")
        }
        try runIfconfig([alias.name, "-alias", address])
    }

    private struct IPv4Interface {
        let name: String
        let address: String
        let netmask: String
    }

    private static func ipv4Interfaces() -> [IPv4Interface] {
        var result: [IPv4Interface] = []
        var ifap: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifap) == 0, let first = ifap else { return [] }
        defer { freeifaddrs(ifap) }

        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let current = cursor {
            defer { cursor = current.pointee.ifa_next }
            guard let raw = current.pointee.ifa_addr,
                  raw.pointee.sa_family == sa_family_t(AF_INET)
            else { continue }
            let address = UnsafeRawPointer(raw).assumingMemoryBound(to: sockaddr_in.self).pointee
            let mask = current.pointee.ifa_netmask.map {
                UnsafeRawPointer($0).assumingMemoryBound(to: sockaddr_in.self).pointee
            }
            result.append(IPv4Interface(
                name: String(cString: current.pointee.ifa_name),
                address: ipv4String(UInt32(bigEndian: address.sin_addr.s_addr)),
                netmask: mask.map { ipv4String(UInt32(bigEndian: $0.sin_addr.s_addr)) } ?? ""
            ))
        }
        return result
    }

    private static func cidrForHostServiceAddress(_ address: String) -> String? {
        var target = in_addr()
        guard inet_pton(AF_INET, address, &target) == 1 else { return nil }
        let targetHost = UInt32(bigEndian: target.s_addr)
        guard targetHost > 0 else { return nil }
        let gateway = ipv4String(targetHost - 1)
        guard let bridge = ipv4Interfaces().first(where: { $0.address == gateway }),
              let prefix = prefixLength(netmask: bridge.netmask)
        else { return nil }
        return "\(gateway)/\(prefix)"
    }

    private static func prefixLength(netmask: String) -> Int? {
        var address = in_addr()
        guard inet_pton(AF_INET, netmask, &address) == 1 else { return nil }
        let value = UInt32(bigEndian: address.s_addr)
        let prefix = value.nonzeroBitCount
        let expected = prefix == 0 ? UInt32(0) : UInt32.max << UInt32(32 - prefix)
        return value == expected ? prefix : nil
    }

    private static func ipv4String(_ value: UInt32) -> String {
        var address = in_addr(s_addr: value.bigEndian)
        var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
        guard inet_ntop(AF_INET, &address, &buffer, socklen_t(INET_ADDRSTRLEN)) != nil else {
            return ""
        }
        return String(decoding: buffer.map { UInt8(bitPattern: $0) }.prefix { $0 != 0 }, as: UTF8.self)
    }

    private static func runIfconfig(_ arguments: [String]) throws {
        _ = try runProcess(path: "/sbin/ifconfig", arguments: arguments)
    }

    private static func reconcileFirewall(_ bindings: [DnsBind.FirewallBinding]) throws {
        try ensurePFEnabled()
        var interfaces: [String: String] = [:]
        var availableBindings: [DnsBind.FirewallBinding] = []
        for binding in bindings.sorted(by: { $0.cidr < $1.cidr }) {
            let parsed = try IPv4CIDR(binding.cidr)
            let gateway = IPv4CIDR.gateway(for: binding.cidr)
            let hostService = IPv4CIDR.hostService(for: binding.cidr)
            let mask = ipv4String(parsed.mask)
            guard let interface = ipv4Interfaces().first(where: {
                $0.address == gateway && $0.netmask == mask
            }) else { continue }
            guard !hostService.isEmpty else { continue }
            interfaces[binding.cidr] = interface.name
            availableBindings.append(binding)
        }
        let body = try DnsBind.firewallRules(
            bindings: availableBindings,
            interfaceByCIDR: interfaces
        )
        _ = try runProcess(
            path: "/sbin/pfctl",
            arguments: ["-a", pfAnchor, "-f", "-"],
            input: Data(body.utf8)
        )
    }

    private static func ensurePFEnabled() throws {
        if FileManager.default.fileExists(atPath: pfTokenPath) { return }
        let output = try runProcess(path: "/sbin/pfctl", arguments: ["-E"])
        let tokens = output.split(whereSeparator: { !$0.isNumber })
        guard let token = tokens.last, !token.isEmpty else {
            throw ServeError.usage("pfctl -E did not return an enable token")
        }
        try Data((String(token) + "\n").utf8)
            .write(to: URL(fileURLWithPath: pfTokenPath), options: .atomic)
        _ = chmod(pfTokenPath, 0o600)
    }

    private static func cleanupManagedNetworking() throws {
        for cidr in loadAliasState().sorted().reversed() {
            try removeHostServiceAlias(cidr: cidr)
        }
        try saveAliasState([])
        if FileManager.default.fileExists(atPath: pfTokenPath) {
            // The anchor owns filter and translation (rdr) rules.
            _ = try runProcess(path: "/sbin/pfctl", arguments: ["-a", pfAnchor, "-F", "all"])
            let token = (try? String(contentsOfFile: pfTokenPath, encoding: .utf8))?
                .trimmingCharacters(in: .whitespacesAndNewlines)
            if let token, !token.isEmpty {
                _ = try runProcess(path: "/sbin/pfctl", arguments: ["-X", token])
            }
            try? FileManager.default.removeItem(atPath: pfTokenPath)
        }
    }

    private static func updateAliasState(cidr: String, present: Bool) throws {
        var state = loadAliasState()
        if present { state.insert(cidr) } else { state.remove(cidr) }
        try saveAliasState(state)
    }

    private static func loadAliasState() -> Set<String> {
        guard let data = try? Data(contentsOf: URL(fileURLWithPath: aliasStatePath)),
              let values = try? JSONDecoder().decode([String].self, from: data)
        else { return [] }
        return Set(values)
    }

    private static func saveAliasState(_ values: Set<String>) throws {
        let data = try JSONEncoder().encode(values.sorted())
        try data.write(to: URL(fileURLWithPath: aliasStatePath), options: .atomic)
        _ = chmod(aliasStatePath, 0o600)
    }

    @discardableResult
    private static func runProcess(
        path: String,
        arguments: [String],
        input: Data? = nil
    ) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: path)
        process.arguments = arguments
        let output = Pipe()
        let errors = Pipe()
        process.standardOutput = output
        process.standardError = errors
        if let input {
            let stdin = Pipe()
            process.standardInput = stdin
            try process.run()
            stdin.fileHandleForWriting.write(input)
            try? stdin.fileHandleForWriting.close()
        } else {
            try process.run()
        }
        process.waitUntilExit()
        let stdout = output.fileHandleForReading.readDataToEndOfFile()
        let stderr = errors.fileHandleForReading.readDataToEndOfFile()
        guard process.terminationStatus == 0 else {
            let message = String(data: stderr, encoding: .utf8) ?? ""
            throw ServeError.usage(
                "\((path as NSString).lastPathComponent) \(arguments.joined(separator: " ")) failed: \(message.trimmingCharacters(in: .whitespacesAndNewlines))"
            )
        }
        return (String(data: stdout, encoding: .utf8) ?? "")
            + (String(data: stderr, encoding: .utf8) ?? "")
    }
}
