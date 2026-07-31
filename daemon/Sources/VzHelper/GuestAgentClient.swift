import Darwin
import Foundation

let guestAgentPort: UInt32 = 21_950
private let guestAgentMaxFrame = 1_048_576

enum GuestAgentError: Error, CustomStringConvertible {
    case unavailable(String)
    case timeout(String)
    case protocolViolation(String)
    case remote(code: String, message: String, details: GuestAgentRemoteDetails)

    var description: String {
        switch self {
        case let .unavailable(message):
            return "guest agent unavailable: \(message); SSH/serial is diagnostic fallback only"
        case let .timeout(operation):
            return "guest agent timeout during \(operation)"
        case let .protocolViolation(message):
            return "guest agent protocol error: \(message)"
        case let .remote(code, message, _):
            return "guest agent \(code): \(message)"
        }
    }
}

struct GuestAgentRemoteDetails: @unchecked Sendable {
    let values: [String: Any]
}

struct AgentHello {
    let version: String
    let capabilities: [String]
}

struct AgentExecResult {
    let exit: Int
    let stdout: String
    let stderr: String
    let truncated: Bool
}

struct AgentInterface {
    let name: String
    let mac: String
    let addresses: [String]
}

enum AgentTimeHintReason: String, Sendable {
    case handshake, wake, manual
}

enum AgentTimeHintAction: String, Sendable {
    case none, stepped, skipped
}

struct AgentTimeHintResult: Sendable {
    let observedGuestUnixMS: Int64
    let offsetMS: Int64
    let action: AgentTimeHintAction
}

final class GuestAgentClient: @unchecked Sendable {
    private let fileDescriptor: Int32
    private let ownsFileDescriptor: Bool
    private var nextID = 0
    private var closed = false

    init(fileDescriptor: Int32, ownsFileDescriptor: Bool = true) {
        self.fileDescriptor = fileDescriptor
        self.ownsFileDescriptor = ownsFileDescriptor
    }

    deinit {
        close()
    }

    func close() {
        guard !closed else { return }
        closed = true
        if ownsFileDescriptor {
            Darwin.close(fileDescriptor)
        }
    }

    func hello(token: String, helperVersion: String, timeout: TimeInterval = 2) throws -> AgentHello {
        let result = try request(
            method: "hello",
            params: ["token": token, "helper_version": helperVersion],
            timeout: timeout,
            allowCancel: false
        )
        guard
            let protocolVersion = result["v"] as? Int,
            protocolVersion == 1,
            let agentVersion = result["agent_version"] as? String,
            let capabilities = result["capabilities"] as? [String]
        else {
            throw GuestAgentError.protocolViolation("invalid hello result")
        }
        return AgentHello(version: agentVersion, capabilities: capabilities)
    }

    func ping(nonce: String? = nil, timeout: TimeInterval = 1) throws {
        var params: [String: Any] = [:]
        if let nonce { params["nonce"] = nonce }
        let result = try request(method: "ping", params: params, timeout: timeout)
        guard result["pong"] as? Bool == true else {
            throw GuestAgentError.protocolViolation("ping did not return pong")
        }
        if let nonce, result["nonce"] as? String != nonce {
            throw GuestAgentError.protocolViolation("ping nonce mismatch")
        }
    }

    func version(timeout: TimeInterval = 1) throws -> AgentHello {
        let result = try request(method: "version", params: [:], timeout: timeout)
        guard
            let protocolVersion = result["v"] as? Int,
            protocolVersion == 1,
            let agentVersion = result["agent_version"] as? String,
            let capabilities = result["capabilities"] as? [String]
        else {
            throw GuestAgentError.protocolViolation("invalid version result")
        }
        return AgentHello(version: agentVersion, capabilities: capabilities)
    }

    func health(timeout: TimeInterval = 2) throws -> String {
        let result = try request(method: "health", params: [:], timeout: timeout)
        guard let status = result["status"] as? String, ["ok", "degraded"].contains(status) else {
            throw GuestAgentError.protocolViolation("invalid health result")
        }
        return status
    }

