import Foundation
import Testing
import VzDaemonKit
@testable import VzSupervisor

struct RestJobExecutionTests {
    @Test func syncDrainsBothPipesPastCapacity() throws {
        let runner = makeRunner()
        let result = try runner.runSync(arguments: [
            "-c", "/usr/bin/head -c 200000 /dev/zero & /usr/bin/head -c 200000 /dev/zero >&2; wait",
        ])
        #expect(result.exitCode == 0)
        #expect(result.stdout.utf8.count == 200_000)
        #expect(result.stderr.utf8.count == 200_000)
    }

    @Test func streamingDrainsLargeStdoutAndCapsUnterminatedStderrLine() throws {
        let runner = makeRunner()
        let id = try runner.start(kind: "test", arguments: [
            "-c", "/usr/bin/head -c 200000 /dev/zero; /usr/bin/head -c 800000 /dev/zero >&2",
        ])
        let job = try completed(runner, id)
        #expect(job.status == .succeeded)
        #expect(job.log.count == 1)
        #expect(job.log.first?.utf8.count == RestJobRunner.maxLineBytes)
        #expect(job.log.first?.hasSuffix("[line truncated]") == true)
        guard case let .object(result)? = job.result, case let .string(stdout)? = result["stdout"] else {
            Issue.record("expected captured stdout")
            return
        }
        #expect(stdout.utf8.count == 200_000)
    }

    @Test func outputLimitAndTimeoutAreExplicitFailures() throws {
        let runner = makeRunner(syncTimeout: 0.2)
        #expect(throws: JobProcessError.self) {
            _ = try runner.runSync(arguments: ["-c", "exec /bin/sleep 30"])
        }
        let normalRunner = makeRunner()
        do {
            _ = try normalRunner.runSync(arguments: ["-c", "/usr/bin/head -c 2000000 /dev/zero"])
            Issue.record("oversized stdout must not become a successful truncated response")
        } catch {
            #expect(String(describing: error).contains("output limit"))
        }
    }

    @Test func stderrRetainsBoundedTailAndSplitUTF8IsPreserved() throws {
        let runner = makeRunner()
        let result = try runner.runSync(arguments: [
            "-c", "/usr/bin/head -c 800000 /dev/zero >&2; printf 'last-error' >&2",
        ])
        #expect(result.stderr.hasPrefix("[earlier stderr truncated]"))
        #expect(result.stderr.hasSuffix("last-error"))
        #expect(result.stderr.utf8.count < 263_000)
        let box = StringBox()
        let accumulator = LineAccumulator { box.append($0) }
        let bytes = Array("größer\n".utf8)
        for byte in bytes { accumulator.append(Data([byte])) }
        accumulator.flush()
        #expect(box.snapshot() == ["größer"])
    }

    @Test func retainedJobsEvictCompletedButNeverActiveJobs() throws {
        let runner = makeRunner(retainedJobs: 2, activeJobs: 2)
        let active = try runner.start(kind: "active", arguments: ["-c", "exec /bin/sleep 30"])
        defer { runner.cancel(active) }
        let old = try runner.start(kind: "old", arguments: ["-c", "printf done"])
        _ = try completed(runner, old)
        let newest = try runner.start(kind: "new", arguments: ["-c", "printf done"])
        _ = try completed(runner, newest)
        #expect(runner.get(old) == nil)
        #expect(runner.get(active) != nil)
        #expect(runner.get(newest) != nil)
        #expect(runner.cancel(active))
        #expect(try completed(runner, active).status == .failed)
    }

    @Test func admissionIs429AndCancellationIsExplicitViaREST() throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let runner = makeRunner(activeJobs: 1)
        let id = try runner.start(kind: "active", arguments: ["-c", "exec /bin/sleep 30"])
        defer { runner.cancel(id) }
        do {
            _ = try runner.start(kind: "extra", arguments: ["-c", "printf never"])
            Issue.record("admission must reject excess active work")
        } catch let error as RestRouteError {
            #expect(error.status == 429)
        }
        let database = try StateDatabase(path: root.appendingPathComponent("state.sqlite").path)
        let router = RestRouter(jobs: runner, database: database, stateDirectory: root)
        let request = RestHTTPRequest(
            method: "POST", path: "/v1/jobs/\(id)/cancel", query: [:], headers: [:], body: Data()
        )
        #expect(router.handle(request).status == 202)
        let job = try completed(runner, id)
        #expect(job.status == .failed)
        #expect(job.error?.contains("job cancelled") == true)
        #expect(router.handle(request).status == 409)
    }
}

private func makeRunner(
    retainedJobs: Int = 32, activeJobs: Int = 4, syncTimeout: TimeInterval = 5
) -> RestJobRunner {
    RestJobRunner(
        stateDirectory: FileManager.default.temporaryDirectory, vzctlPath: "/bin/sh",
        limits: .init(retainedJobs: retainedJobs, activeJobs: activeJobs, syncTimeout: syncTimeout, jobTimeout: 5)
    )
}

private func completed(_ runner: RestJobRunner, _ id: String) throws -> RestJob {
    let deadline = Date().addingTimeInterval(8)
    while Date() < deadline {
        if let job = runner.get(id), job.status == .failed || job.status == .succeeded { return job }
        Thread.sleep(forTimeInterval: 0.01)
    }
    runner.cancel(id)
    throw NSError(domain: "JobTestTimeout", code: 1)
}
