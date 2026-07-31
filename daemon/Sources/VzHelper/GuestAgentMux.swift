import Darwin
import Foundation

enum GuestAgentMuxType: UInt8 {
    case stdin = 0x01
    case stdout = 0x02
    case resize = 0x04
    case exit = 0x05
    case stdinEOF = 0x06
}

enum GuestAgentMux {
    static let maxFrame = 1_048_576

    static func encode(type: GuestAgentMuxType, payload: Data = Data()) throws -> Data {
        guard payload.count <= maxFrame else {
            throw GuestAgentError.protocolViolation("mux frame exceeds 1 MiB")
        }
        var data = Data(capacity: 5 + payload.count)
        data.append(type.rawValue)
        var length = UInt32(payload.count).littleEndian
        withUnsafeBytes(of: &length) { data.append(contentsOf: $0) }
        data.append(payload)
        return data
    }

    static func decode(_ data: Data) throws -> (GuestAgentMuxType, Data) {
        guard data.count >= 5 else {
            throw GuestAgentError.protocolViolation("mux frame too short")
        }
        guard let type = GuestAgentMuxType(rawValue: data[data.startIndex]) else {
            throw GuestAgentError.protocolViolation("unknown mux frame type")
        }
        let length: UInt32 = data.subdata(in: 1..<5).withUnsafeBytes {
            UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self))
        }
        guard length <= maxFrame, data.count == 5 + Int(length) else {
            throw GuestAgentError.protocolViolation("invalid mux frame length")
        }
        let payload = length == 0 ? Data() : data.subdata(in: 5..<data.count)
        return (type, payload)
    }

    static func resizePayload(cols: UInt16, rows: UInt16) -> Data {
        var data = Data(count: 4)
        data.withUnsafeMutableBytes { raw in
            raw.storeBytes(of: cols.littleEndian, as: UInt16.self)
            raw.storeBytes(of: rows.littleEndian, toByteOffset: 2, as: UInt16.self)
        }
        return data
    }

    static func exitStatus(from payload: Data) throws -> Int32 {
        guard payload.count == 4 else {
            throw GuestAgentError.protocolViolation("invalid exit payload")
        }
        return payload.withUnsafeBytes {
            Int32(bitPattern: UInt32(littleEndian: $0.loadUnaligned(as: UInt32.self)))
        }
    }
}
