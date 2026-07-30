import Foundation

/// Shared daemon types — ADR 0002 / 0003.

public enum VzDaemonKit {
    public static let minMacOSMajor = 26
    public static let version = "0.0.1-alpha"
}

public struct StackID: Hashable, Sendable, Codable {
    public var raw: String
    public init(_ raw: String) { self.raw = raw }
}

public enum JournalStatus: String, Sendable, Codable {
    case pending, running, done, failed, aborted
}

public enum ApplyStep: String, Sendable, Codable, CaseIterable {
    case validate
    case acquireLease = "acquire_lease"
    case ensureNets = "ensure_nets"
    case ensureDns = "ensure_dns"
    case ensureImages = "ensure_images"
    case ensureVms = "ensure_vms"
    case attachNets = "attach_nets"
    case startHelpers = "start_helpers"
    case awaitAgents = "await_agents"
    case applyRoutesPolicies = "apply_routes_policies"
    case releaseLease = "release_lease"
}

public struct JournalEntry: Sendable, Codable {
    public var id: UUID
    public var stackID: String
    public var generation: UInt64
    public var step: ApplyStep
    public var status: JournalStatus
    public var payload: [String: String]
    public var error: String?
}

/// Exit codes — ADR 0003.
public enum VzExit: Int32 {
    case ok = 0
    case usage = 2
    case incompleteJournal = 5
    case leaseHeld = 6
    case hostTooOld = 11
}
