import Darwin
import Foundation
import VzDaemonKit

enum HelperGuestServiceBridge {
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
        params: HelperAgentRequest.ServicesHTTPParams,
        runtime: VirtualMachineRuntime,
        token: String,
        stateDirectory: URL,
        vmID: String
    ) async throws -> JSONValue {
        let client = try await runtime.connectToGuestAgent(timeout: 5)
        let hello = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        guard hello.capabilities.contains("guest_publish") else {
            client.close()
            throw RouteApplyError.invalid(
                "guest agent missing guest_publish capability; upgrade guest utils"
            )
        }
        let meta: [String: Any]
        do {
            meta = try client.upgradeServiceStream(
                name: params.name,
                method: params.method,
                path: params.path,
                headers: params.headers
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
        let sessionID = String(
            UUID().uuidString.replacingOccurrences(of: "-", with: "").prefix(16)
        ).lowercased()
        let vmToken = String(StateFileName.component(vmID).suffix(8))
        let socketPath = helpers
            .appendingPathComponent("s-\(vmToken)-\(sessionID).sock")
            .path
        let session = Session(client: client, socketPath: socketPath)
        try bindListener(session)
        DispatchQueue.global().async {
            bridge(session: session)
        }
        var result: [String: JSONValue] = [
            "socket": .string(socketPath),
            "session_id": .string(sessionID),
            "upgraded": .bool(true),
        ]
        if let status = meta["status"] as? Int {
            result["status"] = .number(Double(status))
        }
        if let contentType = meta["content_type"] as? String {
            result["content_type"] = .string(contentType)
        }
        return .object(result)
    }

    private static func bindListener(_ session: Session) throws {
        if FileManager.default.fileExists(atPath: session.socketPath) {
            guard Darwin.unlink(session.socketPath) == 0 else {
                throw HelperError.system("unlink stale guest service socket", errno)
            }
        }
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("guest service socket", errno) }
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(session.socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(fd)
            throw HelperError.invalid("guest service socket path is too long")
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
            throw HelperError.system("bind guest service socket", code)
        }
        guard chmod(session.socketPath, 0o600) == 0, Darwin.listen(fd, 1) == 0 else {
            let code = errno
            Darwin.close(fd)
            Darwin.unlink(session.socketPath)
            throw HelperError.system("listen guest service socket", code)
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

        while true {
            do {
                let (type, payload) = try session.client.readMux(timeout: 3600)
                if type == .stdout, !payload.isEmpty {
                    if !writeAll(payload, to: clientFD) { return }
                }
                if type == .exit { return }
            } catch {
                return
            }
        }
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
        if session.clientFD >= 0 { Darwin.close(session.clientFD) }
        if session.listener >= 0 { Darwin.close(session.listener) }
        session.client.close()
        Darwin.unlink(session.socketPath)
    }
}