    func exec(
        argv: [String],
        cwd: String? = nil,
        environment: [String: String] = [:],
        stdin: Data? = nil,
        timeoutMilliseconds: Int = 30_000,
        helperTimeout: TimeInterval? = nil
    ) throws -> AgentExecResult {
        var params: [String: Any] = [
            "cmd": argv,
            "timeout_ms": timeoutMilliseconds,
        ]
        if let cwd { params["cwd"] = cwd }
        if !environment.isEmpty { params["env"] = environment }
        if let stdin { params["stdin_b64"] = stdin.base64EncodedString() }
        let result = try request(
            method: "exec",
            params: params,
            timeout: helperTimeout ?? (Double(timeoutMilliseconds) / 1_000 + 1)
        )
        guard
            let exit = result["exit"] as? Int,
            let stdout = result["stdout"] as? String,
            let stderr = result["stderr"] as? String,
            let truncated = result["truncated"] as? Bool
        else {
            throw GuestAgentError.protocolViolation("invalid exec result")
        }
        return AgentExecResult(exit: exit, stdout: stdout, stderr: stderr, truncated: truncated)
    }

    /// Negotiates an interactive PTY upgrade. After success, only mux frames
    /// may be used on this connection (`writeMux` / `readMux`).
    func upgradeTTYExec(
        argv: [String],
        cwd: String? = nil,
        environment: [String: String] = [:],
        cols: Int = 80,
        rows: Int = 24,
        timeout: TimeInterval = 10
    ) throws {
        var params: [String: Any] = [
            "cmd": argv,
            "tty": true,
            "cols": cols,
            "rows": rows,
        ]
        if let cwd { params["cwd"] = cwd }
        if !environment.isEmpty { params["env"] = environment }
        let result = try request(
            method: "exec",
            params: params,
            timeout: timeout,
            allowCancel: false
        )
        guard result["upgraded"] as? Bool == true else {
            throw GuestAgentError.protocolViolation("tty exec did not upgrade")
        }
    }

    func writeMux(type: GuestAgentMuxType, payload: Data = Data()) throws {
        let frame = try GuestAgentMux.encode(type: type, payload: payload)
        try writeAll(frame, deadline: Date().addingTimeInterval(30))
    }

    func readMux(timeout: TimeInterval = 30) throws -> (GuestAgentMuxType, Data) {
        let deadline = Date().addingTimeInterval(timeout)
        let header = try readExactly(5, deadline: deadline)
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
        let payload = length == 0
            ? Data()
            : try readExactly(Int(length), deadline: deadline)
        return (type, payload)
    }

    func reportIP(timeout: TimeInterval = 2) throws -> [AgentInterface] {
        let result = try request(method: "report_ip", params: [:], timeout: timeout)
        guard let rawInterfaces = result["interfaces"] as? [[String: Any]] else {
            throw GuestAgentError.protocolViolation("invalid report_ip result")
        }
        return try rawInterfaces.map { item in
            guard
                let name = item["name"] as? String,
                let mac = item["mac"] as? String,
                let addresses = item["addresses"] as? [String],
                !addresses.contains(where: Self.isDotZeroAddress)
            else {
                throw GuestAgentError.protocolViolation("invalid interface result")
            }
            return AgentInterface(name: name, mac: mac, addresses: addresses)
        }
    }

    func timeHint(
        hostUnixMS: Int64 = Int64(Date().timeIntervalSince1970 * 1_000),
        reason: AgentTimeHintReason,
        timeout: TimeInterval = 2
    ) throws -> AgentTimeHintResult {
        let result = try request(
            method: "time_hint",
            params: ["host_unix_ms": hostUnixMS, "reason": reason.rawValue],
            timeout: timeout
        )
        guard
            let observed = (result["observed_guest_unix_ms"] as? NSNumber)?.int64Value,
            let offset = (result["offset_ms"] as? NSNumber)?.int64Value,
            let rawAction = result["action"] as? String,
            let action = AgentTimeHintAction(rawValue: rawAction)
        else {
            throw GuestAgentError.protocolViolation("invalid time_hint result")
        }
        return AgentTimeHintResult(
            observedGuestUnixMS: observed,
            offsetMS: offset,
            action: action
        )
    }

