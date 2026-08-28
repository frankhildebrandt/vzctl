import Darwin
import Dispatch
import Foundation
import VzDaemonKit

private let dnsHeaderLength = 12

struct DNSZone: Equatable, Sendable {
    /// Guest-horizon A records (and shared VM records).
    let records: [String: [String]]
    /// Base names for `*.{host}.{network}.{stack}.vz.test` synthesis.
    let wildcardRecords: [String: [String]]
    /// Listener-scoped single-label names, keyed by `{stack}|{listener-network}`.
    let shortRecords: [String: [String: [String]]]
    /// Reverse IPv4 owner name → canonical host/container FQDN.
    let ptrRecords: [String: [String]]
    /// Host-horizon A records for ingress/OIDC names (loopback for Mac clients).
    let hostRecords: [String: [String]]
    /// Guest-horizon ingress overrides: host → listener network → backend VM IP.
    let ingressGuestAddresses: [String: [String: String]]
    /// Project ownership for host-managed ingress/OIDC names.
    let ingressProjects: [String: String]
    /// Octet-aligned reverse zones covering the hypernetwork CIDRs (guest
    /// horizon). PTR queries outside these zones are forwarded upstream.
    let reverseZones: Set<String>
    let zones: Set<String>
    let ttl: UInt32

    func addresses(for name: String, horizon: DNSHorizon) -> [String]? {
        let canonical = DNSZoneBuilder.canonicalName(name)
        if let ip = DNSZoneBuilder.ipv4Literal(canonical) {
            return [ip]
        }
        switch horizon {
        case .host:
            if let host = hostRecords[canonical] {
                return host
            }
            return records[canonical] ?? wildcardAddresses(for: canonical)
        case let .guest(context):
            if hostRecords[canonical] != nil {
                guard ingressProjects[canonical] == context.project else { return nil }
                if let networkIP = ingressGuestAddresses[canonical]?[context.network] {
                    return [networkIP]
                }
                return [context.hostService]
            }
            if let exact = records[canonical] ?? wildcardAddresses(for: canonical) {
                return exact
            }
            guard !canonical.contains(".") else { return nil }
            return shortRecords[DNSZoneBuilder.contextKey(
                stack: context.stack,
                network: context.network
            )]?[canonical]
        }
    }

    func ptrNames(for name: String) -> [String]? {
        ptrRecords[DNSZoneBuilder.canonicalName(name)]
    }

    func isAuthoritative(for name: String, horizon: DNSHorizon) -> Bool {
        let canonical = DNSZoneBuilder.canonicalName(name)
        if addresses(for: canonical, horizon: horizon) != nil
            || ptrRecords[canonical] != nil
            || canonical == "vz.test"
            || canonical.hasSuffix(".vz.test")
        {
            return true
        }
        guard DNSZoneBuilder.isIPv4Reverse(canonical) else { return false }
        switch horizon {
        case .host:
            // Loop guard: a stale `/etc/resolver/in-addr.arpa` may still route
            // the whole reverse tree here; forwarding via the system resolver
            // would recurse back into vz-edge (SERVFAIL loop).
            return true
        case .guest:
            return matchingReverseZone(for: canonical) != nil
        }
    }

    /// Longest reverse zone covering `name`, or nil when outside the
    /// hypernetwork reverse zones. Zones are octet-aligned
    /// (`a` / `b.a` / `c.b.a` + `.in-addr.arpa`, i.e. 3–5 labels).
    func matchingReverseZone(for name: String) -> String? {
        let labels = DNSZoneBuilder.canonicalName(name).split(separator: ".")
        guard labels.count >= 3 else { return nil }
        for take in stride(from: min(5, labels.count), through: 3, by: -1) {
            let candidate = labels.suffix(take).joined(separator: ".")
            if reverseZones.contains(candidate) { return candidate }
        }
        return nil
    }

    /// SOA owner for negative answers (enables negative caching, RFC 2308).
    func authorityZone(for name: String) -> String {
        let canonical = DNSZoneBuilder.canonicalName(name)
        if DNSZoneBuilder.isIPv4Reverse(canonical) {
            return matchingReverseZone(for: canonical) ?? "in-addr.arpa"
        }
        var best: String?
        for zone in zones where canonical == zone || canonical.hasSuffix(".\(zone)") {
            if zone.count > (best?.count ?? -1) { best = zone }
        }
        return best ?? "vz.test"
    }

    private func wildcardAddresses(for name: String) -> [String]? {
        wildcardRecords
            .filter { name != $0.key && name.hasSuffix(".\($0.key)") }
            .sorted { $0.key.count > $1.key.count }
            .first?.value
    }
}

struct DNSRuntimeRecord: Equatable, Sendable {
    let name: String
    let network: String
    let listenerNetwork: String
    let stack: String
    let project: String
    let ip: String
}

struct DNSGuestContext: Equatable, Sendable {
    let network: String
    let project: String?
    let stack: String?
    let gateway: String
    let hostService: String
}

enum DNSHorizon: Equatable, Sendable, CustomStringConvertible {
    case host
    case guest(DNSGuestContext)

    var description: String {
        switch self {
        case .host: return "host"
        case let .guest(context): return "guest:\(context.network)"
        }
    }
}

enum DNSZoneBuilder {
    static let serviceLabel = "vzctl.dev/dns-services"

