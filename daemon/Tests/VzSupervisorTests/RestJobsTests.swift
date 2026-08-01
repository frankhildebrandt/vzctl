import Foundation
import Testing
@testable import VzSupervisor

struct RestJobsTests {
    @Test func userToolPathPrependsHomebrewAndLocalBin() {
        let path = RestJobRunner.userToolPath(existing: "/usr/bin:/bin:/usr/sbin:/sbin")
        let parts = path.split(separator: ":").map(String.init)
        #expect(parts.contains("/opt/homebrew/bin"))
        #expect(parts.contains("/usr/local/bin"))
        #expect(parts.contains("\(NSHomeDirectory())/.local/bin"))
        #expect(parts.firstIndex(of: "/opt/homebrew/bin")! < parts.firstIndex(of: "/usr/bin")!)
    }

    @Test func userToolPathDedupesExistingEntries() {
        let path = RestJobRunner.userToolPath(
            existing: "/opt/homebrew/bin:/usr/bin:/opt/homebrew/bin"
        )
        let homebrewCount = path.split(separator: ":").filter { $0 == "/opt/homebrew/bin" }.count
        #expect(homebrewCount == 1)
    }

    @Test func jobEnvironmentSetsStateDirProgressAndEnrichedPath() {
        let state = URL(fileURLWithPath: "/tmp/vzctl-job-env", isDirectory: true)
        let env = RestJobRunner.jobEnvironment(
            stateDirectory: state,
            overrides: ["FOO": "bar"],
            processEnvironment: ["PATH": "/usr/bin:/bin", "HOME": "/Users/test"]
        )
        #expect(env["VZCTL_STATE_DIR"] == state.path)
        #expect(env["VZCTL_PROGRESS"] == "1")
        #expect(env["FOO"] == "bar")
        #expect(env["PATH"]?.contains("/opt/homebrew/bin") == true)
        #expect(env["PATH"]?.contains("/usr/bin") == true)
    }

    @Test func cappedLogAppendingKeepsNewestLines() {
        let existing = (1 ... 3).map { "old-\($0)" }
        let added = (1 ... 5).map { "new-\($0)" }
        let result = RestJobRunner.cappedLogAppending(existing: existing, lines: added, limit: 5)
        #expect(result == Array((existing + added).suffix(5)))
    }

    @Test func lineAccumulatorSplitsChunksAndFlushesTail() {
        let box = StringBox()
        let acc = LineAccumulator { line in
            box.append(line)
        }
        acc.append(Data("Baking via builder".utf8))
        #expect(box.snapshot().isEmpty)
        acc.append(Data(" VM…\nStarting builder".utf8))
        #expect(box.snapshot() == ["Baking via builder VM…"])
        acc.append(Data(" VM…\n".utf8))
        #expect(box.snapshot() == ["Baking via builder VM…", "Starting builder VM…"])
        acc.append(Data("done".utf8))
        acc.flush()
        #expect(box.snapshot() == ["Baking via builder VM…", "Starting builder VM…", "done"])
    }

    @Test func restJobJsonIncludesLog() {
        let job = RestJob(
            id: "abc",
            kind: "image.bake",
            status: .running,
            createdAt: "2026-01-01T00:00:00Z",
            updatedAt: "2026-01-01T00:00:01Z",
            result: nil,
            error: nil,
            log: ["line-a", "line-b"]
        )
        guard case let .object(obj) = job.json,
              case let .array(log)? = obj["log"]
        else {
            Issue.record("expected log array in job json")
            return
        }
        #expect(log.count == 2)
        #expect(log[0] == .string("line-a"))
        #expect(log[1] == .string("line-b"))
    }
}
