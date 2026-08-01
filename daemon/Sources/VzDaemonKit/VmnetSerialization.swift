import Foundation
import vmnet

/// Encode/decode `vmnet_network_copy_serialization` payloads for UDS JSON-RPC.
///
/// Apple returns an XPC dictionary with a single `networkSerialization` data
/// value (no FDs). That blob round-trips across processes via
/// `vmnet_network_create_with_serialization` while the supervisor keeps the
/// original `vmnet_network_ref` alive (ADR 0002).
public enum VmnetSerialization {
    public static let dictionaryKey = "networkSerialization"

    public enum Error: Swift.Error, CustomStringConvertible {
        case copyFailed(vmnet_return_t)
        case unexpectedType
        case missingBlob
        case recreateFailed(vmnet_return_t)

        public var description: String {
            switch self {
            case let .copyFailed(status):
                return "vmnet_network_copy_serialization failed (\(status.rawValue))"
            case .unexpectedType:
                return "vmnet serialization is not an XPC dictionary"
            case .missingBlob:
                return "vmnet serialization missing \(dictionaryKey) data"
            case let .recreateFailed(status):
                return "vmnet_network_create_with_serialization failed (\(status.rawValue))"
            }
        }
    }

    /// Extract the portable blob from a live supervisor-owned network ref.
    public static func blob(from network: vmnet_network_ref) throws -> Data {
        var status: vmnet_return_t = .VMNET_SUCCESS
        guard let serialized = vmnet_network_copy_serialization(network, &status) else {
            throw Error.copyFailed(status)
        }
        let type = xpc_get_type(serialized)
        guard type == XPC_TYPE_DICTIONARY else {
            throw Error.unexpectedType
        }
        guard let value = xpc_dictionary_get_value(serialized, dictionaryKey),
              xpc_get_type(value) == XPC_TYPE_DATA
        else {
            throw Error.missingBlob
        }
        let length = xpc_data_get_length(value)
        guard let pointer = xpc_data_get_bytes_ptr(value), length > 0 else {
            throw Error.missingBlob
        }
        return Data(bytes: pointer, count: length)
    }

    /// Rebuild a process-local `vmnet_network_ref` from a transported blob.
    /// Caller owns the returned CF-retained reference and must release it.
    public static func network(from blob: Data) throws -> vmnet_network_ref {
        let dictionary = xpc_dictionary_create(nil, nil, 0)
        try blob.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else {
                throw Error.missingBlob
            }
            xpc_dictionary_set_data(dictionary, dictionaryKey, base, blob.count)
        }
        var status: vmnet_return_t = .VMNET_SUCCESS
        guard let network = vmnet_network_create_with_serialization(dictionary, &status) else {
            throw Error.recreateFailed(status)
        }
        return network
    }

    public static func base64(from blob: Data) -> String {
        blob.base64EncodedString()
    }

    public static func blob(fromBase64 string: String) throws -> Data {
        guard let data = Data(base64Encoded: string) else {
            throw Error.missingBlob
        }
        return data
    }
}
