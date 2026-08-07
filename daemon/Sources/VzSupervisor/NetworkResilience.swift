import AppKit
import Darwin
import Foundation
import Network
import VzDaemonKit

enum NetworkResilienceState: String, Sendable {
    case healthy
    case suspended
    case offline
    case stabilizing
    case recovering
    case captive
    case conflict
    case degraded
}

struct NetworkEgressResult: Equatable, Sendable {
    var classification: String
    var phase: String
    var statusCode: Int?
    var latencyMS: Int64
    var errorCode: String?

    static let unknown = NetworkEgressResult(
        classification: "unknown",
        phase: "none",
        statusCode: nil,
        latencyMS: 0,
        errorCode: nil
    )

    var json: JSONValue {
        .object([
            "classification": .string(classification),
            "phase": .string(phase),
            "status_code": statusCode.map { .number(Double($0)) } ?? .null,
            "latency_ms": .number(Double(latencyMS)),
            "error_code": errorCode.map(JSONValue.string) ?? .null,
        ])
    }
}

struct NetworkCIDRConflict: Equatable, Sendable {
    var network: String
    var cidr: String
    var hostRoute: String
    var interface: String

    var json: JSONValue {
        .object([
            "network": .string(network),
            "cidr": .string(cidr),
            "host_route": .string(hostRoute),
            "interface": .string(interface),
        ])
    }
}

struct NetworkRecoveryResult: Sendable {
    var internalOK: Bool
    var hostEgress: NetworkEgressResult
    var networkEgress: [String: NetworkEgressResult]
    var conflicts: [NetworkCIDRConflict]
    var error: String?

    func state(pathSatisfied: Bool) -> NetworkResilienceState {
        guard pathSatisfied else { return .offline }
        if !conflicts.isEmpty { return .conflict }
        if !internalOK { return .degraded }
        if hostEgress.classification == "captive" { return .captive }
        if hostEgress.classification == "offline" { return .offline }
        if networkEgress.values.contains(where: { $0.classification == "captive" }) {
            return .captive
        }
        if networkEgress.values.contains(where: { $0.classification == "offline" }) {
            return .degraded
        }
        return .healthy
    }
}

struct NetworkResilienceStatus: Sendable {
    var state: NetworkResilienceState = .stabilizing
    var epoch: UInt64 = 0
    var pathSatisfied = false
    var interfaceTypes: [String] = []
    var lastEvent = "startup"
    var internalOK = true
    var hostEgress = NetworkEgressResult.unknown
    var networkEgress: [String: NetworkEgressResult] = [:]
    var conflicts: [NetworkCIDRConflict] = []
    var lastError: String?
    var lastTransitionAt = ISO8601DateFormatter().string(from: Date())

    var json: JSONValue {
        .object([
            "state": .string(state.rawValue),
            "epoch": .number(Double(epoch)),
            "path_satisfied": .bool(pathSatisfied),
            "interfaces": .array(interfaceTypes.map(JSONValue.string)),
            "last_event": .string(lastEvent),
            "internal_ok": .bool(internalOK),
            "host_egress": hostEgress.json,
            "network_egress": .object(networkEgress.mapValues(\.json)),
            "cidr_conflicts": .array(conflicts.map(\.json)),
            "last_error": lastError.map(JSONValue.string) ?? .null,
            "last_transition_at": .string(lastTransitionAt),
        ])
    }
}

final class NetworkResilienceController: @unchecked Sendable {
    typealias Probe = @Sendable () -> NetworkRecoveryResult
    typealias Fallback = @Sendable (NetworkRecoveryResult) -> Bool
    typealias Event = @Sendable (_ type: String, _ data: [String: JSONValue]) -> Void

    private let lock = NSLock()
    private let worker = DispatchQueue(label: "vzctl.network-resilience", qos: .utility)
    private let debounce: TimeInterval
    private let recoveryBudget: TimeInterval
    private let passiveRetry: TimeInterval
    private let probe: Probe
    private let fallback: Fallback
    private let event: Event
    private var status = NetworkResilienceStatus()
    private var pathFingerprint = ""
    private var generation: UInt64 = 0
    private var fallbackAttemptedEpoch: UInt64?
    private var stopped = false
    private var monitor: NWPathMonitor?
    private var sleepObserver: NSObjectProtocol?
    private var wakeObserver: NSObjectProtocol?

    init(
        debounce: TimeInterval = 2,
        recoveryBudget: TimeInterval = 30,
        passiveRetry: TimeInterval = 10,
        probe: @escaping Probe,
        fallback: @escaping Fallback = { _ in false },
        event: @escaping Event
    ) {
        self.debounce = debounce
        self.recoveryBudget = recoveryBudget
        self.passiveRetry = passiveRetry
        self.probe = probe
        self.fallback = fallback
        self.event = event
    }

