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
        reapPIDFile(pidFile, exceptPID: exceptPID)
        reapMatchingCommandLines(binary: binary, workDir: workDir, exceptPID: exceptPID)
    }

    /// Counts live processes that match an edge service work directory.
    static func countOrphans(binary: String, workDir: String, exceptPID: pid_t?) -> Int {
        matchingPIDs(binary: binary, workDir: workDir, exceptPID: exceptPID).count
    }

    private static func reapPIDFile(_ path: String, exceptPID: pid_t?) {
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return }
        guard let pid = Int32(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 1 else {
            return
        }
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
        let binaryBase = URL(fileURLWithPath: binary).lastPathComponent
        guard !binaryBase.isEmpty else { return [] }
        return listProcesses().filter { entry in
            if let exceptPID, entry.pid == exceptPID { return false }
            guard entry.command.contains(binaryBase) else { return false }
            return entry.command.contains(workDir)
        }.map(\.pid)
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
        let command: String
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
            guard let command = processArguments(pid: pid) else { return nil }
            return ProcessEntry(pid: pid, command: command)
        }
    }

    private static func processArguments(pid: pid_t) -> String? {
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
            let data = Data(bytes: raw.baseAddress!, count: size)
            guard data.count > MemoryLayout<Int32>.size else { return nil }
            let argsSize = data.withUnsafeBytes {
                $0.load(as: Int32.self)
            }
            let argsStart = MemoryLayout<Int32>.size
            let argsEnd = min(data.count, argsStart + Int(argsSize))
            let argsData = data.subdata(in: argsStart ..< argsEnd)
            let pieces = argsData.split(separator: 0).compactMap {
                String(bytes: $0, encoding: .utf8)
            }.filter { !$0.isEmpty }
            guard !pieces.isEmpty else { return nil }
            return pieces.joined(separator: " ")
        }
    }
}
