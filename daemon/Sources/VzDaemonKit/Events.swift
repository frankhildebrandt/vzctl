import Foundation

public struct EventEnvelope: Codable, Equatable, Sendable {
    public let v: Int
    public let ts: String
    public let type: String
    public let data: [String: JSONValue]

    public init(
        v: Int = 1,
        ts: String,
        type: String,
        data: [String: JSONValue]
    ) {
        self.v = v
        self.ts = ts
        self.type = type
        self.data = data
    }

    public init(type: String, data: [String: JSONValue], at date: Date = Date()) {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        self.init(ts: formatter.string(from: date), type: type, data: data)
    }
}

public enum EventFilterError: Error, Equatable {
    case invalidPattern(String)
}

public struct EventFilter: Equatable, Sendable {
    private let patterns: [String]

    public init(_ expression: String?) throws {
        guard let expression else {
            patterns = ["*"]
            return
        }

        let parsed = expression.split(separator: ",", omittingEmptySubsequences: false)
            .map { $0.trimmingCharacters(in: .whitespaces) }
        guard !parsed.isEmpty else {
            throw EventFilterError.invalidPattern(expression)
        }
        for pattern in parsed {
            let stars = pattern.filter { $0 == "*" }.count
            guard !pattern.isEmpty,
                  stars <= 1,
                  stars == 0 || pattern.hasSuffix("*")
            else {
                throw EventFilterError.invalidPattern(pattern)
            }
        }
        patterns = parsed
    }

    public func matches(_ eventType: String) -> Bool {
        patterns.contains { pattern in
            if pattern == "*" {
                return true
            }
            if pattern.hasSuffix("*") {
                return eventType.hasPrefix(pattern.dropLast())
            }
            return eventType == pattern
        }
    }
}
