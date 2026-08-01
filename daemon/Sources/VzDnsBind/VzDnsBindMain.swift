import Darwin
import Dispatch
import Foundation
import VzDaemonKit

@main
enum VzDnsBindMain {
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
                  help

                UDP: binds SOCK_DGRAM on privileged ports and returns the FD via SCM_RIGHTS.
                TCP: binds+listens SOCK_STREAM, then streams accepted client FDs via SCM_RIGHTS
                on the same UDS connection (macOS cannot reliably accept on handed-off listeners).
                No DNS or proxy logic.
                """
            )
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

            var buffer = [UInt8](repeating: 0, count: 4096)
            let count = Darwin.recv(client, &buffer, buffer.count, 0)
            guard count > 0 else {
                try sendFailure(on: client, "empty request")
                return
            }
            let request = try DnsBind.parseRequest(Data(buffer.prefix(Int(count))))
            let proto = request.proto.lowercased()
            let bound = try bindSocket(
                address: request.address,
                port: request.port,
                proto: proto
            )

            if proto == DnsBind.protoTCP {
                // Keep listening FD in-helper; stream accepted clients over this UDS.
                defer { Darwin.close(bound) }
                let listening = try JSONEncoder().encode(
                    DnsBind.BindResponse(ok: true, event: "listening")
                )
                var framed = listening
                framed.append(0x0A)
                try UnixFDPassing.send(payload: framed, fileDescriptor: nil, on: client)

                while true {
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

    private static func bindSocket(address: String, port: UInt16, proto: String) throws -> Int32 {
        let isTCP = proto == DnsBind.protoTCP
        if isTCP {
            try ensureHostServiceAlias(address)
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

    /// Guest TCP cannot reach vmnet `.0`; ingress binds `.1`. Ensure that alias exists.
    private static func ensureHostServiceAlias(_ address: String) throws {
        if address == "127.0.0.1" || address == "0.0.0.0" { return }
        var target = in_addr()
        guard inet_pton(AF_INET, address, &target) == 1 else { return }
        let targetHost = UInt32(bigEndian: target.s_addr)

        var ifap: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&ifap) == 0, let first = ifap else { return }
        defer { freeifaddrs(ifap) }

        var interfaceName: String?
        var cursor: UnsafeMutablePointer<ifaddrs>? = first
        while let current = cursor {
            defer { cursor = current.pointee.ifa_next }
            guard let raw = current.pointee.ifa_addr, raw.pointee.sa_family == sa_family_t(AF_INET)
            else {
                continue
            }
            let sin = UnsafeRawPointer(raw).assumingMemoryBound(to: sockaddr_in.self).pointee
            let ip = UInt32(bigEndian: sin.sin_addr.s_addr)
            if ip == targetHost {
                return // already assigned
            }
            // Prefer the bridge that already owns the matching `.0` network address.
            if (ip & 0xffff_ff00) == (targetHost & 0xffff_ff00), (ip & 0xff) == 0 {
                interfaceName = String(cString: current.pointee.ifa_name)
            }
        }
        guard let interfaceName else {
            throw ServeError.usage("no vmnet bridge found for host-service alias \(address)")
        }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/sbin/ifconfig")
        process.arguments = [interfaceName, "alias", address, "netmask", "255.255.255.0"]
        let errPipe = Pipe()
        process.standardError = errPipe
        process.standardOutput = Pipe()
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            let err = String(data: errPipe.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8)
                ?? ""
            throw ServeError.usage(
                "ifconfig alias \(address) on \(interfaceName) failed: \(err.trimmingCharacters(in: .whitespacesAndNewlines))"
            )
        }
    }
}
