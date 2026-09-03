import Foundation

/// Join per-project Caddyfiles into one process config.
///
/// Shared :80/:443 cannot run two Caddy children on the same loopback ports.
/// The first global block is kept; later globals are dropped; site blocks are concatenated.
enum CaddyfileMerge {
    static func merge(_ files: [String]) -> String {
        let files = files
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
            .filter { !$0.isEmpty }
        guard !files.isEmpty else { return "" }
        var global = ""
        var sites: [String] = []
        for file in files {
            if let split = splitLeadingGlobalBlock(file) {
                if global.isEmpty { global = split.global }
                let rest = split.sites.trimmingCharacters(in: .whitespacesAndNewlines)
                if !rest.isEmpty { sites.append(rest) }
            } else {
                sites.append(file)
            }
        }
        if global.isEmpty {
            return sites.joined(separator: "\n\n") + (sites.isEmpty ? "" : "\n")
        }
        if sites.isEmpty {
            return global + "\n"
        }
        return global + "\n\n" + sites.joined(separator: "\n\n") + "\n"
    }

    static func splitLeadingGlobalBlock(_ source: String) -> (global: String, sites: String)? {
        let chars = Array(source)
        var i = 0
        while i < chars.count, chars[i].isWhitespace { i += 1 }
        guard i < chars.count, chars[i] == "{" else { return nil }
        var depth = 0
        var j = i
        while j < chars.count {
            if chars[j] == "{" { depth += 1 }
            if chars[j] == "}" {
                depth -= 1
                if depth == 0 {
                    let global = String(chars[i ... j])
                    let sites = String(chars[(j + 1)...])
                    return (global, sites)
                }
            }
            j += 1
        }
        return nil
    }
}