    func start() {
        let monitor = NWPathMonitor()
        monitor.pathUpdateHandler = { [weak self] path in
            self?.receive(path: path)
        }
        self.monitor = monitor
        monitor.start(queue: worker)
        sleepObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.willSleepNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            self?.willSleep()
        }
        wakeObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: nil
        ) { [weak self] _ in
            self?.didWake()
        }
    }

    func stop() {
        let observers = lock.withLock { () -> (NSObjectProtocol?, NSObjectProtocol?) in
            stopped = true
            generation &+= 1
            return (sleepObserver, wakeObserver)
        }
        monitor?.cancel()
        if let observer = observers.0 {
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
        if let observer = observers.1 {
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
    }

    func health() -> JSONValue { lock.withLock { status.json } }

    func receiveForTest(satisfied: Bool, interfaces: [String], event reason: String) {
        receive(
            satisfied: satisfied,
            interfaces: interfaces,
            reason: reason,
            fingerprint: nil
        )
    }

    func sleepForTest() { willSleep() }

    func wakeForTest() { didWake() }

    private func receive(path: NWPath) {
        var interfaces: [String] = []
        if path.usesInterfaceType(.wiredEthernet) { interfaces.append("ethernet") }
        if path.usesInterfaceType(.wifi) { interfaces.append("wifi") }
        if path.usesInterfaceType(.other) { interfaces.append("other") }
        if path.isExpensive { interfaces.append("expensive") }
        if path.isConstrained { interfaces.append("constrained") }
        let pathDetails = path.availableInterfaces
            .map { "\($0.type):\($0.index):\($0.name)" }
            .sorted()
            .joined(separator: ",")
        let fingerprint = [
            String(describing: path.status),
            path.supportsDNS ? "dns" : "no-dns",
            path.supportsIPv4 ? "v4" : "no-v4",
            path.supportsIPv6 ? "v6" : "no-v6",
            pathDetails,
        ].joined(separator: "|")
        receive(
            satisfied: path.status == .satisfied,
            interfaces: interfaces.sorted(),
            reason: "path",
            fingerprint: fingerprint
        )
    }

    private func receive(
        satisfied: Bool,
        interfaces: [String],
        reason: String,
        fingerprint explicitFingerprint: String?
    ) {
        let fingerprint = explicitFingerprint
            ?? "\(satisfied):\(interfaces.joined(separator: ","))"
        let scheduled = lock.withLock { () -> (UInt64, UInt64)? in
            guard !stopped, fingerprint != pathFingerprint else { return nil }
            pathFingerprint = fingerprint
            generation &+= 1
            status.epoch &+= 1
            status.pathSatisfied = satisfied
            status.interfaceTypes = interfaces
            status.lastEvent = reason
            transitionLocked(to: satisfied ? .stabilizing : .offline)
            return (generation, status.epoch)
        }
        guard let scheduled else { return }
        event("host.network_changed", [
            "epoch": .number(Double(scheduled.1)),
            "path_satisfied": .bool(satisfied),
            "interfaces": .array(interfaces.map(JSONValue.string)),
        ])
        guard satisfied else { return }
        scheduleRecovery(generation: scheduled.0, epoch: scheduled.1)
    }

    private func willSleep() {
        let epoch = lock.withLock { () -> UInt64 in
            generation &+= 1
            status.epoch &+= 1
            status.lastEvent = "sleep"
            transitionLocked(to: .suspended)
            return status.epoch
        }
        event("host.sleep", ["epoch": .number(Double(epoch))])
    }

    private func didWake() {
        let scheduled = lock.withLock { () -> (UInt64, UInt64, Bool) in
            generation &+= 1
            status.epoch &+= 1
            status.lastEvent = "wake"
            transitionLocked(to: status.pathSatisfied ? .stabilizing : .offline)
            return (generation, status.epoch, status.pathSatisfied)
        }
        event("host.wake", ["epoch": .number(Double(scheduled.1))])
        if scheduled.2 {
            scheduleRecovery(generation: scheduled.0, epoch: scheduled.1)
        }
    }

    private func scheduleRecovery(generation target: UInt64, epoch: UInt64) {
        worker.asyncAfter(deadline: .now() + debounce) { [weak self] in
            self?.recover(generation: target, epoch: epoch)
        }
    }

    private func recover(generation target: UInt64, epoch: UInt64) {
        let budgetNanoseconds = UInt64(max(0, recoveryBudget) * 1_000_000_000)
        let deadline = DispatchTime.now().uptimeNanoseconds &+ budgetNanoseconds
        var delay: TimeInterval = 0
        var attempt = 0
        while true {
            var active = lock.withLock {
                !stopped && generation == target && status.pathSatisfied
            }
            guard active else { return }
            if delay > 0 {
                Thread.sleep(forTimeInterval: delay)
                active = lock.withLock {
                    !stopped && generation == target && status.pathSatisfied
                }
                guard active else { return }
            }
            attempt += 1
            lock.withLock { transitionLocked(to: .recovering) }
            event("network.recovering", [
                "epoch": .number(Double(epoch)),
                "attempt": .number(Double(attempt)),
            ])
            let result = probe()
            let nextState = result.state(pathSatisfied: true)
            lock.withLock {
                status.internalOK = result.internalOK
                status.hostEgress = result.hostEgress
                status.networkEgress = result.networkEgress
                status.conflicts = result.conflicts
                status.lastError = result.error
                transitionLocked(to: nextState)
            }
            switch nextState {
            case .healthy:
                event("network.recovered", [
                    "epoch": .number(Double(epoch)),
                    "attempt": .number(Double(attempt)),
                ])
                return
            case .captive:
                event("network.degraded", [
                    "epoch": .number(Double(epoch)),
                    "state": .string(nextState.rawValue),
                ])
                schedulePassiveRetry(generation: target, epoch: epoch)
                return
            case .conflict:
                event("network.cidr_conflict", [
                    "epoch": .number(Double(epoch)),
                    "conflicts": .array(result.conflicts.map(\.json)),
                ])
                schedulePassiveRetry(generation: target, epoch: epoch)
                return
            case .offline:
                schedulePassiveRetry(generation: target, epoch: epoch)
                return
            case .degraded where DispatchTime.now().uptimeNanoseconds < deadline:
                let now = DispatchTime.now().uptimeNanoseconds
                let remaining = Double(deadline - now) / 1_000_000_000
                let nextDelay = min(max(delay * 2, 1), 8)
                if nextDelay < remaining {
                    delay = nextDelay
                    continue
                }
                if remaining > 0 { Thread.sleep(forTimeInterval: remaining) }
                let stillActive = lock.withLock {
                    !stopped && generation == target && status.pathSatisfied
                }
                guard stillActive else { return }
                finishDegraded(
                    result, generation: target, epoch: epoch, attempt: attempt
                )
                return
            default:
                finishDegraded(
                    result, generation: target, epoch: epoch, attempt: attempt
                )
                return
            }
        }
    }

    private func finishDegraded(
        _ result: NetworkRecoveryResult,
        generation target: UInt64,
        epoch: UInt64,
        attempt: Int
    ) {
        let mayAttemptFallback = lock.withLock { () -> Bool in
            guard fallbackAttemptedEpoch != epoch else { return false }
            fallbackAttemptedEpoch = epoch
            return true
        }
        if mayAttemptFallback, fallback(result) {
            let recovered = probe()
            let recoveredState = recovered.state(pathSatisfied: true)
            lock.withLock {
                status.internalOK = recovered.internalOK
                status.hostEgress = recovered.hostEgress
                status.networkEgress = recovered.networkEgress
                status.conflicts = recovered.conflicts
                status.lastError = recovered.error
                transitionLocked(to: recoveredState)
            }
            if recoveredState == .healthy {
                event("network.recovered", [
                    "epoch": .number(Double(epoch)),
                    "attempt": .number(Double(attempt + 1)),
                    "fallback": .bool(true),
                ])
                return
            }
        }
        event("network.degraded", [
            "epoch": .number(Double(epoch)),
            "state": .string(NetworkResilienceState.degraded.rawValue),
            "error": result.error.map(JSONValue.string) ?? .null,
        ])
        schedulePassiveRetry(generation: target, epoch: epoch)
    }

    private func schedulePassiveRetry(generation target: UInt64, epoch: UInt64) {
        worker.asyncAfter(deadline: .now() + passiveRetry) { [weak self] in
            guard let self else { return }
            let active = lock.withLock {
                !stopped && generation == target && status.pathSatisfied
            }
            guard active else { return }
            recover(generation: target, epoch: epoch)
        }
    }

    private func transitionLocked(to state: NetworkResilienceState) {
        status.state = state
        status.lastTransitionAt = ISO8601DateFormatter().string(from: Date())
    }
}

final class HostEgressProber: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
    private let lock = NSLock()
    private var redirected = false

    func probe(url: URL, timeout: TimeInterval = 5) -> NetworkEgressResult {
        let started = DispatchTime.now().uptimeNanoseconds
        let semaphore = DispatchSemaphore(value: 0)
        let box = HostProbeBox()
        let configuration = URLSessionConfiguration.ephemeral
        configuration.timeoutIntervalForRequest = timeout
        configuration.timeoutIntervalForResource = timeout
        configuration.urlCache = nil
        let session = URLSession(configuration: configuration, delegate: self, delegateQueue: nil)
        var request = URLRequest(url: url)
        request.cachePolicy = .reloadIgnoringLocalAndRemoteCacheData
        request.setValue("vzctl-network-probe/1", forHTTPHeaderField: "User-Agent")
        session.dataTask(with: request) { data, response, error in
            box.set(data: data, response: response, error: error)
            semaphore.signal()
        }.resume()
        let wait = semaphore.wait(timeout: .now() + timeout + 1)
        session.invalidateAndCancel()
        let elapsed = Int64(
            (DispatchTime.now().uptimeNanoseconds - started) / 1_000_000
        )
        guard wait == .success else {
            return NetworkEgressResult(
                classification: "offline", phase: "http", statusCode: nil,
                latencyMS: elapsed, errorCode: "timeout"
            )
        }
        let value = box.value()
        if let error = value.error as? URLError {
            return NetworkEgressResult(
                classification: "offline",
                phase: Self.phase(for: error.code),
                statusCode: nil,
                latencyMS: elapsed,
                errorCode: Self.code(for: error.code)
            )
        }
        guard let response = value.response as? HTTPURLResponse else {
            return NetworkEgressResult(
                classification: "offline", phase: "http", statusCode: nil,
                latencyMS: elapsed, errorCode: "response"
            )
        }
        let wasRedirected = lock.withLock { () -> Bool in
            defer { redirected = false }
            return redirected
        }
        let online = (200 ..< 300).contains(response.statusCode) && !wasRedirected
        return NetworkEgressResult(
            classification: online ? "online" : "captive",
            phase: "http",
            statusCode: response.statusCode,
            latencyMS: elapsed,
            errorCode: nil
        )
    }

    func urlSession(
        _ session: URLSession,
        task: URLSessionTask,
        willPerformHTTPRedirection response: HTTPURLResponse,
        newRequest request: URLRequest,
        completionHandler: @escaping (URLRequest?) -> Void
    ) {
        lock.withLock { redirected = true }
        completionHandler(nil)
    }

    private static func phase(for code: URLError.Code) -> String {
        switch code {
        case .cannotFindHost, .dnsLookupFailed: "dns"
        case .secureConnectionFailed, .serverCertificateUntrusted,
             .serverCertificateHasBadDate, .serverCertificateNotYetValid: "tls"
        case .timedOut: "http"
        default: "tcp"
        }
    }

    private static func code(for code: URLError.Code) -> String {
        switch code {
        case .timedOut: "timeout"
        case .cannotFindHost, .dnsLookupFailed: "dns"
        case .secureConnectionFailed, .serverCertificateUntrusted,
             .serverCertificateHasBadDate, .serverCertificateNotYetValid: "tls"
        default: "connect"
        }
    }
}

