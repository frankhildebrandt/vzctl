import Foundation
import VzDaemonKit
import vmnet

struct HelperVmnetNIC: Sendable {
    let networkName: String
    let ip: String
    let macAddress: String
    /// Process-local ref rebuilt from supervisor serialization; owned by runtime.
    nonisolated(unsafe) let network: vmnet_network_ref
}

enum HelperVmnetClient {
    static func fetchAttachments(
        vmID: String,
        socketPath: String,
        bundleURL: URL
    ) throws -> [HelperVmnetNIC] {
        let request = JSONRPCRequest(
            method: "helper.networks",
            params: .object(["vm_id": .string(vmID)]),
            id: .number(Double.random(in: 1...9_000_000))
        )
        let response = try SupervisorRPC.send(request, socketPath: socketPath)
        if let error = response.error {
            throw HelperError.invalid("helper.networks: \(error.message)")
        }
        guard case let .object(result)? = response.result,
              case let .array(rawAttachments)? = result["attachments"]
        else {
            throw HelperError.invalid("helper.networks returned invalid result")
        }
        let macByNetwork = try HelperArguments.manifestNICs(bundleURL: bundleURL)
        let orderedMACs = try HelperArguments.manifestMACList(bundleURL: bundleURL)
        var nics: [HelperVmnetNIC] = []
        nics.reserveCapacity(rawAttachments.count)
        for (index, item) in rawAttachments.enumerated() {
            guard case let .object(values) = item,
                  case let .string(networkName)? = values["network"],
                  case let .string(ip)? = values["ip"],
                  case let .string(serialization)? = values["serialization"]
            else {
                throw HelperError.invalid("helper.networks attachment \(index) is invalid")
            }
            let blob = try VmnetSerialization.blob(fromBase64: serialization)
            let network: vmnet_network_ref
            do {
                network = try VmnetSerialization.network(from: blob)
            } catch {
                throw HelperError.invalid(
                    "cannot recreate vmnet for \(networkName): \(error)"
                )
            }
            let mac = macByNetwork[networkName]
                ?? orderedMACs[safe: index]
                ?? randomLocalMAC()
            nics.append(
                HelperVmnetNIC(
                    networkName: networkName,
                    ip: ip,
                    macAddress: mac,
                    network: network
                )
            )
        }
        return nics
    }

    private static func randomLocalMAC() -> String {
        String(
            format: "02:%02x:%02x:%02x:%02x:%02x",
            UInt8.random(in: 0...255),
            UInt8.random(in: 0...255),
            UInt8.random(in: 0...255),
            UInt8.random(in: 0...255),
            UInt8.random(in: 0...255)
        )
    }
}

private enum SupervisorRPC {
    static func send(_ request: JSONRPCRequest, socketPath: String) throws -> JSONRPCResponse {
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
        guard connected == 0 else {
            throw HelperError.system("connect \(socketPath)", errno)
        }

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

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}
