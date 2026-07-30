import Foundation

struct HostWakeDetector {
    private let minimumGap: TimeInterval
    private var lastObservation: Date?

    init(minimumGap: TimeInterval = 5) {
        self.minimumGap = minimumGap
    }

    mutating func observe(_ now: Date) -> Bool {
        defer { lastObservation = now }
        guard let lastObservation else { return false }
        return now.timeIntervalSince(lastObservation) >= minimumGap
    }
}
