import Darwin
import Foundation
import VzDaemonKit

enum HelperRouteClient {
    static func run(
        _ operation: RouterOperation,
        _ plan: RouterPlan,
        stateDirectory: URL,
        timeoutSeconds: Int = 35
    ) throws -> JSONValue {
        let path = stateDirectory
            .appendingPathComponent("helpers", isDirectory: true)
            .appendingPathComponent("\(StateFileName.socketComponent(plan.vmID)).sock")
            .path
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw RouteApplyError.unavailable("helper socket: \(errno)") }
        defer { Darwin.close(fd) }
        var receiveTimeout = timeval(tv_sec: timeoutSeconds, tv_usec: 0)
        setsockopt(
            fd,
            SOL_SOCKET,
            SO_RCVTIMEO,
            &receiveTimeout,
            socklen_t(MemoryLayout<timeval>.size)
        )
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            throw RouteApplyError.unavailable("helper socket path is too long")
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
        guard connected == 0 else {
            throw RouteApplyError.unavailable(
                "router helper \(plan.vmID) is unavailable at \(path)"
            )
        }
        let request = JSONRPCRequest(
            method: "route.\(operation.rawValue)",
            params: plan.json,
            id: .number(1)
        )
        let encoded = try JSONRPCFraming.encode(request)
        guard writeAll(encoded, to: fd), let line = readLine(from: fd) else {
            throw RouteApplyError.unavailable("router helper \(plan.vmID) did not respond")
        }
        let response = try JSONRPCFraming.decode(JSONRPCResponse.self, from: line)
        if let error = response.error {
            throw RouteApplyError.guest(error.message)
        }
        guard let result = response.result, case .object = result else {
            throw RouteApplyError.guest("router helper returned an invalid response")
        }
        return result
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

    private static func readLine(from fd: Int32) -> Data? {
        var data = Data()
        var byte: UInt8 = 0
        while Darwin.read(fd, &byte, 1) == 1 {
            data.append(byte)
            if byte == 0x0A { return data }
            if data.count > 1_048_576 { return nil }
        }
        return nil
    }
}
