import Darwin
import Foundation

enum JobProcessError: Error, CustomStringConvertible {
    case timedOut, cancelled, outputLimit, drainTimeout, readFailed(Int32)

    var description: String {
        switch self {
        case .timedOut: return "job process timed out; already applied resources were not rolled back"
        case .cancelled: return "job cancelled; already applied resources were not rolled back"
        case .outputLimit: return "job stdout exceeded the output limit"
        case .drainTimeout: return "job exited but a child process kept its output pipes open"
        case let .readFailed(code): return "job pipe read failed: \(String(cString: strerror(code)))"
        }
    }
}

/// One bounded pump drains both pipes, including while terminating a child.
/// Nonblocking reads also bound completion when descendants inherit pipe handles.
enum JobProcessRunner {
    static func run(
        executable: String, arguments: [String], environment: [String: String],
        timeout: TimeInterval, stdoutLimit: Int = 1_048_576, stderrLimit: Int = 262_144,
        cancelled: () -> Bool = { false }, onStderr: (Data) -> Void = { _ in }
    ) throws -> (exitCode: Int32, stdout: String, stderr: String) {
        if cancelled() { throw JobProcessError.cancelled }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        process.environment = environment
        let out = Pipe()
        let err = Pipe()
        let handles = [out.fileHandleForReading, out.fileHandleForWriting,
                       err.fileHandleForReading, err.fileHandleForWriting]
        defer { for handle in handles { try? handle.close() } }
        process.standardOutput = out
        process.standardError = err
        try process.run()
        try? out.fileHandleForWriting.close()
        try? err.fileHandleForWriting.close()
        let descriptors = [out.fileHandleForReading.fileDescriptor, err.fileHandleForReading.fileDescriptor]
        for descriptor in descriptors {
            _ = fcntl(descriptor, F_SETFL, fcntl(descriptor, F_GETFL) | O_NONBLOCK)
        }
        var stdout = Data()
        var stderr = Data()
        var stderrTruncated = false
        var eof = [false, false]
        var failure: JobProcessError?
        let deadline = Date().addingTimeInterval(timeout)
        var terminateAt: Date?
        var exitedAt: Date?
        var buffer = [UInt8](repeating: 0, count: 16_384)
        while true {
            for index in 0..<2 where !eof[index] {
                // Fairness keeps timeout/cancellation responsive under nonstop output.
                for _ in 0..<16 {
                    let count = Darwin.read(descriptors[index], &buffer, buffer.count)
                    if count == 0 { eof[index] = true; break }
                    if count < 0 {
                        if errno == EINTR { continue }
                        if errno != EAGAIN && errno != EWOULDBLOCK {
                            failure = failure ?? .readFailed(errno)
                            eof[index] = true
                        }
                        break
                    }
                    let bytes = Data(buffer.prefix(count))
                    if index == 0 {
                        let remaining = max(0, stdoutLimit - stdout.count)
                        stdout.append(bytes.prefix(remaining))
                        if bytes.count > remaining { failure = failure ?? .outputLimit }
                    } else {
                        onStderr(bytes)
                        stderr.append(bytes)
                        if stderr.count > stderrLimit {
                            stderr = Data(stderr.suffix(stderrLimit))
                            stderrTruncated = true
                        }
                    }
                }
            }
            if cancelled() { failure = failure ?? .cancelled }
            if Date() >= deadline { failure = failure ?? .timedOut }
            if process.isRunning {
                if failure != nil {
                    if terminateAt == nil {
                        process.terminate()
                        terminateAt = Date()
                    } else if Date().timeIntervalSince(terminateAt!) >= 2 {
                        _ = Darwin.kill(process.processIdentifier, SIGKILL)
                    }
                }
            } else {
                if eof.allSatisfy({ $0 }) { break }
                if exitedAt == nil { exitedAt = Date() }
                if Date().timeIntervalSince(exitedAt!) >= 1 {
                    failure = failure ?? .drainTimeout
                    break
                }
            }
            var pollDescriptors = descriptors.enumerated().map {
                pollfd(fd: eof[$0.offset] ? -1 : $0.element, events: Int16(POLLIN | POLLHUP), revents: 0)
            }
            _ = Darwin.poll(&pollDescriptors, nfds_t(pollDescriptors.count), 25)
        }
        process.waitUntilExit()
        if let failure { throw failure }
        return (
            process.terminationStatus,
            String(decoding: stdout, as: UTF8.self),
            (stderrTruncated ? "[earlier stderr truncated]\n" : "") + String(decoding: stderr, as: UTF8.self)
        )
    }
}