    /// Host-owned ingress/OIDC names (without trailing dot), e.g. `auth.svc.edge-dmz.vz.test`.
    static func build(
        snapshot: NetworkSnapshot,
        ttl: UInt32,
        hostServices: [String] = [],
        ingressGuestAddresses: [String: [String: String]] = [:],
        runtimeRecords: [DNSRuntimeRecord] = []
    ) -> DNSZone {
        let networks = Dictionary(uniqueKeysWithValues: snapshot.networks.map { ($0.name, $0) })
        var records: [String: Set<String>] = [:]
        var wildcardRecords: [String: Set<String>] = [:]
        var shortRecords: [String: [String: Set<String>]] = [:]
        var ptrRecords: [String: Set<String>] = [:]
        var zones: Set<String> = []
        for attachment in snapshot.attachments {
            guard let network = networks[attachment.networkName],
                  network.runtimeState == "active",
                  let project = attachment.project ?? network.project,
                  let vm = dnsLabel(vmDNSLabel(attachment.vmID)),
                  let net = dnsLabel(attachment.networkName),
                  let stack = dnsLabel(project),
                  ipv4Bytes(attachment.ip) != nil
            else {
                continue
            }

            let zone = "\(stack).vz.test"
            zones.insert(zone)
            let fqdn = "\(vm).\(net).\(zone)"
            insertMachineRecord(
                name: vm,
                fqdn: fqdn,
                ip: attachment.ip,
                listenerNetwork: attachment.networkName,
                stack: stack,
                records: &records,
                wildcardRecords: &wildcardRecords,
                shortRecords: &shortRecords,
                ptrRecords: &ptrRecords
            )

            let rawServices = attachment.labels[serviceLabel]
                ?? network.labels[serviceLabel]
                ?? ""
            for rawService in rawServices.split(separator: ",") {
                guard let service = dnsLabel(String(rawService).trimmingCharacters(in: .whitespaces))
                else {
                    continue
                }
                records["\(service).svc.\(zone)", default: []].insert(attachment.ip)
            }
        }

        for runtime in runtimeRecords {
            guard let name = dnsLabel(runtime.name), name != "svc",
                  let network = dnsLabel(runtime.network),
                  let stack = dnsLabel(runtime.stack),
                  ipv4Bytes(runtime.ip) != nil
            else { continue }
            let zone = "\(stack).vz.test"
            let fqdn = "\(name).\(network).\(zone)"
            zones.insert(zone)
            insertMachineRecord(
                name: name,
                fqdn: fqdn,
                ip: runtime.ip,
                listenerNetwork: runtime.listenerNetwork,
                stack: stack,
                records: &records,
                wildcardRecords: &wildcardRecords,
                shortRecords: &shortRecords,
                ptrRecords: &ptrRecords
            )
        }

        var reverseZones: Set<String> = []
        for network in snapshot.networks where network.runtimeState == "active" {
            reverseZones.formUnion(self.reverseZones(forCIDR: network.cidr))
        }

        var hostRecords: [String: [String]] = [:]
        var ingressProjects: [String: String] = [:]
        for raw in hostServices {
            let name = canonicalName(raw)
            guard !name.isEmpty else { continue }
            // Resolve project for split-horizon gateway mapping.
            let parts = name.split(separator: ".")
            var project: String?
            // short.svc.project.vz.test → project at count-3
            if parts.count >= 4,
               parts[parts.count - 2] == "vz",
               parts[parts.count - 1] == "test",
               parts[parts.count - 3] != "svc"
            {
                project = String(parts[parts.count - 3])
            } else if parts.count >= 5,
                      parts[parts.count - 2] == "vz",
                      parts[parts.count - 1] == "test",
                      parts[parts.count - 4] == "svc"
            {
                project = String(parts[parts.count - 3])
            }

            // Split horizon: Mac `/etc/resolver` → loopback (host cannot dial its
            // own bridge `.1` alias: EHOSTDOWN). Guests → host-service `.1`
            // (vmnet drops guest TCP to `.0`; never 127.0.0.1 — that would be the
            // guest's own loopback if mDNSResponder steals the query).
            hostRecords[name] = ["127.0.0.1"]
            if let project { ingressProjects[name] = project }
            zones.insert(zoneName(from: name))
        }

        return DNSZone(
            records: records.mapValues { $0.sorted() },
            wildcardRecords: wildcardRecords.mapValues { $0.sorted() },
            shortRecords: shortRecords.mapValues { names in
                names.mapValues { $0.sorted() }
            },
            ptrRecords: ptrRecords.mapValues { $0.sorted() },
            hostRecords: hostRecords,
            ingressGuestAddresses: ingressGuestAddresses,
            ingressProjects: ingressProjects,
            reverseZones: reverseZones,
            zones: zones.filter { !$0.isEmpty },
            ttl: min(30, max(5, ttl))
        )
    }

    /// Octet-aligned IPv4 reverse zones covering `cidr`, e.g. `10.0.80.0/24` →
    /// `80.0.10.in-addr.arpa`. Prefixes between octet boundaries expand into
    /// multiple zones (a /23 claims two /24 zones); prefixes longer than /24
    /// claim the enclosing /24 zone.
    static func reverseZones(forCIDR cidr: String) -> [String] {
        guard let parsed = try? IPv4CIDR(cidr) else { return [] }
        let octets = min(3, (parsed.prefixLength + 7) / 8)
        let zoneBits = octets * 8
        let count = parsed.prefixLength >= zoneBits
            ? 1
            : 1 << (zoneBits - parsed.prefixLength)
        let stride = UInt32(1) << UInt32(32 - zoneBits)
        return (0 ..< count).map { index in
            let base = parsed.network &+ UInt32(index) &* stride
            let name = (0 ..< octets)
                .map { String((base >> UInt32(24 - 8 * $0)) & 0xFF) }
                .reversed()
                .joined(separator: ".")
            return "\(name).in-addr.arpa"
        }
    }

