import Darwin
import Foundation
import Testing
import VzDaemonKit
@testable import VzEdge

@Suite(.serialized)
struct EmbeddedProcessManagerTests {
    @Test func unchangedConfigurationKeepsChildAndChangedConfigurationReplacesIt() throws {
        let fixture = try ProcessFixture()
        defer { fixture.cleanup() }
        let manager = EmbeddedProcessManager()
        defer { manager.shutdown() }
        let firstSpec = try fixture.spec(config: "first")
        let first = try #require(try manager.reconcile([firstSpec]).first)
        let firstPID = try pid(first)
        for _ in 0..<3 {
            let repeatedSpec = try fixture.spec(config: "first")
            let repeated = try #require(try manager.reconcile([repeatedSpec]).first)
            #expect(try pid(repeated) == firstPID)
            #expect(repeated["restarts"] == .number(0))
        }
        let second = try #require(try manager.reconcile([fixture.spec(config: "second")]).first)
        #expect(try pid(second) != firstPID)
        #expect(kill(firstPID, 0) != 0)
        #expect(fixture.openLogDescriptors() == 0)
    }

    @Test func readinessTimeoutStopsSilentChildWithoutReadingLiveTerminationStatus() throws {
        let fixture = try ProcessFixture()
        defer { fixture.cleanup() }
        let manager = EmbeddedProcessManager()
        defer { manager.shutdown() }
        let spec = try fixture.spec(config: "silent", readiness: .init(host: "invalid", port: 1))
        do {
            _ = try manager.ensure(spec)
            Issue.record("A child without a listening socket must fail readiness")
        } catch {
            #expect(String(describing: error).contains("readiness timed out"))
        }
        let startedPID = try #require(Int32(
            String(contentsOf: fixture.root.appendingPathComponent("started.pid"), encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        ))
        #expect(kill(startedPID, 0) != 0)
        #expect(manager.statusAll().first?["running"] == .bool(false))
        #expect(fixture.openLogDescriptors() == 0)
    }

    @Test func repeatedStartAndStopReleaseParentLogDescriptors() throws {
        let fixture = try ProcessFixture()
        defer { fixture.cleanup() }
        let manager = EmbeddedProcessManager()
        defer { manager.shutdown() }
        let spec = try fixture.spec(config: "same")
        for _ in 0..<5 {
            _ = try manager.ensure(spec)
            #expect(fixture.openLogDescriptors() == 0)
            manager.stop(name: spec.name)
            #expect(fixture.openLogDescriptors() == 0)
        }
    }

    @Test func validatorDrainsOutputLargerThanPipeBuffer() throws {
        let fixture = try ProcessFixture(verboseValidator: true)
        defer { fixture.cleanup() }
        let manager = EmbeddedProcessManager()
        defer { manager.shutdown() }
        let result = try manager.ensure(fixture.spec(config: "verbose"))
        #expect(result["running"] == .bool(true))
    }

    @Test func validatorTimeoutReapsChildWithoutStartingService() throws {
        let fixture = try ProcessFixture(hangingValidator: true)
        defer { fixture.cleanup() }
        let manager = EmbeddedProcessManager(validatorTimeout: 1)
        defer { manager.shutdown() }
        let started = ProcessInfo.processInfo.systemUptime
        do {
            _ = try manager.ensure(fixture.spec(config: "hang"))
            Issue.record("A hanging validator must time out")
        } catch {
            #expect(String(describing: error).contains("caddy validation timed out"))
        }
        #expect(ProcessInfo.processInfo.systemUptime - started < 5)
        let validatorPID = try #require(Int32(
            String(contentsOf: fixture.root.appendingPathComponent("validator.pid"), encoding: .utf8)
                .trimmingCharacters(in: .whitespacesAndNewlines)
        ))
        #expect(kill(validatorPID, 0) != 0)
        #expect(!FileManager.default.fileExists(atPath: fixture.root.appendingPathComponent("started.pid").path))
        #expect(manager.statusAll().isEmpty)
    }
}

private func pid(_ status: [String: JSONValue]) throws -> pid_t {
    guard case let .number(value) = status["pid"] else {
        throw NSError(domain: "MissingProcessID", code: 1)
    }
    return pid_t(value)
}

private struct ProcessFixture {
    let root: URL

    init(verboseValidator: Bool = false, hangingValidator: Bool = false) throws {
        root = FileManager.default.temporaryDirectory
            .appendingPathComponent("vzctl-child-test-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let script = """
        #!/bin/sh
        if [ "$1" = validate ]; then
          \(hangingValidator ? "echo $$ > validator.pid; exec /bin/sleep 30" : ":")
          \(verboseValidator ? "/usr/bin/head -c 262144 /dev/zero >&2" : ":")
          exit 0
        fi
        echo $$ > started.pid
        exec /bin/sleep 30
        """
        let binary = root.appendingPathComponent("caddy")
        try script.write(to: binary, atomically: true, encoding: .utf8)
        _ = chmod(binary.path, 0o700)
    }

    func spec(
        config: String, readiness: EmbeddedProcessManager.Readiness? = nil
    ) throws -> EmbeddedProcessManager.Spec {
        let configPath = try EdgeServiceConfiguration.write(
            config, directory: root, prefix: "Caddyfile"
        )
        return .init(
            kind: "caddy", project: "test", name: "caddy-test",
            binary: root.appendingPathComponent("caddy").path,
            arguments: ["run", "--config", configPath, "--adapter", "caddyfile"],
            workDir: root.path, pidFile: root.appendingPathComponent("caddy.pid").path,
            env: [:], readiness: readiness
        )
    }

    func openLogDescriptors() -> Int {
        let logName = root.appendingPathComponent("caddy-test.log").lastPathComponent
        let descriptors = (try? FileManager.default.contentsOfDirectory(atPath: "/dev/fd")) ?? []
        return descriptors.compactMap(Int32.init).filter { descriptor in
            var path = [CChar](repeating: 0, count: Int(MAXPATHLEN))
            guard fcntl(descriptor, F_GETPATH, &path) == 0 else { return false }
            let value = String(decoding: path.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
            return value.contains(root.lastPathComponent) && value.hasSuffix(logName)
        }.count
    }

    func cleanup() { try? FileManager.default.removeItem(at: root) }
}