    func fsMount(
        name: String,
        target: String,
        readOnly: Bool = false,
        timeout: TimeInterval = 10
    ) throws {
        _ = try request(
            method: "fs.mount",
            params: [
                "name": name,
                "target": target,
                "read_only": readOnly,
            ],
            timeout: timeout
        )
    }

    func fsUnmount(
        name: String? = nil,
        target: String? = nil,
        timeout: TimeInterval = 10
    ) throws {
        var params: [String: Any] = [:]
        if let name { params["name"] = name }
        if let target { params["target"] = target }
        _ = try request(method: "fs.unmount", params: params, timeout: timeout)
    }

    @discardableResult
    func caInject(
        pem: String,
        fingerprint: String,
        name: String = "vzctl-local",
        timeout: TimeInterval = 30
    ) throws -> [String: Any] {
        try request(
            method: "ca_inject",
            params: [
                "pem": pem,
                "fingerprint": fingerprint,
                "name": name,
            ],
            timeout: timeout
        )
    }

    private func request(
        method: String,
        params: [String: Any],
        timeout: TimeInterval,
        allowCancel: Bool = true
    ) throws -> [String: Any] {
        guard !closed else {
            throw GuestAgentError.unavailable("connection is closed")
        }
        nextID += 1
        let id = "helper-\(nextID)"
        let deadline = Date().addingTimeInterval(timeout)
        let envelope: [String: Any] = [
            "v": 1,
            "id": id,
            "method": method,
            "params": params,
        ]
        do {
            try writeFrame(envelope, deadline: deadline)
            let response = try readFrame(deadline: deadline)
            return try decodeResponse(response, expectedID: id)
        } catch let error as GuestAgentError {
            switch error {
            case .timeout:
                if allowCancel {
                    try? sendCancel(targetID: id)
                }
                close()
            case .unavailable, .protocolViolation:
                close()
            case let .remote(code, _, _):
                if code == "auth" { close() }
            }
            throw error
        } catch {
            close()
            throw GuestAgentError.unavailable(String(describing: error))
        }
    }

    private func sendCancel(targetID: String) throws {
        nextID += 1
        let deadline = Date().addingTimeInterval(0.25)
        try writeFrame(
            [
                "v": 1,
                "id": "helper-\(nextID)",
                "method": "cancel",
                "params": ["id": targetID],
            ],
            deadline: deadline
        )
    }

    private func writeFrame(_ object: [String: Any], deadline: Date) throws {
        guard JSONSerialization.isValidJSONObject(object) else {
            throw GuestAgentError.protocolViolation("request is not valid JSON")
        }
        let payload = try JSONSerialization.data(withJSONObject: object)
        guard !payload.isEmpty, payload.count <= guestAgentMaxFrame else {
            throw GuestAgentError.protocolViolation("request frame size is invalid")
        }
        var length = UInt32(payload.count).littleEndian
        let prefix = withUnsafeBytes(of: &length) { Data($0) }
        try writeAll(prefix, deadline: deadline)
        try writeAll(payload, deadline: deadline)
    }