    static func canonicalName(_ name: String) -> String {
        name.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
    }

    static func contextKey(stack: String?, network: String) -> String {
        "\(stack ?? "")|\(canonicalName(network))"
    }

    private static func insertMachineRecord(
        name: String,
        fqdn: String,
        ip: String,
        listenerNetwork: String,
        stack: String,
        records: inout [String: Set<String>],
        wildcardRecords: inout [String: Set<String>],
        shortRecords: inout [String: [String: Set<String>]],
        ptrRecords: inout [String: Set<String>]
    ) {
        records[fqdn, default: []].insert(ip)
        wildcardRecords[fqdn, default: []].insert(ip)
        let key = contextKey(stack: stack, network: listenerNetwork)
        shortRecords[key, default: [:]][name, default: []].insert(ip)
        if let reverse = reverseName(for: ip) {
            ptrRecords[reverse, default: []].insert(fqdn)
        }
    }

    static func reverseName(for ip: String) -> String? {
        guard let bytes = ipv4Bytes(ip) else { return nil }
        return bytes.reversed().map(String.init).joined(separator: ".") + ".in-addr.arpa"
    }

    /// Dotted-quad IPv4 used as a DNS name (`192.168.171.199`), not a hostname.
    static func ipv4Literal(_ name: String) -> String? {
        let canonical = canonicalName(name)
        let labels = canonical.split(separator: ".", omittingEmptySubsequences: false)
        guard labels.count == 4, ipv4Bytes(canonical) != nil else { return nil }
        return canonical
    }

    /// IPv4 reverse tree. The host horizon answers the whole tree locally as a
    /// loop guard (a stale `/etc/resolver/in-addr.arpa` would bounce unknown
    /// PTRs back into vz-edge); the guest horizon only claims the hypernetwork
    /// reverse zones and forwards everything else upstream.
    static func isIPv4Reverse(_ name: String) -> Bool {
        let canonical = canonicalName(name)
        return canonical == "in-addr.arpa" || canonical.hasSuffix(".in-addr.arpa")
    }

    private static func zoneName(from host: String) -> String {
        let parts = host.split(separator: ".")
        guard parts.count >= 3 else { return host }
        return parts.suffix(3).joined(separator: ".")
    }

    private static func dnsLabel(_ value: String) -> String? {
        let label = value.lowercased()
        guard !label.isEmpty,
              label.utf8.count <= 63,
              label.first?.isLetter == true || label.first?.isNumber == true,
              label.last?.isLetter == true || label.last?.isNumber == true,
              label.allSatisfy({ $0.isLetter || $0.isNumber || $0 == "-" })
        else {
            return nil
        }
        return label
    }

    /// Runtime VM IDs may be namespaced as `{project}/{vm}`; DNS uses the basename.
    static func vmDNSLabel(_ value: String) -> String {
        if let slash = value.lastIndex(of: "/") {
            let basename = String(value[value.index(after: slash)...])
            return basename.isEmpty ? value : basename
        }
        return value
    }
}

struct DNSConfiguration: Sendable {
    let hostAddress: String
    let hostPort: UInt16
    /// Public port configured as nameserver in Guests.
    let guestPort: UInt16
    /// Exclusive socket port behind the PF redirect for privileged Guest DNS.
    let guestBackendPort: UInt16
    let ttl: UInt32
    let upstream: String

    init(
        hostAddress: String,
        hostPort: UInt16,
        guestPort: UInt16,
        guestBackendPort: UInt16? = nil,
        ttl: UInt32,
        upstream: String
    ) {
        self.hostAddress = hostAddress
        self.hostPort = hostPort
        self.guestPort = guestPort
        self.guestBackendPort = guestBackendPort
            ?? (DnsBind.needsPrivilege(port: guestPort)
                ? DnsBind.defaultGuestDNSBackendPort : guestPort)
        self.ttl = ttl
        self.upstream = upstream
    }

    static func environment(
        _ values: [String: String] = ProcessInfo.processInfo.environment
    ) -> DNSConfiguration {
        let guestPort = port(values["VZCTL_DNS_GUEST_PORT"])
            ?? DnsBind.defaultGuestDNSPort
        return DNSConfiguration(
            hostAddress: values["VZCTL_DNS_HOST"] ?? "127.0.0.1",
            hostPort: port(values["VZCTL_DNS_PORT"]) ?? 15_353,
            guestPort: guestPort,
            guestBackendPort: port(values["VZCTL_DNS_GUEST_BACKEND_PORT"]),
            ttl: UInt32(values["VZCTL_DNS_TTL"] ?? "") ?? 15,
            upstream: values["VZCTL_DNS_UPSTREAM"] ?? "system"
        )
    }

    private static func port(_ value: String?) -> UInt16? {
        guard let parsed = value.flatMap(UInt16.init), parsed > 0 else { return nil }
        return parsed
    }
}

struct DNSHealth: Sendable {
    let ok: Bool
    let listeners: [String]
    let records: Int
    let zones: Int
    let ttl: UInt32
    let upstream: String
    let lastError: String?

    var json: JSONValue {
        .object([
            "ok": .bool(ok),
            "listeners": .array(listeners.map(JSONValue.string)),
            "records": .number(Double(records)),
            "zones": .number(Double(zones)),
            "ttl": .number(Double(ttl)),
            "upstream": .string(upstream),
            "last_error": lastError.map(JSONValue.string) ?? .null,
        ])
    }
}

