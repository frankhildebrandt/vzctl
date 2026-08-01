import Darwin
import Dispatch
import Foundation
import VzDaemonKit

private let dnsHeaderLength = 12

struct DNSZone: Equatable, Sendable {
    /// Guest-horizon A records (and shared VM records).
    let records: [String: [String]]
    /// Host-horizon A records for ingress/OIDC names (loopback for Mac clients).
    let hostRecords: [String: [String]]
    /// Project ownership for host-managed ingress/OIDC names.
    let ingressProjects: [String: String]
    let zones: Set<String>
    let ttl: UInt32

    func addresses(for name: String, horizon: DNSHorizon) -> [String]? {
        let canonical = DNSZoneBuilder.canonicalName(name)
        switch horizon {
        case .host:
            if let host = hostRecords[canonical] {
                return host
            }
            return records[canonical]
        case let .guest(context):
            if hostRecords[canonical] != nil {
                guard ingressProjects[canonical] == context.project else { return nil }
                return [context.hostService]
            }
            return records[canonical]
        }
    }

    func isAuthoritative(for name: String) -> Bool {
        let canonical = DNSZoneBuilder.canonicalName(name)
        return canonical == "vz.test"
            || canonical.hasSuffix(".vz.test")
    }
}

struct DNSGuestContext: Equatable, Sendable {
    let network: String
    let project: String?
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
        hostServices: [String] = []
    ) -> DNSZone {
        let networks = Dictionary(uniqueKeysWithValues: snapshot.networks.map { ($0.name, $0) })
        var records: [String: Set<String>] = [:]
        var zones: Set<String> = []
        for attachment in snapshot.attachments {
            guard let network = networks[attachment.networkName],
                  network.runtimeState == "active",
                  let project = attachment.project ?? network.project,
                  let vm = dnsLabel(vmDNSLabel(attachment.vmID)),
                  let net = dnsLabel(attachment.networkName),
                  let projectLabel = dnsLabel(project),
                  ipv4Bytes(attachment.ip) != nil
            else {
                continue
            }

            let zone = "\(projectLabel).vz.test"
            zones.insert(zone)
            records["\(vm).\(net).\(zone)", default: []].insert(attachment.ip)

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
            hostRecords: hostRecords,
            ingressProjects: ingressProjects,
            zones: zones.filter { !$0.isEmpty },
            ttl: min(30, max(5, ttl))
        )
    }

    static func canonicalName(_ name: String) -> String {
        name.lowercased().trimmingCharacters(in: CharacterSet(charactersIn: "."))
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
    let guestPort: UInt16
    let ttl: UInt32
    let upstream: String

    static func environment(
        _ values: [String: String] = ProcessInfo.processInfo.environment
    ) -> DNSConfiguration {
        DNSConfiguration(
            hostAddress: values["VZCTL_DNS_HOST"] ?? "127.0.0.1",
            hostPort: port(values["VZCTL_DNS_PORT"]) ?? 15_353,
            guestPort: port(values["VZCTL_DNS_GUEST_PORT"])
                ?? 53,
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

private struct DNSListener {
    let descriptor: Int32
    let source: DispatchSourceRead
    /// Fixed at bind time so concurrent DispatchSource handlers cannot mis-classify
    /// guest `.0:53` sockets as the host horizon (which maps `*.svc` → 127.0.0.1).
    let horizon: DNSHorizon
}

private struct DNSQuestion {
    let name: String
    let type: UInt16
    let dnsClass: UInt16
    let endOffset: Int
}

final class DNSServer: @unchecked Sendable {
    private let configuration: DNSConfiguration
    private let lock = NSLock()
    private let queue = DispatchQueue(label: "dev.vzctl.dns", attributes: .concurrent)
    private var zone: DNSZone
    private var listeners: [String: DNSListener] = [:]
    private var desiredListeners: Set<String> = []
    private var guestContexts: [DNSGuestContext] = []
    private var hostServices: [String] = []
    private var lastError: String?
    private var stopped = false

    init(configuration: DNSConfiguration = .environment()) {
        self.configuration = configuration
        zone = DNSZone(
            records: [:],
            hostRecords: [:],
            ingressProjects: [:],
            zones: [],
            ttl: min(30, max(5, configuration.ttl))
        )
    }

    func setHostServices(_ names: [String]) {
        lock.withLock {
            hostServices = names.map(DNSZoneBuilder.canonicalName)
        }
    }

    @discardableResult
    func reload(snapshot: NetworkSnapshot) -> DNSHealth {
        let services = lock.withLock { hostServices }
        let nextZone = DNSZoneBuilder.build(
            snapshot: snapshot,
            ttl: configuration.ttl,
            hostServices: services
        )
        let contexts = snapshot.networks.compactMap { network -> DNSGuestContext? in
            guard network.runtimeState == "active", !network.isDockerBackend else { return nil }
            let gateway = IPv4CIDR.gateway(for: network.cidr)
            let hostService = IPv4CIDR.hostService(for: network.cidr)
            guard !gateway.isEmpty, !hostService.isEmpty else { return nil }
            return DNSGuestContext(
                network: network.name,
                project: network.project.map(DNSZoneBuilder.canonicalName),
                gateway: gateway,
                hostService: hostService
            )
        }
        var desired = Dictionary(uniqueKeysWithValues: contexts.map {
            (endpoint($0.gateway, configuration.guestPort), DNSHorizon.guest($0))
        })
        desired[endpoint(configuration.hostAddress, configuration.hostPort)] = .host
        let desiredKeys = Set(desired.keys)

        lock.withLock {
            guard !stopped else { return }
            zone = nextZone
            desiredListeners = desiredKeys
            guestContexts = contexts.sorted { $0.network < $1.network }
            lastError = nil

            for key in listeners.keys where !desiredKeys.contains(key) {
                removeListenerLocked(key)
            }
            for key in desired.keys.sorted() {
                if let existing = listeners[key], existing.horizon == desired[key] { continue }
                removeListenerLocked(key)
                do {
                    listeners[key] = try makeListener(endpoint: key, horizon: desired[key]!)
                } catch {
                    lastError = [lastError, "\(key): \(error)"]
                        .compactMap { $0 }
                        .joined(separator: "; ")
                }
            }
        }
        return health()
    }

    func health() -> DNSHealth {
        lock.withLock {
            let active = listeners.keys.sorted()
            return DNSHealth(
                ok: !stopped && Set(active) == desiredListeners && lastError == nil,
                listeners: active,
                records: zone.records.values.reduce(0) { $0 + $1.count },
                zones: zone.zones.count,
                ttl: zone.ttl,
                upstream: configuration.upstream,
                lastError: lastError
            )
        }
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
            listener.source.cancel()
            Darwin.close(listener.descriptor)
        }
    }

    private func makeListener(endpoint value: String, horizon: DNSHorizon) throws -> DNSListener {
        guard let parsed = parseEndpoint(value) else {
            throw DNSError.invalidEndpoint(value)
        }
        if DnsBind.needsPrivilege(port: parsed.port) {
            let descriptor = try DnsBindClient.bindUDP(
                address: parsed.address,
                port: parsed.port
            )
            return try adoptListener(horizon: horizon, descriptor: descriptor)
        }
        return try bindListenerLocally(
            endpoint: value,
            address: parsed.address,
            port: parsed.port,
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
        let source = DispatchSource.makeReadSource(
            fileDescriptor: descriptor,
            queue: queue
        )
        // Capture horizon by value on the listener; do not re-derive from endpoint
        // strings inside the concurrent handler.
        source.setEventHandler { [weak self, horizon, descriptor] in
            self?.receive(on: descriptor, horizon: horizon)
        }
        source.resume()
        return DNSListener(descriptor: descriptor, source: source, horizon: horizon)
    }

    private func removeListenerLocked(_ key: String) {
        guard let listener = listeners.removeValue(forKey: key) else { return }
        listener.source.cancel()
        Darwin.close(listener.descriptor)
    }

    private func receive(on descriptor: Int32, horizon: DNSHorizon) {
        var buffer = [UInt8](repeating: 0, count: 65_535)
        var peer = sockaddr_storage()
        var peerLength = socklen_t(MemoryLayout<sockaddr_storage>.size)
        let count = withUnsafeMutablePointer(to: &peer) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.recvfrom(descriptor, &buffer, buffer.count, 0, $0, &peerLength)
            }
        }
        guard count > 0 else { return }
        let request = Data(buffer.prefix(Int(count)))
        let responseData = makeResponse(for: request, horizon: horizon)
        guard !responseData.isEmpty else { return }
        _ = responseData.withUnsafeBytes { bytes in
            withUnsafePointer(to: &peer) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.sendto(
                        descriptor,
                        bytes.baseAddress,
                        responseData.count,
                        0,
                        $0,
                        peerLength
                    )
                }
            }
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
        if snapshot.isAuthoritative(for: question.name) {
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
            let exists = snapshot.addresses(for: question.name, horizon: horizon) != nil
            return authoritativeResponse(
                request,
                question: question,
                addresses: [],
                ttl: snapshot.ttl,
                responseCode: exists ? 0 : 3
            )
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
            let sent = request.withUnsafeBytes { bytes in
                withUnsafePointer(to: &address) {
                    $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                        Darwin.sendto(
                            descriptor,
                            bytes.baseAddress,
                            request.count,
                            0,
                            $0,
                            socklen_t(MemoryLayout<sockaddr_in>.size)
                        )
                    }
                }
            }
            guard sent == request.count else { continue }
            var response = [UInt8](repeating: 0, count: 65_535)
            let count = Darwin.recv(descriptor, &response, response.count, 0)
            if count > 0 {
                return Data(response.prefix(Int(count)))
            }
        }
        return nil
    }

    private func upstreamEndpoints() -> [String] {
        if configuration.upstream != "system" {
            return configuration.upstream
                .split(separator: ",")
                .map { String($0).trimmingCharacters(in: .whitespaces) }
                .map { $0.contains(":") ? $0 : endpoint($0, 53) }
        }
        guard let resolv = try? String(contentsOfFile: "/etc/resolv.conf", encoding: .utf8)
        else {
            return []
        }
        return resolv.split(separator: "\n").compactMap { line in
            let fields = line.split(whereSeparator: \.isWhitespace)
            guard fields.count >= 2, fields[0] == "nameserver" else { return nil }
            let address = String(fields[1])
            guard ipv4Bytes(address) != nil else { return nil }
            return endpoint(address, 53)
        }
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
    responseCode: UInt16 = 0
) -> Data {
    var response = Data()
    appendUInt16(readUInt16(request, 0) ?? 0, to: &response)
    let requestFlags = readUInt16(request, 2) ?? 0
    appendUInt16(0x8480 | (requestFlags & 0x0100) | responseCode, to: &response)
    appendUInt16(1, to: &response)
    appendUInt16(UInt16(addresses.count), to: &response)
    appendUInt16(0, to: &response)
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
    return response
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