    private func readFrame(deadline: Date) throws -> [String: Any] {
        let prefix = try readExactly(4, deadline: deadline)
        let length = prefix.withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))
        }
        guard length > 0, length <= guestAgentMaxFrame else {
            throw GuestAgentError.protocolViolation("response frame size is invalid")
        }
        let payload = try readExactly(Int(length), deadline: deadline)
        let object = try JSONSerialization.jsonObject(with: payload)
        guard let dictionary = object as? [String: Any] else {
            throw GuestAgentError.protocolViolation("response is not a JSON object")
        }
        return dictionary
    }

    private func decodeResponse(
        _ response: [String: Any],
        expectedID: String
    ) throws -> [String: Any] {
        guard response["v"] as? Int == 1, response["id"] as? String == expectedID else {
            throw GuestAgentError.protocolViolation("response envelope mismatch")
        }
        if response["ok"] as? Bool == true {
            guard response["error"] == nil, let result = response["result"] as? [String: Any] else {
                throw GuestAgentError.protocolViolation("invalid success response")
            }
            return result
        }
        guard
            response["result"] == nil,
            let error = response["error"] as? [String: Any],
            let code = error["code"] as? String,
            let message = error["message"] as? String
        else {
            throw GuestAgentError.protocolViolation("invalid error response")
        }
        throw GuestAgentError.remote(
            code: code,
            message: message,
            details: GuestAgentRemoteDetails(values: error["details"] as? [String: Any] ?? [:])
        )
    }

    private func writeAll(_ data: Data, deadline: Date) throws {
        try data.withUnsafeBytes { rawBuffer in
            guard let baseAddress = rawBuffer.baseAddress else { return }
            var offset = 0
            while offset < rawBuffer.count {
                try waitFor(events: Int16(POLLOUT), deadline: deadline)
                let written = Darwin.write(
                    fileDescriptor,
                    baseAddress.advanced(by: offset),
                    rawBuffer.count - offset
                )
                if written > 0 {
                    offset += written
                } else if written < 0, errno == EINTR {
                    continue
                } else {
                    throw GuestAgentError.unavailable("write failed")
                }
            }
        }
    }

    private func readExactly(_ count: Int, deadline: Date) throws -> Data {
        var result = Data(count: count)
        var offset = 0
        try result.withUnsafeMutableBytes { rawBuffer in
            guard let baseAddress = rawBuffer.baseAddress else { return }
            while offset < count {
                try waitFor(events: Int16(POLLIN), deadline: deadline)
                let received = Darwin.read(
                    fileDescriptor,
                    baseAddress.advanced(by: offset),
                    count - offset
                )
                if received > 0 {
                    offset += received
                } else if received == 0 {
                    throw GuestAgentError.unavailable("connection closed")
                } else if errno == EINTR {
                    continue
                } else {
                    throw GuestAgentError.unavailable("read failed")
                }
            }
        }
        return result
    }

    private func waitFor(events: Int16, deadline: Date) throws {
        while true {
            let remaining = deadline.timeIntervalSinceNow
            guard remaining > 0 else {
                throw GuestAgentError.timeout("request")
            }
            var descriptor = pollfd(fd: fileDescriptor, events: events, revents: 0)
            let milliseconds = Int32(min(remaining * 1_000, Double(Int32.max)).rounded(.up))
            let result = Darwin.poll(&descriptor, 1, milliseconds)
            if result > 0 {
                if descriptor.revents & events != 0 {
                    return
                }
                if descriptor.revents & Int16(POLLNVAL | POLLERR | POLLHUP) != 0 {
                    throw GuestAgentError.unavailable("socket closed")
                }
                continue
            }
            if result == 0 {
                throw GuestAgentError.timeout("request")
            }
            if errno != EINTR {
                throw GuestAgentError.unavailable("poll failed")
            }
        }
    }

    private static func isDotZeroAddress(_ value: String) -> Bool {
        guard let slash = value.firstIndex(of: "/") else { return true }
        let address = String(value[..<slash])
        return address.split(separator: ".").last == "0"
    }
}

enum AgentToken {
    static func load(from url: URL) throws -> String {
        let attributes = try FileManager.default.attributesOfItem(atPath: url.path)
        guard (attributes[.type] as? FileAttributeType) == .typeRegular else {
            throw HelperError.invalid("agent token is not a regular file")
        }
        guard (attributes[.posixPermissions] as? NSNumber)?.uint16Value == 0o600 else {
            throw HelperError.invalid("agent token file must have mode 0600")
        }
        let raw = try String(contentsOf: url, encoding: .utf8)
        let token = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard
            !token.isEmpty,
            !token.contains(where: \.isWhitespace),
            token.range(of: "^[A-Za-z0-9_-]+$", options: .regularExpression) != nil
        else {
            throw HelperError.invalid("agent token must be unpadded base64url")
        }
        let base64 = token
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        let padded = base64 + String(repeating: "=", count: (4 - base64.count % 4) % 4)
        guard let decoded = Data(base64Encoded: padded), decoded.count >= 32 else {
            throw HelperError.invalid("agent token must contain at least 256 random bits")
        }
        return token
    }
}