private struct DNSPeer: Sendable {
    let storage: sockaddr_storage
    let length: socklen_t
}

private struct DNSListener {
    let descriptor: Int32
    let source: DispatchSourceRead
    /// Fixed at bind time so concurrent DispatchSource handlers cannot mis-classify
    /// guest `.0:53` sockets as the host horizon (which maps `*.svc` → 127.0.0.1).
    let horizon: DNSHorizon
}

struct DNSQuestion {
    let name: String
    let type: UInt16
    let dnsClass: UInt16
    let endOffset: Int
}

final class DNSServer: @unchecked Sendable {
    typealias SystemResolve = @Sendable (Data, DNSQuestion) -> Data?

    private let configuration: DNSConfiguration
    private let systemResolve: SystemResolve
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "dev.vzctl.dns")
    private let resolveQueue = DispatchQueue(label: "dev.vzctl.dns.resolve", attributes: .concurrent)
    /// Caps concurrent blocking upstream resolves so a query burst cannot
    /// exhaust the GCD thread pool and starve authoritative answers.
    private let resolveSlots = DispatchSemaphore(value: 32)
    private var zone: DNSZone
    private var listeners: [String: DNSListener] = [:]
    private var desiredListeners: Set<String> = []
    private var guestContexts: [DNSGuestContext] = []
    private var hostServices: [String] = []
    private var ingressGuestAddresses: [String: [String: String]] = [:]
    private var runtimeRecords: [DNSRuntimeRecord] = []
    private var lastError: String?
    private var stopped = false

    init(
        configuration: DNSConfiguration = .environment(),
        systemResolve: @escaping SystemResolve = {
            SystemDNSResolver.resolve(request: $0, question: $1)
        }
    ) {
        self.configuration = configuration
        self.systemResolve = systemResolve
        zone = DNSZone(
            records: [:],
            wildcardRecords: [:],
            shortRecords: [:],
            ptrRecords: [:],
            hostRecords: [:],
            ingressGuestAddresses: [:],
            ingressProjects: [:],
            reverseZones: [],
            zones: [],
            ttl: min(30, max(5, configuration.ttl))
        )
    }

    func setIngressHostServices(
        hosts: [String],
        guestAddresses: [String: [String: String]] = [:]
    ) {
        lock.withLock {
            hostServices = hosts.map(DNSZoneBuilder.canonicalName)
            ingressGuestAddresses = Dictionary(
                uniqueKeysWithValues: guestAddresses.map {
                    (DNSZoneBuilder.canonicalName($0.key), $0.value)
                }
            )
        }
    }

    /// Legacy host-only ingress list; guests resolve via bridge `.1` for every host.
    func setHostServices(_ names: [String]) {
        setIngressHostServices(hosts: names)
    }

    func setRuntimeRecords(_ records: [DNSRuntimeRecord]) {
        lock.withLock { runtimeRecords = records }
    }

    @discardableResult
    func reload(snapshot: NetworkSnapshot) -> DNSHealth {
        let state = lock.withLock { (hostServices, ingressGuestAddresses, runtimeRecords) }
        let nextZone = DNSZoneBuilder.build(
            snapshot: snapshot,
            ttl: configuration.ttl,
            hostServices: state.0,
            ingressGuestAddresses: state.1,
            runtimeRecords: state.2
        )
        let contexts = snapshot.networks.compactMap { network -> DNSGuestContext? in
            guard network.runtimeState == "active", !network.isDockerBackend else { return nil }
            let gateway = IPv4CIDR.gateway(for: network.cidr)
            let hostService = IPv4CIDR.hostService(for: network.cidr)
            guard !gateway.isEmpty, !hostService.isEmpty else { return nil }
            return DNSGuestContext(
                network: network.name,
                project: network.project.map(DNSZoneBuilder.canonicalName),
                stack: network.project.map(DNSZoneBuilder.canonicalName),
                gateway: gateway,
                hostService: hostService
            )
        }
        var desired = Dictionary(uniqueKeysWithValues: contexts.map {
            (endpoint($0.gateway, configuration.guestPort), DNSHorizon.guest($0))
        })
        desired[endpoint(configuration.hostAddress, configuration.hostPort)] = .host
        let desiredKeys = Set(desired.keys)

        // Retired descriptors are closed by their cancel handlers once the last
        // event handler finished; closing them inline could hand a reused FD
        // number to an in-flight handler with the wrong horizon.
        let toRebind = lock.withLock { () -> [String: DNSHorizon] in
            guard !stopped else { return [:] }
            zone = nextZone
            desiredListeners = desiredKeys
            guestContexts = contexts.sorted { $0.network < $1.network }
            lastError = nil

            for key in listeners.keys where !desiredKeys.contains(key) {
                listeners.removeValue(forKey: key)?.source.cancel()
            }
            var toRebind: [String: DNSHorizon] = [:]
            for (key, horizon) in desired {
                if let existing = listeners[key], existing.horizon == horizon {
                    continue
                }
                listeners.removeValue(forKey: key)?.source.cancel()
                toRebind[key] = horizon
            }
            return toRebind
        }

        // Bind outside the zone lock: dns-bind RPC must not block in-flight UDP handlers.
        var created: [String: DNSListener] = [:]
        var bindErrors: [String] = []
        for key in toRebind.keys.sorted() {
            guard let horizon = toRebind[key] else { continue }
            do {
                created[key] = try makeListener(endpoint: key, horizon: horizon)
            } catch {
                bindErrors.append("\(key): \(error)")
            }
        }
        lock.withLock {
            guard !stopped else {
                for listener in created.values {
                    listener.source.cancel()
                }
                return
            }
            for (key, listener) in created {
                listeners[key] = listener
            }
            if !bindErrors.isEmpty {
                lastError = bindErrors.joined(separator: "; ")
            }
        }
        return health()
    }

    func health() -> DNSHealth {
        let snapshot = lock.withLock {
            (
                listeners.keys.sorted(),
                desiredListeners,
                zone.records.values.reduce(0) { $0 + $1.count },
                zone.zones.count,
                zone.ttl,
                lastError
            )
        }
        let hostEndpoint = endpoint(configuration.hostAddress, configuration.hostPort)
        let probeError = snapshot.0.contains(hostEndpoint) ? probeHostListener() : nil
        let combinedError = [snapshot.5, probeError]
            .compactMap { $0 }
            .filter { !$0.isEmpty }
            .joined(separator: "; ")
        let lastErrorValue = combinedError.isEmpty ? nil : combinedError
        return DNSHealth(
            ok: !stopped && Set(snapshot.0) == snapshot.1 && lastErrorValue == nil,
            listeners: snapshot.0,
            records: snapshot.2,
            zones: snapshot.3,
            ttl: snapshot.4,
            upstream: configuration.upstream,
            lastError: lastErrorValue
        )
    }

    /// Debug/resolve helper: return A-addresses for both horizons from the live zone.
    func lookup(_ name: String, network: String? = nil) -> (host: [String]?, guest: [String]?) {
        let state = lock.withLock { (zone, guestContexts) }
        let contexts = state.1.filter { network == nil || $0.network == network }
        let guest = contexts
            .compactMap { state.0.addresses(for: name, horizon: .guest($0)) }
            .flatMap { $0 }
        let uniqueGuest = Array(Set(guest)).sorted()
        return (
            state.0.addresses(for: name, horizon: .host),
            uniqueGuest.isEmpty ? nil : uniqueGuest
        )
    }

    func shutdown() {
        let current: [DNSListener] = lock.withLock {
            guard !stopped else { return [] }
            stopped = true
            desiredListeners.removeAll()
            let values = Array(listeners.values)
            listeners.removeAll()
            return values
        }
        for listener in current {
            // Cancel handler owns the close (see adoptListener).
            listener.source.cancel()
        }
    }

    /// Verify the host DNS listener responds on the wire (catches stale UDP sockets).
    private func probeHostListener() -> String? {
        var lastError: String?
        for attempt in 0 ..< 3 {
            if let error = probeHostListenerOnce() {
                lastError = error
                if attempt < 2 {
                    usleep(100_000)
                }
                continue
            }
            return nil
        }
        return lastError
    }

    private func probeHostListenerOnce() -> String? {
        let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard descriptor >= 0 else {
            return "host DNS probe socket: \(String(cString: strerror(errno)))"
        }
        defer { Darwin.close(descriptor) }

        // Short probe timeout: health() runs synchronously in RPC handlers and
        // must not stall callers for seconds when the listener is dead.
        var timeout = timeval(tv_sec: 0, tv_usec: 500_000)
        setsockopt(
            descriptor,
            SOL_SOCKET,
            SO_RCVTIMEO,
            &timeout,
            socklen_t(MemoryLayout<timeval>.size)
        )

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = configuration.hostPort.bigEndian
        guard inet_pton(AF_INET, configuration.hostAddress, &address.sin_addr) == 1 else {
            return "host DNS probe address is invalid"
        }
        let connected = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard connected == 0 else {
            return "host DNS probe connect: \(String(cString: strerror(errno)))"
        }

        let request = dnsProbeQuery()
        let sent = request.withUnsafeBytes { bytes in
            Darwin.send(descriptor, bytes.baseAddress, request.count, 0)
        }
        guard sent == request.count else {
            return "host DNS probe send: \(String(cString: strerror(errno)))"
        }

        var buffer = [UInt8](repeating: 0, count: 512)
        let count = Darwin.recv(descriptor, &buffer, buffer.count, 0)
        guard count >= dnsHeaderLength else {
            return "host DNS probe receive: \(String(cString: strerror(errno)))"
        }
        return nil
    }

    private func makeListener(endpoint value: String, horizon: DNSHorizon) throws -> DNSListener {
        guard let parsed = parseEndpoint(value) else {
            throw DNSError.invalidEndpoint(value)
        }
        let bindPort: UInt16
        switch horizon {
        case .host:
            bindPort = parsed.port
        case .guest:
            bindPort = configuration.guestBackendPort
        }
        if DnsBind.needsPrivilege(port: bindPort) {
            let descriptor = try DnsBindClient.bindUDP(
                address: parsed.address,
                port: bindPort
            )
            do {
                return try adoptListener(horizon: horizon, descriptor: descriptor)
            } catch {
                Darwin.close(descriptor)
                throw error
            }
        }
        return try bindListenerLocally(
            endpoint: endpoint(parsed.address, bindPort),
            address: parsed.address,
            port: bindPort,
            horizon: horizon
        )
    }

    private func bindListenerLocally(
        endpoint value: String,
        address: String,
        port: UInt16,
        horizon: DNSHorizon
    ) throws -> DNSListener {
        let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard descriptor >= 0 else { throw DNSError.system("socket", errno) }
        do {
            // SO_REUSEADDR required alongside mDNSResponder *:53 (see DnsBind UDP note).
            var reuse: Int32 = 1
            setsockopt(
                descriptor,
                SOL_SOCKET,
                SO_REUSEADDR,
                &reuse,
                socklen_t(MemoryLayout<Int32>.size)
            )
            var addr = sockaddr_in()
            addr.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            addr.sin_family = sa_family_t(AF_INET)
            addr.sin_port = port.bigEndian
            guard inet_pton(AF_INET, address, &addr.sin_addr) == 1 else {
                throw DNSError.invalidEndpoint(value)
            }
            let result = withUnsafePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.bind(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard result == 0 else { throw DNSError.system("bind \(value)", errno) }
            return try adoptListener(horizon: horizon, descriptor: descriptor)
        } catch {
            Darwin.close(descriptor)
            throw error
        }
    }

    private func adoptListener(horizon: DNSHorizon, descriptor: Int32) throws -> DNSListener {
        try setNonBlocking(descriptor)
        let source = DispatchSource.makeReadSource(
            fileDescriptor: descriptor,
            queue: queue
        )
        // Capture horizon by value on the listener; do not re-derive from endpoint
        // strings inside the handler.
        source.setEventHandler { [weak self, horizon, descriptor] in
            self?.receive(on: descriptor, horizon: horizon)
        }
        // The cancel handler owns the close: cancel() is asynchronous, so an
        // inline close could recycle the FD number into a fresh listener while
        // the old event handler still reads from it.
        source.setCancelHandler {
            Darwin.close(descriptor)
        }
        source.resume()
        return DNSListener(descriptor: descriptor, source: source, horizon: horizon)
    }

    private func receive(on descriptor: Int32, horizon: DNSHorizon) {
        while true {
            var buffer = [UInt8](repeating: 0, count: 65_535)
            var peer = sockaddr_storage()
            var peerLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
            let count = withUnsafeMutablePointer(to: &peer) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.recvfrom(descriptor, &buffer, buffer.count, 0, $0, &peerLength)
                }
            }
            if count < 0 {
                if errno == EAGAIN || errno == EWOULDBLOCK { break }
                return
            }
            guard count > 0 else { break }
            let request = Data(buffer.prefix(Int(count)))
            let peerCopy = DNSPeer(storage: peer, length: peerLength)
            if requiresUpstream(request, horizon: horizon) {
                resolveUpstream(to: peerCopy, on: descriptor, request: request, horizon: horizon)
            } else {
                reply(to: peerCopy, on: descriptor, request: request, horizon: horizon)
            }
        }
    }

    /// Answer via the (blocking) upstream path off the listener queue. The
    /// listener FD may be retired and its number reused while the resolve
    /// blocks, so the async reply goes out on a private dup of the socket.
    private func resolveUpstream(
        to peer: DNSPeer,
        on descriptor: Int32,
        request: Data,
        horizon: DNSHorizon
    ) {
        guard resolveSlots.wait(timeout: .now()) == .success else {
            send(errorResponse(request, responseCode: 2), to: peer, on: descriptor)
            return
        }
        let duplicated = Darwin.dup(descriptor)
        guard duplicated >= 0 else {
            resolveSlots.signal()
            send(errorResponse(request, responseCode: 2), to: peer, on: descriptor)
            return
        }
        resolveQueue.async { [self] in
            defer {
                Darwin.close(duplicated)
                resolveSlots.signal()
            }
            reply(to: peer, on: duplicated, request: request, horizon: horizon)
        }
    }

    private func reply(
        to peer: DNSPeer,
        on descriptor: Int32,
        request: Data,
        horizon: DNSHorizon
    ) {
        let responseData = makeResponse(for: request, horizon: horizon)
        guard !responseData.isEmpty else { return }
        send(responseData, to: peer, on: descriptor)
    }

    private func send(_ data: Data, to peer: DNSPeer, on descriptor: Int32) {
        guard !data.isEmpty else { return }
        var peerStorage = peer.storage
        let peerLength = peer.length
        _ = data.withUnsafeBytes { bytes in
            withUnsafePointer(to: &peerStorage) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.sendto(
                        descriptor,
                        bytes.baseAddress,
                        data.count,
                        0,
                        $0,
                        peerLength
                    )
                }
            }
        }
    }

    private func requiresUpstream(_ request: Data, horizon: DNSHorizon) -> Bool {
        guard let question = parseQuestion(request) else { return false }
        let snapshot = lock.withLock { zone }
        return !snapshot.isAuthoritative(for: question.name, horizon: horizon)
    }

    private func setNonBlocking(_ descriptor: Int32) throws {
        let flags = fcntl(descriptor, F_GETFL, 0)
        guard flags >= 0 else { throw DNSError.system("fcntl F_GETFL", errno) }
        guard fcntl(descriptor, F_SETFL, flags | O_NONBLOCK) == 0 else {
            throw DNSError.system("fcntl O_NONBLOCK", errno)
        }
    }

    func makeResponse(for request: Data, horizon: DNSHorizon = .host) -> Data {
        response(for: request, horizon: horizon)
    }

    func response(for request: Data, horizon: DNSHorizon = .host) -> Data {
        guard let question = parseQuestion(request) else {
            return errorResponse(request, responseCode: 1)
        }
        let snapshot = lock.withLock { zone }
        if snapshot.isAuthoritative(for: question.name, horizon: horizon) {
            if question.type == 1,
               question.dnsClass == 1,
               let addresses = snapshot.addresses(for: question.name, horizon: horizon)
            {
                return authoritativeResponse(
                    request,
                    question: question,
                    addresses: addresses,
                    ttl: snapshot.ttl
                )
            }
            if question.type == 12,
               question.dnsClass == 1,
               let names = snapshot.ptrNames(for: question.name)
            {
                return authoritativePTRResponse(
                    request,
                    question: question,
                    names: names,
                    ttl: snapshot.ttl
                )
            }
            let exists = snapshot.addresses(for: question.name, horizon: horizon) != nil
                || snapshot.ptrNames(for: question.name) != nil
            return authoritativeResponse(
                request,
                question: question,
                addresses: [],
                ttl: snapshot.ttl,
                responseCode: exists ? 0 : 3,
                authorityZone: snapshot.authorityZone(for: question.name)
            )
        }
        if configuration.upstream == "system" {
            return systemResolve(request, question)
                ?? errorResponse(request, responseCode: 2)
        }
        return forward(request) ?? errorResponse(request, responseCode: 2)
    }

    private func forward(_ request: Data) -> Data? {
        for upstream in upstreamEndpoints() {
            guard let parsed = parseEndpoint(upstream) else { continue }
            let descriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
            guard descriptor >= 0 else { continue }
            defer { Darwin.close(descriptor) }

            var timeout = timeval(tv_sec: 2, tv_usec: 0)
            setsockopt(
                descriptor,
                SOL_SOCKET,
                SO_RCVTIMEO,
                &timeout,
                socklen_t(MemoryLayout<timeval>.size)
            )
            var address = sockaddr_in()
            address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
            address.sin_family = sa_family_t(AF_INET)
            address.sin_port = parsed.port.bigEndian
            guard inet_pton(AF_INET, parsed.address, &address.sin_addr) == 1 else { continue }
            // connect() lets the kernel drop datagrams from foreign peers.
            let connected = withUnsafePointer(to: &address) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
                }
            }
            guard connected == 0 else { continue }
            let sent = request.withUnsafeBytes { bytes in
                Darwin.send(descriptor, bytes.baseAddress, request.count, 0)
            }
            guard sent == request.count else { continue }
            let requestID = readUInt16(request, 0)
            var response = [UInt8](repeating: 0, count: 65_535)
            while true {
                let count = Darwin.recv(descriptor, &response, response.count, 0)
                guard count >= dnsHeaderLength else { break }
                let data = Data(response.prefix(Int(count)))
                // Ignore replies whose transaction ID does not match the query.
                if readUInt16(data, 0) == requestID {
                    return data
                }
            }
        }
        return nil
    }

    private func upstreamEndpoints() -> [String] {
        configuration.upstream
            .split(separator: ",")
            .map { String($0).trimmingCharacters(in: .whitespaces) }
            .map { $0.contains(":") ? $0 : endpoint($0, 53) }
    }
}

