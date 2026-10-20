import Darwin
import Foundation

enum EmbeddedProcessReaper {
    /// Stops stale edge-owned children before a new managed process is started.
    static func reapStale(
        binary: String,
        workDir: String,
        pidFile: String,
        exceptPID: pid_t? = nil
    ) {
        reapPIDFile(pidFile, binary: binary, workDir: workDir, exceptPID: exceptPID)
        reapMatchingCommandLines(binary: binary, workDir: workDir, exceptPID: exceptPID)
    }

    /// Counts live processes that match an edge service work directory.
    static func countOrphans(binary: String, workDir: String, exceptPID: pid_t?) -> Int {
        matchingPIDs(binary: binary, workDir: workDir, exceptPID: exceptPID).count
    }

    private static func reapPIDFile(
        _ path: String, binary: String, workDir: String, exceptPID: pid_t?
    ) {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return }
        guard let pid = Int32(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 1 else {
            return
        }
        // A stale PID file may refer to an unrelated process after PID reuse.
        guard let arguments = processArguments(pid: pid),
              matches(arguments: arguments, binary: binary, workDir: workDir) else { return }
        terminate(pid: pid, exceptPID: exceptPID)
    }

    private static func reapMatchingCommandLines(
        binary: String,
        workDir: String,
        exceptPID: pid_t?
    ) {
        for pid in matchingPIDs(binary: binary, workDir: workDir, exceptPID: exceptPID) {
            terminate(pid: pid, exceptPID: exceptPID)
        }
    }

    private static func matchingPIDs(
        binary: String,
        workDir: String,
        exceptPID: pid_t?
    ) -> [pid_t] {
        return listProcesses().filter { entry in
            if let exceptPID, entry.pid == exceptPID { return false }
            return matches(arguments: entry.arguments, binary: binary, workDir: workDir)
        }.map(\.pid)
    }

    static func matches(arguments: [String], binary: String, workDir: String) -> Bool {
        let binaryBase = URL(fileURLWithPath: binary).lastPathComponent
        guard !binaryBase.isEmpty, !workDir.isEmpty else { return false }
        // Include the interpreter's script argument, but never match an arbitrary
        // later argument or environment value containing the service name.
        guard arguments.prefix(3).contains(where: {
            URL(fileURLWithPath: $0).lastPathComponent == binaryBase
        }) else { return false }
        let directory = workDir.hasSuffix("/") ? String(workDir.dropLast()) : workDir
        return arguments.contains { $0 == directory || $0.hasPrefix(directory + "/") }
    }

    private static func terminate(pid: pid_t, exceptPID: pid_t?) {
        guard pid > 1, pid != exceptPID else { return }
        guard kill(pid, 0) == 0 else { return }
        _ = kill(pid, SIGTERM)
        let deadline = Date().addingTimeInterval(3)
        while Date() < deadline {
            if kill(pid, 0) != 0 { return }
            Thread.sleep(forTimeInterval: 0.05)
        }
        _ = kill(pid, SIGKILL)
    }

    private struct ProcessEntry {
        let pid: pid_t
        let arguments: [String]
    }

    private static func listProcesses() -> [ProcessEntry] {
        var length: size_t = 0
        var mib: [Int32] = [CTL_KERN, KERN_PROC, KERN_PROC_ALL, 0]
        guard sysctl(&mib, u_int(mib.count), nil, &length, nil, 0) == 0, length > 0 else {
            return []
        }
        let count = Int(length) / MemoryLayout<kinfo_proc>.stride
        var processes = [kinfo_proc](repeating: kinfo_proc(), count: count)
        guard sysctl(&mib, u_int(mib.count), &processes, &length, nil, 0) == 0 else {
            return []
        }
        return processes.compactMap { process in
            let pid = process.kp_proc.p_pid
            guard pid > 1 else { return nil }
            guard let arguments = processArguments(pid: pid) else { return nil }
            return ProcessEntry(pid: pid, arguments: arguments)
        }
    }

    private static func processArguments(pid: pid_t) -> [String]? {
        var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
        var size: Int = 0
        guard sysctl(&mib, u_int(mib.count), nil, &size, nil, 0) == 0, size > 0 else {
            return nil
        }
        var buffer = [UInt8](repeating: 0, count: size)
        return buffer.withUnsafeMutableBytes { raw in
            guard sysctl(&mib, u_int(mib.count), raw.baseAddress, &size, nil, 0) == 0 else {
                return nil
            }
            return parseArguments(Data(bytes: raw.baseAddress!, count: size))
        }
    }

    /// KERN_PROCARGS2: argc, executable path, NUL padding, argc arguments, environment.
    /// argc is an argument count, not the byte length of the argument buffer.
    static func parseArguments(_ data: Data) -> [String]? {
        let headerSize = MemoryLayout<Int32>.size
        guard data.count > headerSize else { return nil }
        let argc = data.withUnsafeBytes { $0.loadUnaligned(as: Int32.self) }
        guard argc > 0, Int(argc) <= data.count - headerSize else { return nil }
        var offset = headerSize
        func readString() -> String? {
            guard offset < data.count,
                  let end = data[offset...].firstIndex(of: 0),
                  let string = String(data: data[offset..<end], encoding: .utf8) else { return nil }
            offset = end + 1
            return string
        }
        guard let executable = readString(), !executable.isEmpty else { return nil }
        while offset < data.count && data[offset] == 0 { offset += 1 }
        var arguments = [executable]
        for _ in 0..<argc {
            guard let argument = readString() else { return nil }
            arguments.append(argument)
        }
        return arguments
    }
}
