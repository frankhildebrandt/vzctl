import Darwin
import Foundation
import VzDaemonKit

enum HelperState: String, Sendable {
    case starting, running, stopped, failed
}

final class SupervisorReporter: @unchecked Sendable {
    private let vmID: String
    private let bundle: String
    private let socketPath: String
    private let logLock = NSLock()
    private var lastError: String?

    init(vmID: String, bundle: String, socketPath: String) {
        self.vmID = vmID
        self.bundle = bundle
        self.socketPath = socketPath
    }

    func report(_ state: HelperState, method: String = "helper.state") {
        let request = JSONRPCRequest(
            method: method,
            params: .object([
                "vm_id": .string(vmID),
                "state": .string(state.rawValue),
                "pid": .number(Double(getpid())),
                "bundle": .string(bundle),
            ]),
            id: .number(Double.random(in: 1...9_000_000))
        )
        do {
            _ = try send(request)
            logLock.withLock { lastError = nil }
        } catch {
            let message = String(describing: error)
            let shouldLog = logLock.withLock { () -> Bool in
                defer { lastError = message }
                return lastError != message
            }
            if shouldLog {
                fputs("supervisor unavailable; retrying: \(message)\n", stderr)
            }
        }
    }

    func reportClockCorrection(
        _ result: AgentTimeHintResult,
        reason: AgentTimeHintReason
    ) {
        let request = JSONRPCRequest(
            method: "vm.clock_corrected",
            params: .object([
                "vm_id": .string(vmID),
                "reason": .string(reason.rawValue),
                "observed_guest_unix_ms": .number(Double(result.observedGuestUnixMS)),
                "offset_ms": .number(Double(result.offsetMS)),
                "action": .string(result.action.rawValue),
            ]),
            id: .number(Double.random(in: 1...9_000_000))
        )
        do {
            _ = try send(request)
        } catch {
            fputs("supervisor unavailable; clock correction report deferred\n", stderr)
        }
    }

    private func send(_ request: JSONRPCRequest) throws -> JSONRPCResponse {
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("socket", errno) }
        defer { Darwin.close(fd) }

        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw HelperError.invalid("supervisor socket path is too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard connected == 0 else { throw HelperError.system("connect \(socketPath)", errno) }

        let encoded = try JSONRPCFraming.encode(request)
        let wrote = encoded.withUnsafeBytes { raw -> Bool in
            guard let base = raw.baseAddress else { return true }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
                if count <= 0 { return false }
                offset += count
            }
            return true
        }
        guard wrote else { throw HelperError.system("write supervisor request", errno) }

        var response = Data()
        var byte: UInt8 = 0
        while Darwin.read(fd, &byte, 1) == 1 {
            response.append(byte)
            if byte == 0x0A { break }
        }
        guard response.last == 0x0A else {
            throw HelperError.invalid("supervisor closed without response")
        }
        return try JSONRPCFraming.decode(JSONRPCResponse.self, from: response)
    }
}