private enum DNSError: Error, CustomStringConvertible {
    case invalidEndpoint(String)
    case system(String, Int32)

    var description: String {
        switch self {
        case let .invalidEndpoint(value):
            return "invalid DNS endpoint: \(value)"
        case let .system(operation, code):
            return "\(operation): \(String(cString: strerror(code)))"
        }
    }
}

private func endpoint(_ address: String, _ port: UInt16) -> String {
    "\(address):\(port)"
}

private func dnsProbeQuery() -> Data {
    var packet = Data([0x12, 0x34, 0x01, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0])
    for label in "vz.test".split(separator: ".") {
        packet.append(UInt8(label.utf8.count))
        packet.append(contentsOf: label.utf8)
    }
    packet.append(0)
    packet.append(contentsOf: [0, 1, 0, 1])
    return packet
}

private func parseEndpoint(_ value: String) -> (address: String, port: UInt16)? {
    guard let separator = value.lastIndex(of: ":"),
          let port = UInt16(value[value.index(after: separator)...])
    else {
        return nil
    }
    let address = String(value[..<separator])
    guard ipv4Bytes(address) != nil else { return nil }
    return (address, port)
}

private func ipv4Bytes(_ value: String) -> [UInt8]? {
    var address = in_addr()
    guard inet_pton(AF_INET, value, &address) == 1 else { return nil }
    return withUnsafeBytes(of: &address.s_addr) { Array($0) }
}