private final class HostProbeBox: @unchecked Sendable {
    private let lock = NSLock()
    private var stored: (Data?, URLResponse?, Error?) = (nil, nil, nil)

    func set(data: Data?, response: URLResponse?, error: Error?) {
        lock.withLock { stored = (data, response, error) }
    }

    func value() -> (data: Data?, response: URLResponse?, error: Error?) {
        lock.withLock { stored }
    }
}

struct HostRoute: Equatable, Sendable {
    var cidr: String
    var interface: String
}

enum HostRouteScanner {
    static func conflicts(
        networks: [NetworkRecord],
        routes: [HostRoute] = systemRoutes()
    ) -> [NetworkCIDRConflict] {
        networks.flatMap { network -> [NetworkCIDRConflict] in
            guard let networkCIDR = try? IPv4CIDR(network.cidr) else { return [] }
            return routes.compactMap { route in
                guard !isVzctlInterface(route.interface),
                      let routeCIDR = try? IPv4CIDR(route.cidr),
                      networkCIDR.overlaps(routeCIDR)
                else { return nil }
                return NetworkCIDRConflict(
                    network: network.name,
                    cidr: network.cidr,
                    hostRoute: route.cidr,
                    interface: route.interface
                )
            }
        }.sorted {
            ($0.network, $0.hostRoute, $0.interface) < ($1.network, $1.hostRoute, $1.interface)
        }
    }

