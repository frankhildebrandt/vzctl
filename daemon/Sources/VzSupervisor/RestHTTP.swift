import Foundation
import VzDaemonKit

struct RestErrorBody: Codable, Sendable {
    struct ErrorPayload: Codable, Sendable {
        var code: String
        var message: String
        var details: [String: JSONValue]?
    }

    var error: ErrorPayload
}

enum RestErrorCode: String, Sendable {
    case badRequest = "bad_request"
    case notFound = "not_found"
    case conflict = "conflict"
    case unauthorized = "unauthorized"
    case failedPrecondition = "failed_precondition"
    case internalError = "internal"
    case notImplemented = "not_implemented"
}

struct RestHTTPRequest: Sendable {
    var method: String
    var path: String
    var query: [String: String]
    var headers: [String: String]
    var body: Data
}

struct RestHTTPResponse: Sendable {
    var status: Int
    var headers: [String: String]
    var body: Data

    static func json(
        _ status: Int,
        _ value: some Encodable,
        headers: [String: String] = [:]
    ) throws -> RestHTTPResponse {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        var merged = headers
        merged["Content-Type"] = "application/json; charset=utf-8"
        return RestHTTPResponse(status: status, headers: merged, body: try encoder.encode(value))
    }

    static func jsonValue(
        _ status: Int,
        _ value: JSONValue,
        headers: [String: String] = [:]
    ) throws -> RestHTTPResponse {
        try json(status, value, headers: headers)
    }

    static func text(
        _ status: Int,
        _ text: String,
        contentType: String = "text/plain; charset=utf-8"
    ) -> RestHTTPResponse {
        RestHTTPResponse(
            status: status,
            headers: ["Content-Type": contentType],
            body: Data(text.utf8)
        )
    }

    static func error(
        _ status: Int,
        code: RestErrorCode,
        message: String,
        details: [String: JSONValue]? = nil
    ) throws -> RestHTTPResponse {
        try json(
            status,
            RestErrorBody(error: .init(code: code.rawValue, message: message, details: details))
        )
    }

    static func empty(_ status: Int) -> RestHTTPResponse {
        RestHTTPResponse(status: status, headers: [:], body: Data())
    }
}

enum RestHTTP {
    static let statusText: [Int: String] = [
        200: "OK",
        201: "Created",
        202: "Accepted",
        204: "No Content",
        400: "Bad Request",
        401: "Unauthorized",
        404: "Not Found",
        409: "Conflict",
        500: "Internal Server Error",
        501: "Not Implemented",
    ]

    static func parseRequest(from data: Data) -> RestHTTPRequest? {
        guard let headerEnd = findHeaderEnd(data) else { return nil }
        let headerData = data[..<headerEnd]
        let bodyStart = headerEnd + 4
        let body = bodyStart < data.count ? Data(data[bodyStart...]) : Data()

        guard let headerText = String(data: headerData, encoding: .utf8) else { return nil }
        let lines = headerText.split(separator: "\r\n", omittingEmptySubsequences: false)
        guard let requestLine = lines.first else { return nil }
        let parts = requestLine.split(separator: " ")
        guard parts.count >= 2 else { return nil }
        let method = String(parts[0])
        let rawTarget = String(parts[1])
        let (path, query) = splitTarget(rawTarget)

        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let name = String(line[..<colon]).trimmingCharacters(in: .whitespaces).lowercased()
            let value = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
            headers[name] = value
        }

        var bodyData = body
        if let lengthText = headers["content-length"], let length = Int(lengthText) {
            if bodyData.count > length {
                bodyData = bodyData.prefix(length)
            }
            // Incomplete body — caller should wait for more bytes.
            if bodyData.count < length {
                return nil
            }
        }

        return RestHTTPRequest(
            method: method.uppercased(),
            path: path,
            query: query,
            headers: headers,
            body: Data(bodyData)
        )
    }

    static func encodeResponse(_ response: RestHTTPResponse) -> Data {
        let reason = statusText[response.status] ?? "OK"
        var headerLines = [
            "HTTP/1.1 \(response.status) \(reason)",
            "Content-Length: \(response.body.count)",
            "Connection: close",
        ]
        for (key, value) in response.headers.sorted(by: { $0.key < $1.key }) {
            if key.lowercased() == "content-length" || key.lowercased() == "connection" {
                continue
            }
            headerLines.append("\(key): \(value)")
        }
        var data = Data((headerLines.joined(separator: "\r\n") + "\r\n\r\n").utf8)
        data.append(response.body)
        return data
    }

    private static func findHeaderEnd(_ data: Data) -> Int? {
        let pattern: [UInt8] = [0x0D, 0x0A, 0x0D, 0x0A]
        guard data.count >= 4 else { return nil }
        for i in 0 ... (data.count - 4) {
            if data[i] == pattern[0],
               data[i + 1] == pattern[1],
               data[i + 2] == pattern[2],
               data[i + 3] == pattern[3]
            {
                return i
            }
        }
        return nil
    }

    private static func splitTarget(_ target: String) -> (String, [String: String]) {
        guard let q = target.firstIndex(of: "?") else {
            return (percentDecode(target), [:])
        }
        let path = percentDecode(String(target[..<q]))
        let queryString = String(target[target.index(after: q)...])
        var query: [String: String] = [:]
        for pair in queryString.split(separator: "&") {
            let pieces = pair.split(separator: "=", maxSplits: 1)
            let key = percentDecode(String(pieces[0]))
            let value = pieces.count > 1 ? percentDecode(String(pieces[1])) : ""
            query[key] = value
        }
        return (path, query)
    }

    static func percentDecode(_ value: String) -> String {
        var result = ""
        var i = value.startIndex
        while i < value.endIndex {
            let ch = value[i]
            if ch == "%",
               value.index(i, offsetBy: 2, limitedBy: value.endIndex) != nil
            {
                let hex = value[value.index(after: i) ... value.index(i, offsetBy: 2)]
                if let byte = UInt8(hex, radix: 16) {
                    result.append(Character(UnicodeScalar(byte)))
                    i = value.index(i, offsetBy: 3)
                    continue
                }
            }
            if ch == "+" {
                result.append(" ")
            } else {
                result.append(ch)
            }
            i = value.index(after: i)
        }
        return result
    }

    static func pathSegments(_ path: String) -> [String] {
        path.split(separator: "/", omittingEmptySubsequences: true)
            .map { percentDecode(String($0)) }
    }
}
