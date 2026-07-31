import Darwin
import Foundation
import VzDaemonKit

enum HelperExecTTYBridge {
    private final class Session: @unchecked Sendable {
        let client: GuestAgentClient
        let socketPath: String
        var listener: Int32 = -1
        var clientFD: Int32 = -1

        init(client: GuestAgentClient, socketPath: String) {
            self.client = client
            self.socketPath = socketPath
        }
    }

    static func start(
        params: HelperAgentRequest.ExecTTYParams,
        runtime: VirtualMachineRuntime,
        token: String,
        stateDirectory: URL,
        vmID: String
    ) async throws -> JSONValue {
        let client = try await runtime.connectToGuestAgent(timeout: 5)
        let hello = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        guard hello.capabilities.contains("exec_tty") else {
            client.close()
            throw RouteApplyError.invalid(
                "guest agent missing exec_tty capability; rebake the base image"
            )
        }
        do {
            try client.upgradeTTYExec(
                argv: params.cmd,
                cwd: params.cwd,
                environment: params.env,
                cols: params.cols,
                rows: params.rows
            )
        } catch {
            client.close()
            throw error
        }

        let helpers = stateDirectory.appendingPathComponent("helpers", isDirectory: true)
        try FileManager.default.createDirectory(
            at: helpers,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        // Keep the path well under macOS AF_UNIX sun_path (~104 bytes). Embedding
        // StateFileName + a full UUID under Application Support exceeds the limit.
        let sessionID = String(
            UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(16)
        ).lowercased()
        let vmToken = String(StateFileName.component(vmID).suffix(8))
        let socketPath = helpers
            .appendingPathComponent("e-\(vmToken)-\(sessionID).sock")
            .path
        let session = Session(client: client, socketPath: socketPath)
        try bindListener(session)
        DispatchQueue.global().async {
            bridge(session: session)
        }
        return .object([
            "socket": .string(socketPath),
            "session_id": .string(sessionID),
        ])
    }

    private static func bindListener(_ session: Session) throws {
        if FileManager.default.fileExists(atPath: session.socketPath) {
            guard Darwin.unlink(session.socketPath) == 0 else {
                throw HelperError.system("unlink stale exec tty socket", errno)
            }
        }
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("exec tty socket", errno) }
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(session.socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(fd)
            throw HelperError.invalid("exec tty socket path is too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            let code = errno
            Darwin.close(fd)
            throw HelperError.system("bind exec tty socket", code)
        }
        guard chmod(session.socketPath, 0o600) == 0, Darwin.listen(fd, 1) == 0 else {
            let code = errno
            Darwin.close(fd)
            Darwin.unlink(session.socketPath)
            throw HelperError.system("listen exec tty socket", code)
        }
        session.listener = fd
    }

    private static func bridge(session: Session) {
        defer { cleanup(session: session) }
        var address = sockaddr_un()
        var length = socklen_t(MemoryLayout<sockaddr_un>.size)
        let clientFD = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.accept(session.listener, $0, &length)
            }
        }
        guard clientFD >= 0 else { return }
        session.clientFD = clientFD
        Darwin.close(session.listener)
        session.listener = -1

        let group = DispatchGroup()
        group.enter()
        DispatchQueue.global().async {
            defer { group.leave() }
            while true {
                do {
                    let (type, payload) = try readMux(from: clientFD)
                    try session.client.writeMux(type: type, payload: payload)
                    if type == .stdinEOF { return }
                } catch {
                    return
                }
            }
        }
        group.enter()
        DispatchQueue.global().async {
            defer { group.leave() }
            while true {
                do {
                    let (type, payload) = try session.client.readMux(timeout: 3600)
                    let frame = try GuestAgentMux.encode(type: type, payload: payload)
                    if !writeAll(frame, to: clientFD) { return }
                    if type == .exit { return }
                } catch {
                    return
                }
            }
        }
        _ = group.wait(timeout: .distantFuture)
    }

    private static func readMux(from fd: Int32) throws -> (GuestAgentMuxType, Data) {
        let header = try readExact(5, from: fd)
        let typeRaw = header[header.startIndex]
        guard let type = GuestAgentMuxType(rawValue: typeRaw) else {
            throw GuestAgentError.protocolViolation("unknown mux frame type")
        }
        let length = header.subdata(in: 1..<5).withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))
        }
        guard length <= GuestAgentMux.maxFrame else {
            throw GuestAgentError.protocolViolation("mux frame exceeds 1 MiB")
        }
        let payload = length == 0 ? Data() : try readExact(Int(length), from: fd)
        return (type, payload)
    }

    private static func readExact(_ count: Int, from fd: Int32) throws -> Data {
        var data = Data(count: count)
        var offset = 0
        while offset < count {
            let received = data.withUnsafeMutableBytes { raw -> Int in
                guard let base = raw.baseAddress else { return -1 }
                return Darwin.read(fd, base.advanced(by: offset), count - offset)
            }
            if received > 0 {
                offset += received
            } else if received == 0 {
                throw GuestAgentError.unavailable("cli socket closed")
            } else if errno == EINTR {
                continue
            } else {
                throw GuestAgentError.unavailable("cli socket read failed")
            }
        }
        return data
    }

    private static func writeAll(_ data: Data, to fd: Int32) -> Bool {
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return true }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
                if count <= 0 { return false }
                offset += count
            }
            return true
        }
    }

    private static func cleanup(session: Session) {
        if session.clientFD >= 0 {
            Darwin.shutdown(session.clientFD, SHUT_RDWR)
            Darwin.close(session.clientFD)
            session.clientFD = -1
        }
        if session.listener >= 0 {
            Darwin.close(session.listener)
            session.listener = -1
        }
        session.client.close()
        Darwin.unlink(session.socketPath)
    }
}