private func parseQuestion(_ message: Data) -> DNSQuestion? {
    guard message.count >= dnsHeaderLength,
          readUInt16(message, 4) == 1
    else {
        return nil
    }
    var offset = dnsHeaderLength
    guard let name = decodeName(message, offset: &offset),
          offset + 4 <= message.count,
          let type = readUInt16(message, offset),
          let dnsClass = readUInt16(message, offset + 2)
    else {
        return nil
    }
    return DNSQuestion(
        name: DNSZoneBuilder.canonicalName(name),
        type: type,
        dnsClass: dnsClass,
        endOffset: offset + 4
    )
}

private func decodeName(_ message: Data, offset: inout Int) -> String? {
    var labels: [String] = []
    var cursor = offset
    var jumped = false
    var visited: Set<Int> = []

    while cursor < message.count {
        guard !visited.contains(cursor) else { return nil }
        visited.insert(cursor)
        let length = Int(message[cursor])
        if length == 0 {
            cursor += 1
            if !jumped { offset = cursor }
            return labels.joined(separator: ".")
        }
        if length & 0xC0 == 0xC0 {
            guard cursor + 1 < message.count else { return nil }
            let pointer = ((length & 0x3F) << 8) | Int(message[cursor + 1])
            guard pointer < message.count else { return nil }
            if !jumped { offset = cursor + 2 }
            jumped = true
            cursor = pointer
            continue
        }
        guard length <= 63, cursor + 1 + length <= message.count else { return nil }
        let bytes = message[(cursor + 1)..<(cursor + 1 + length)]
        guard let label = String(bytes: bytes, encoding: .utf8) else { return nil }
        labels.append(label)
        cursor += 1 + length
        if !jumped { offset = cursor }
    }
    return nil
}