    /// Combines interface networks with the routing table. The latter is
    /// required for VPN split routes, which are not represented by an utun
    /// interface address.
    static func systemRoutes() -> [HostRoute] {
        var routes = connectedRoutes()
        let process = Process()
        let pipe = Pipe()
        process.executableURL = URL(fileURLWithPath: "/usr/sbin/netstat")
        process.arguments = ["-rn", "-f", "inet"]
        process.standardOutput = pipe
        process.standardError = FileHandle.nullDevice
        do {
            try process.run()
            let data = pipe.fileHandleForReading.readDataToEndOfFile()
            process.waitUntilExit()
            if process.terminationStatus == 0 {
                routes.append(contentsOf: parseRouteTable(String(decoding: data, as: UTF8.self)))
            }
        } catch {
            // Connected routes still provide a useful conservative fallback.
        }
        return unique(routes)
    }

    static func parseRouteTable(_ output: String) -> [HostRoute] {
        output.split(whereSeparator: \.isNewline).compactMap { line in
            let fields = line.split(whereSeparator: \.isWhitespace).map(String.init)
            guard fields.count >= 4,
                  fields[0] != "default",
                  let cidr = routeDestinationCIDR(fields[0]),
                  !isVzctlInterface(fields[3])
            else { return nil }
            return HostRoute(cidr: cidr, interface: fields[3])
        }
    }

