import Foundation
import VzDaemonKit

/// Coalesces card/detail polling without hiding changes to the stack YAML.
/// Failed collections are never cached. Mutations invalidate existing entries.
final class RestStatusCache: @unchecked Sendable {
    private struct Entry {
        var config: Data?
        var value: JSONValue
        var expires: Date
    }

    private let condition = NSCondition()
    private var entries: [String: Entry] = [:]
    private var collecting = Set<String>()
    private var generation = 0
    private let lifetime: TimeInterval
    private let now: () -> Date

    init(lifetime: TimeInterval = 15, now: @escaping () -> Date = Date.init) {
        self.lifetime = lifetime
        self.now = now
    }

    func invalidate() {
        condition.lock()
        generation += 1
        entries.removeAll()
        condition.broadcast()
        condition.unlock()
    }

    func get(path: String, config: Data?, build: () throws -> JSONValue) throws -> JSONValue {
        condition.lock()
        while collecting.contains(path) { condition.wait() }
        if let entry = entries[path], entry.config == config, entry.expires > now() {
            condition.unlock()
            return entry.value
        }
        let startedGeneration = generation
        collecting.insert(path)
        condition.unlock()
        do {
            let value = try build()
            condition.lock()
            if generation == startedGeneration {
                // Polling arbitrary historical paths must not grow memory forever.
                entries = entries.filter { $0.value.expires > now() }
                if entries.count >= 128 { entries.removeAll() }
                entries[path] = Entry(config: config, value: value, expires: now().addingTimeInterval(lifetime))
            }
            collecting.remove(path)
            condition.broadcast()
            condition.unlock()
            return value
        } catch {
            condition.lock()
            collecting.remove(path)
            condition.broadcast()
            condition.unlock()
            throw error
        }
    }
}
