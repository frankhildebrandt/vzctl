import Foundation

public indirect enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() {
            self = .null
        } else if let value = try? container.decode(Bool.self) {
            self = .bool(value)
        } else if let value = try? container.decode(Double.self) {
            self = .number(value)
        } else if let value = try? container.decode(String.self) {
            self = .string(value)
        } else if let value = try? container.decode([JSONValue].self) {
            self = .array(value)
        } else {
            self = .object(try container.decode([String: JSONValue].self))
        }
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null:
            try container.encodeNil()
        case let .bool(value):
            try container.encode(value)
        case let .number(value):
            try container.encode(value)
        case let .string(value):
            try container.encode(value)
        case let .array(value):
            try container.encode(value)
        case let .object(value):
            try container.encode(value)
        }
    }
}

public struct JSONRPCRequest: Codable, Equatable, Sendable {
    public var jsonrpc: String
    public var method: String
    public var params: JSONValue?
    public var id: JSONValue?

    public init(
        jsonrpc: String = "2.0",
        method: String,
        params: JSONValue? = nil,
        id: JSONValue? = nil
    ) {
        self.jsonrpc = jsonrpc
        self.method = method
        self.params = params
        self.id = id
    }
}

public struct JSONRPCError: Codable, Equatable, Sendable {
    public var code: Int
    public var message: String

    public init(code: Int, message: String) {
        self.code = code
        self.message = message
    }
}

public struct JSONRPCResponse: Codable, Equatable, Sendable {
    public var jsonrpc = "2.0"
    public var result: JSONValue?
    public var error: JSONRPCError?
    public var id: JSONValue?

    public init(result: JSONValue? = nil, error: JSONRPCError? = nil, id: JSONValue?) {
        self.result = result
        self.error = error
        self.id = id
    }
}

public enum JSONRPCFraming {
    public static func encode<T: Encodable>(_ message: T) throws -> Data {
        var data = try JSONEncoder().encode(message)
        data.append(0x0A)
        return data
    }

    public static func decode<T: Decodable>(_ type: T.Type, from line: Data) throws -> T {
        var payload = line
        while let last = payload.last, last == 0x0A || last == 0x0D {
            payload.removeLast()
        }
        return try JSONDecoder().decode(type, from: payload)
    }
}