private func authoritativeResponse(
    _ request: Data,
    question: DNSQuestion,
    addresses: [String],
    ttl: UInt32,
    responseCode: UInt16 = 0,
    authorityZone: String? = nil
) -> Data {
    let soa = authorityZone.flatMap { soaRecord(zone: $0, ttl: ttl) }
    var response = Data()
    appendUInt16(readUInt16(request, 0) ?? 0, to: &response)
    let requestFlags = readUInt16(request, 2) ?? 0
    appendUInt16(0x8480 | (requestFlags & 0x0100) | responseCode, to: &response)
    appendUInt16(1, to: &response)
    appendUInt16(UInt16(addresses.count), to: &response)
    appendUInt16(soa == nil ? 0 : 1, to: &response)
    appendUInt16(0, to: &response)
    response.append(request[dnsHeaderLength..<question.endOffset])

    for address in addresses {
        guard let bytes = ipv4Bytes(address) else { continue }
        appendUInt16(0xC00C, to: &response)
        appendUInt16(1, to: &response)
        appendUInt16(1, to: &response)
        appendUInt32(ttl, to: &response)
        appendUInt16(4, to: &response)
        response.append(contentsOf: bytes)
    }
    if let soa {
        response.append(soa)
    }
    return response
}

/// Synthetic SOA for negative answers so clients can cache NXDOMAIN/NODATA
/// (RFC 2308). MNAME/RNAME are informational; minimum doubles as negative TTL.
private func soaRecord(zone: String, ttl: UInt32) -> Data? {
    guard let owner = encodeDNSName(zone),
          let primary = encodeDNSName("dns.vz.test"),
          let contact = encodeDNSName("hostmaster.vz.test")
    else {
        return nil
    }
    var rdata = Data()
    rdata.append(primary)
    rdata.append(contact)
    appendUInt32(1, to: &rdata)
    appendUInt32(ttl, to: &rdata)
    appendUInt32(ttl, to: &rdata)
    appendUInt32(86_400, to: &rdata)
    appendUInt32(ttl, to: &rdata)

    var record = Data()
    record.append(owner)
    appendUInt16(6, to: &record)
    appendUInt16(1, to: &record)
    appendUInt32(ttl, to: &record)
    appendUInt16(UInt16(rdata.count), to: &record)
    record.append(rdata)
    return record
}

