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
            print("privilege: UDP bind helper for guest DNS :53 (ADR 0002)")
        case "help", "-h", "--help":
            print(
                """
                vz-dns-bind — privileged UDP bind helper

                Commands:
                  version
                  serve --allow-uid <uid> [--socket <path>]
                  help

                Binds AF_INET SOCK_DGRAM sockets on privileged ports and
                returns them over UDS via SCM_RIGHTS. No DNS logic.
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
            throw ServeError.usage("socket path too long: \(options.socketPath)")
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
        guard bindResult == 0 else { throw ServeError.system("bind", errno) }

        // User supervisor must connect; root still accepts. Auth via getpeereid.
        chown(options.socketPath, options.allowUID, options.allowUID)
        chmod(options.socketPath, 0o600)

        guard Darwin.listen(listener, 16) == 0 else {
            throw ServeError.system("listen", errno)
        }

        print("listening: \(options.socketPath) allow-uid=\(options.allowUID)")
        fflush(stdout)

        signal(SIGPIPE, SIG_IGN)
        while true {
            let client = Darwin.accept(listener, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                throw ServeError.system("accept", errno)
            }
            DispatchQueue.global().async {
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
            let bound = try bindUDP(address: request.address, port: request.port)
            defer { Darwin.close(bound) }
            let response = try JSONEncoder().encode(DnsBind.BindResponse(ok: true))
            var framed = response
            framed.append(0x0A)
            try UnixFDPassing.send(payload: framed, fileDescriptor: bound, on: client)
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

    private static func bindUDP(address: String, port: UInt16) throws -> Int32 {
        let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard descriptor >= 0 else { throw ServeError.system("socket", errno) }
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
        return descriptor
    }
}