    static func connectedRoutes() -> [HostRoute] {
        var routes: [HostRoute] = []
        var first: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&first) == 0, let first else { return routes }
        defer { freeifaddrs(first) }
        var current: UnsafeMutablePointer<ifaddrs>? = first
        while let item = current?.pointee {
            defer { current = item.ifa_next }
            guard let address = item.ifa_addr,
                  let maskAddress = item.ifa_netmask,
                  address.pointee.sa_family == UInt8(AF_INET),
                  maskAddress.pointee.sa_family == UInt8(AF_INET),
                  let name = item.ifa_name.map({ String(cString: $0) }),
                  !isVzctlInterface(name)
            else { continue }
            let addressValue = address.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
                UInt32(bigEndian: $0.pointee.sin_addr.s_addr)
            }
            let maskValue = maskAddress.withMemoryRebound(to: sockaddr_in.self, capacity: 1) {
                UInt32(bigEndian: $0.pointee.sin_addr.s_addr)
            }
            let prefix = maskValue.nonzeroBitCount
            guard (8 ... 30).contains(prefix) else { continue }
            let networkValue = addressValue & maskValue
            var networkAddress = in_addr(s_addr: networkValue.bigEndian)
            var buffer = [CChar](repeating: 0, count: Int(INET_ADDRSTRLEN))
            guard inet_ntop(
                AF_INET,
                &networkAddress,
                &buffer,
                socklen_t(INET_ADDRSTRLEN)
            ) != nil else {
                continue
            }
            let text = String(
                decoding: buffer.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) },
                as: UTF8.self
            )
            routes.append(HostRoute(cidr: "\(text)/\(prefix)", interface: name))
        }
        return unique(routes)
    }

    private static func unique(_ routes: [HostRoute]) -> [HostRoute] {
        Array(Set(routes.map { "\($0.cidr)|\($0.interface)" })).compactMap { key in
            let fields = key.split(separator: "|", maxSplits: 1).map(String.init)
            guard fields.count == 2 else { return nil }
            return HostRoute(cidr: fields[0], interface: fields[1])
        }.sorted { ($0.cidr, $0.interface) < ($1.cidr, $1.interface) }
    }

    private static func routeDestinationCIDR(_ destination: String) -> String? {
        let pieces = destination.split(separator: "/", omittingEmptySubsequences: false)
        guard pieces.count <= 2 else { return nil }
        let octets = pieces[0].split(separator: ".", omittingEmptySubsequences: false)
        guard (1 ... 4).contains(octets.count) else { return nil }
        let values = octets.compactMap { UInt8($0) }
        guard values.count == octets.count else { return nil }
        let prefix = pieces.count == 2 ? Int(pieces[1]) : octets.count * 8
        guard let prefix, (8 ... 30).contains(prefix) else { return nil }
        let padded = values + Array(repeating: 0, count: 4 - values.count)
        let address = padded.map(String.init).joined(separator: ".")
        guard let parsed = try? IPv4CIDR("\(address)/\(prefix)") else { return nil }
        return parsed.canonical
    }

    private static func isVzctlInterface(_ name: String) -> Bool {
        name == "lo0" || name.hasPrefix("bridge") || name.hasPrefix("vmenet")
    }
}