private func authoritativePTRResponse(
    _ request: Data,
    question: DNSQuestion,
    names: [String],
    ttl: UInt32
) -> Data {
    var response = Data()
    appendUInt16(readUInt16(request, 0) ?? 0, to: &response)
    let requestFlags = readUInt16(request, 2) ?? 0
    appendUInt16(0x8480 | (requestFlags & 0x0100), to: &response)
    appendUInt16(1, to: &response)
    appendUInt16(UInt16(names.count), to: &response)
    appendUInt16(0, to: &response)
    appendUInt16(0, to: &response)
    response.append(request[dnsHeaderLength..<question.endOffset])

    for name in names {
        guard let encoded = encodeDNSName(name) else { continue }
        appendUInt16(0xC00C, to: &response)
        appendUInt16(12, to: &response)
        appendUInt16(1, to: &response)
        appendUInt32(ttl, to: &response)
        appendUInt16(UInt16(encoded.count), to: &response)
        response.append(encoded)
    }
    return response
}

private func encodeDNSName(_ name: String) -> Data? {
    var encoded = Data()
    for label in DNSZoneBuilder.canonicalName(name).split(separator: ".") {
        guard !label.isEmpty, label.utf8.count <= 63 else { return nil }
        encoded.append(UInt8(label.utf8.count))
        encoded.append(contentsOf: label.utf8)
    }
    encoded.append(0)
    return encoded
}

private func errorResponse(_ request: Data, responseCode: UInt16) -> Data {
    guard request.count >= dnsHeaderLength else { return Data() }
    var response = Data()
    appendUInt16(readUInt16(request, 0) ?? 0, to: &response)
    let requestFlags = readUInt16(request, 2) ?? 0
    appendUInt16(0x8080 | (requestFlags & 0x0100) | responseCode, to: &response)
    appendUInt16(0, to: &response)
    appendUInt16(0, to: &response)
    appendUInt16(0, to: &response)
    appendUInt16(0, to: &response)
    return response
}

private func readUInt16(_ data: Data, _ offset: Int) -> UInt16? {
    guard offset + 2 <= data.count else { return nil }
    return UInt16(data[offset]) << 8 | UInt16(data[offset + 1])
}

private func appendUInt16(_ value: UInt16, to data: inout Data) {
    data.append(UInt8((value >> 8) & 0xFF))
    data.append(UInt8(value & 0xFF))
}

private func appendUInt32(_ value: UInt32, to data: inout Data) {
    data.append(UInt8((value >> 24) & 0xFF))
    data.append(UInt8((value >> 16) & 0xFF))
    data.append(UInt8((value >> 8) & 0xFF))
    data.append(UInt8(value & 0xFF))
}
