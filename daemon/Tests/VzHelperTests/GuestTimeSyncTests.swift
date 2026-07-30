import Foundation
import Testing
@testable import VzHelper

@Test func wakeDetectorIgnoresNormalTicksAndDetectsSleepGap() {
    var detector = HostWakeDetector(minimumGap: 5)
    let start = Date(timeIntervalSince1970: 1_000)

    let first = detector.observe(start)
    let normalTick = detector.observe(start.addingTimeInterval(1))
    let wake = detector.observe(start.addingTimeInterval(301))
    let nextTick = detector.observe(start.addingTimeInterval(302))

    #expect(!first)
    #expect(!normalTick)
    #expect(wake)
    #expect(!nextTick)
}
