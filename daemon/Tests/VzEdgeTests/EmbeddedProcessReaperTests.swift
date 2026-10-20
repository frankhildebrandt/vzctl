import Foundation
import Testing
@testable import VzEdge

@Test func stalePIDFileDoesNotTerminateUnrelatedProcess() throws {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("vzctl-reaper-identity-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/bin/sleep")
    process.arguments = ["30"]
    try process.run()
    defer {
        if process.isRunning { process.terminate() }
        process.waitUntilExit()
    }
    let pidFile = root.appendingPathComponent("caddy.pid")
    try "\(process.processIdentifier)\n".write(to: pidFile, atomically: true, encoding: .utf8)
    EmbeddedProcessReaper.reapStale(
        binary: "/opt/homebrew/bin/caddy", workDir: root.path, pidFile: pidFile.path
    )
    #expect(process.isRunning)
}

@Test func processArgumentsDecodeArgCountAndExcludeEnvironment() {
    let arguments = [
        "/opt/homebrew/bin/caddy", "run", "--config",
        "/tmp/edge/ingress/Caddyfile.abcdef", "--adapter", "caddyfile",
    ]
    let data = processArgumentsBuffer(
        executable: "/opt/homebrew/bin/caddy", arguments: arguments,
        environment: ["SECRET=never included", "OTHER=/tmp/sibling"]
    )
    #expect(EmbeddedProcessReaper.parseArguments(data) == ["/opt/homebrew/bin/caddy"] + arguments)
    #expect(EmbeddedProcessReaper.matches(
        arguments: EmbeddedProcessReaper.parseArguments(data) ?? [],
        binary: "/opt/homebrew/bin/caddy", workDir: "/tmp/edge/ingress"
    ))
}

@Test func processArgumentsKeepEmptyArgumentAndRejectTruncation() {
    let data = processArgumentsBuffer(executable: "/bin/tool", arguments: ["tool", "", "last"])
    #expect(EmbeddedProcessReaper.parseArguments(data) == ["/bin/tool", "tool", "", "last"])
    #expect(EmbeddedProcessReaper.parseArguments(Data(data.dropLast())) == nil)
    #expect(EmbeddedProcessReaper.parseArguments(Data([0, 0, 0])) == nil)
}

@Test func orphanMatchingRequiresBinaryAndDirectoryBoundary() {
    let arguments = ["/opt/bin/caddy", "caddy", "run", "--config", "/tmp/ingress/ab/Caddyfile"]
    #expect(!EmbeddedProcessReaper.matches(
        arguments: arguments, binary: "/opt/bin/caddy", workDir: "/tmp/ingress/a"
    ))
    #expect(!EmbeddedProcessReaper.matches(
        arguments: ["/bin/echo", "echo", "caddy is here", "/tmp/ingress/a/file"],
        binary: "/opt/bin/caddy", workDir: "/tmp/ingress/a"
    ))
    #expect(EmbeddedProcessReaper.matches(
        arguments: ["/bin/sh", "/bin/sh", "/tmp/caddy", "run", "--config", "/tmp/ingress/a/file"],
        binary: "/tmp/caddy", workDir: "/tmp/ingress/a"
    ))
}

private func processArgumentsBuffer(
    executable: String, arguments: [String], environment: [String] = []
) -> Data {
    var argc = Int32(arguments.count)
    var result = withUnsafeBytes(of: &argc) { Data($0) }
    result.append(contentsOf: executable.utf8)
    result.append(contentsOf: [0, 0, 0])
    for value in arguments + environment {
        result.append(contentsOf: value.utf8)
        result.append(0)
    }
    return result
}
