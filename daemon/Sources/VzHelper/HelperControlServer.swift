import Darwin
import Dispatch
import Foundation
import VzDaemonKit

final class HelperControlServer: @unchecked Sendable {
    let socketPath: String
    private let routeHandler: @Sendable (RouterOperation, RouterPlan) async throws -> JSONValue
    private let agentHandler: @Sendable (String, JSONValue?) async throws -> JSONValue
    private let lock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false

    init(
        vmID: String,
        stateDirectory: URL,
        routeHandler: @escaping @Sendable (RouterOperation, RouterPlan) async throws -> JSONValue,
        agentHandler: @escaping @Sendable (String, JSONValue?) async throws -> JSONValue
    ) {
        socketPath = stateDirectory
            .appendingPathComponent("helpers", isDirectory: true)
            .appendingPathComponent("\(StateFileName.component(vmID)).sock")
            .path
        self.routeHandler = routeHandler
        self.agentHandler = agentHandler
    }

    func start() throws {
        if FileManager.default.fileExists(atPath: socketPath) {
            guard Darwin.unlink(socketPath) == 0 else {
                throw HelperError.system("unlink stale helper control socket", errno)
            }
        }
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("helper control socket", errno) }
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(socketPath.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(fd)
            throw HelperError.invalid("helper control socket path is too long")
        }
        withUnsafeMutableBytes(of: &address.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        let bound = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
            }
        }
        guard bound == 0 else {
            let code = errno
            Darwin.close(fd)
            throw HelperError.system("bind helper control socket", code)
        }
        guard chmod(socketPath, 0o600) == 0, Darwin.listen(fd, 4) == 0 else {
            let code = errno
            Darwin.close(fd)
            Darwin.unlink(socketPath)
            throw HelperError.system("listen helper control socket", code)
        }
        lock.withLock {
            listener = fd
            ownsSocket = true
        }
        DispatchQueue.global().async { [weak self] in self?.acceptLoop(fd) }
    }

    func stop() {
        let state = lock.withLock { () -> (Int32, Bool) in
            let state = (listener, ownsSocket)
            listener = -1
            ownsSocket = false
            return state
        }
        if state.0 >= 0 {
            Darwin.shutdown(state.0, SHUT_RDWR)
            Darwin.close(state.0)
        }
        if state.1 { Darwin.unlink(socketPath) }
    }

    deinit {
        stop()
    }

    private func acceptLoop(_ fd: Int32) {
        while lock.withLock({ listener == fd }) {
            let client = Darwin.accept(fd, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                return
            }
            handle(client)
            Darwin.close(client)
        }
    }

    private func handle(_ fd: Int32) {
        guard peerUID(fd) == geteuid(), let data = readLine(fd) else { return }
        let response: JSONRPCResponse
        do {
            let request = try JSONRPCFraming.decode(JSONRPCRequest.self, from: data)
            let box = AsyncResultBox()
            if request.method.hasPrefix("route.") {
                guard let operation = RouterOperation(
                    rawValue: String(request.method.dropFirst("route.".count))
                ) else {
                    write(
                        JSONRPCResponse(
                            error: JSONRPCError(code: -32601, message: "Method not found"),
                            id: request.id ?? .null
                        ),
                        to: fd
                    )
                    return
                }
                let plan = try Self.plan(from: request.params)
                Task {
                    do {
                        box.finish(.success(try await routeHandler(operation, plan)))
                    } catch {
                        box.finish(.failure(error))
                    }
                }
            } else if request.method.hasPrefix("agent.") {
                guard HelperAgentProxy.methods.contains(request.method) else {
                    write(
                        JSONRPCResponse(
                            error: JSONRPCError(code: -32601, message: "Method not found"),
                            id: request.id ?? .null
                        ),
                        to: fd
                    )
                    return
                }
                Task {
                    do {
                        box.finish(
                            .success(try await agentHandler(request.method, request.params))
                        )
                    } catch {
                        box.finish(.failure(error))
                    }
                }
            } else {
                write(
                    JSONRPCResponse(
                        error: JSONRPCError(code: -32601, message: "Method not found"),
                        id: request.id ?? .null
                    ),
                    to: fd
                )
                return
            }
            switch box.wait() {
            case let .success(result):
                response = JSONRPCResponse(
                    result: result,
                    id: request.id ?? .null
                )
            case let .failure(error):
                response = JSONRPCResponse(
                    error: Self.rpcError(from: error),
                    id: request.id ?? .null
                )
            }
        } catch {
            response = JSONRPCResponse(
                error: JSONRPCError(code: -32602, message: String(describing: error)),
                id: .null
            )
        }
        write(response, to: fd)
    }

    private static func rpcError(from error: Error) -> JSONRPCError {
        if let routeError = error as? RouteApplyError {
            return JSONRPCError(code: routeError.rpcCode, message: routeError.description)
        }
        if let guestError = error as? GuestAgentError {
            return JSONRPCError(code: -32019, message: guestError.description)
        }
        if let helperError = error as? HelperError, case let .invalid(message) = helperError {
            return JSONRPCError(code: -32602, message: message)
        }
        return JSONRPCError(code: -32019, message: String(describing: error))
    }

    private static func plan(from value: JSONValue?) throws -> RouterPlan {
        guard case let .object(params)? = value,
              case let .string(vmID)? = params["vm_id"],
              case let .array(rawNetworks)? = params["networks"]
        else {
            throw RouteApplyError.invalid("invalid route plan")
        }
        let networks = try rawNetworks.map { item -> RouterNetwork in
            guard case let .object(values) = item,
                  case let .string(name)? = values["name"],
                  case let .string(cidr)? = values["cidr"],
                  case let .string(address)? = values["address"]
            else {
                throw RouteApplyError.invalid("invalid router network")
            }
            return RouterNetwork(name: name, cidr: cidr, address: address)
        }
        let rawPolicies: [JSONValue]
        if case let .array(items)? = params["policies"] {
            rawPolicies = items
        } else {
            rawPolicies = []
        }
        let policies = try rawPolicies.map { item -> ForwardPolicy in
            guard case let .object(values) = item,
                  case let .string(name)? = values["name"],
                  case let .string(network)? = values["network"],
                  case let .string(forward)? = values["forward"]
            else {
                throw RouteApplyError.invalid("invalid forward policy")
            }
            let rawAllows: [JSONValue]
            if case let .array(items)? = values["allow"] {
                rawAllows = items
            } else {
                rawAllows = []
            }
            let allows = try rawAllows.map { raw -> PolicyAllow in
                guard case let .object(allow) = raw,
                      case let .string(to)? = allow["to"],
                      case let .string(proto)? = allow["proto"]
                else {
                    throw RouteApplyError.invalid("invalid allow rule in policy \(name)")
                }
                let ports: [Int]
                if case let .array(values)? = allow["ports"] {
                    ports = try values.map {
                        guard case let .number(value) = $0, value.rounded() == value else {
                            throw RouteApplyError.invalid("invalid port in policy \(name)")
                        }
                        return Int(value)
                    }
                } else {
                    ports = []
                }
                return PolicyAllow(to: to, proto: proto, ports: ports)
            }
            return ForwardPolicy(
                name: name,
                network: network,
                forward: forward,
                allow: allows
            )
        }
        return try RouterPlan(vmID: vmID, networks: networks, policies: policies)
    }

    private func peerUID(_ fd: Int32) -> uid_t? {
        var credentials = xucred()
        var length = socklen_t(MemoryLayout<xucred>.size)
        let result = withUnsafeMutablePointer(to: &credentials) {
            getsockopt(fd, SOL_LOCAL, LOCAL_PEERCRED, $0, &length)
        }
        return result == 0 ? credentials.cr_uid : nil
    }

    private func readLine(_ fd: Int32) -> Data? {
        var data = Data()
        var byte: UInt8 = 0
        while Darwin.read(fd, &byte, 1) == 1 {
            data.append(byte)
            if byte == 0x0A { return data }
            if data.count > 1_048_576 { return nil }
        }
        return nil
    }

    private func write(_ response: JSONRPCResponse, to fd: Int32) {
        guard let data = try? JSONRPCFraming.encode(response) else { return }
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
                if count <= 0 { return }
                offset += count
            }
        }
    }
}

private final class AsyncResultBox: @unchecked Sendable {
    private let lock = NSLock()
    private let semaphore = DispatchSemaphore(value: 0)
    private var result: Result<JSONValue, Error>?

    func finish(_ value: Result<JSONValue, Error>) {
        lock.withLock { result = value }
        semaphore.signal()
    }

    func wait() -> Result<JSONValue, Error> {
        semaphore.wait()
        return lock.withLock { result! }
    }
}

enum RouterGuestConfigurator {
    static func run(
        _ operation: RouterOperation,
        _ plan: RouterPlan,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws -> JSONValue {
        let client = try await runtime.connectToGuestAgent(timeout: 5)
        defer { client.close() }
        _ = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        let current = try currentConfiguration(client: client)
        let changes = policyChanges(current: current, desired: plan.json)
        if operation == .status {
            guard let current else {
                throw RouteApplyError.guest("router has no active vzctl nftables rules")
            }
            return response(
                configuration: current,
                changed: false,
                active: true,
                policyChanges: []
            )
        }
        if operation == .plan {
            return response(
                configuration: plan.json,
                changed: current != plan.json,
                active: current != nil,
                policyChanges: changes
            )
        }

        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys]
        let payload = try encoder.encode(plan.json)
        let applied = try client.exec(
            argv: ["/bin/sh", "-ceu", routerApplyScript, "--", plan.nftables],
            stdin: payload,
            timeoutMilliseconds: 30_000
        )
        guard applied.exit == 0, !applied.truncated else {
            throw RouteApplyError.guest(
                "router apply failed (exit \(applied.exit)): \(applied.stderr)"
            )
        }
        return response(
            configuration: plan.json,
            changed: applied.stdout.contains("changed=true"),
            active: true,
            policyChanges: changes
        )
    }

    private static func currentConfiguration(client: GuestAgentClient) throws -> JSONValue? {
        let result = try client.exec(
            argv: ["/bin/sh", "-ceu", routerStatusScript],
            timeoutMilliseconds: 5_000
        )
        if result.exit == 3 { return nil }
        guard result.exit == 0, !result.truncated,
              let data = result.stdout.data(using: .utf8)
        else {
            throw RouteApplyError.guest(
                "router status failed (exit \(result.exit)): \(result.stderr)"
            )
        }
        return try JSONDecoder().decode(JSONValue.self, from: data)
    }

    private static func response(
        configuration: JSONValue,
        changed: Bool,
        active: Bool,
        policyChanges: [JSONValue]
    ) -> JSONValue {
        guard case let .object(values) = configuration else { return .null }
        return .object([
            "changed": .bool(changed),
            "active": .bool(active),
            "forward_policy": values["forward_policy"] ?? .string("drop"),
            "policies": values["policies"] ?? .array([]),
            "rules": values["rules"] ?? .array([]),
            "policy_changes": .array(policyChanges),
        ])
    }

    private static func policyChanges(
        current: JSONValue?,
        desired: JSONValue
    ) -> [JSONValue] {
        func policies(_ value: JSONValue?) -> [String: JSONValue] {
            guard case let .object(root)? = value,
                  case let .array(items)? = root["policies"]
            else { return [:] }
            var result: [String: JSONValue] = [:]
            for item in items {
                guard case let .object(values) = item,
                      case let .string(name)? = values["name"]
                else { continue }
                result[name] = item
            }
            return result
        }
        let old = policies(current)
        let new = policies(desired)
        return Set(old.keys).union(new.keys).sorted().compactMap { name in
            let operation: String
            if old[name] == nil {
                operation = "add"
            } else if new[name] == nil {
                operation = "remove"
            } else if old[name] != new[name] {
                operation = "update"
            } else {
                return nil
            }
            return .object([
                "operation": .string(operation),
                "policy": .string(name),
            ])
        }
    }

    static let routerStatusScript = """
        if ! command -v nft >/dev/null 2>&1 ||
           ! nft list table inet vzctl >/dev/null 2>&1 ||
           [ ! -f /etc/vzctl/routes.json ]; then
          exit 3
        fi
        cat /etc/vzctl/routes.json
        """

    static let routerApplyScript = """
        changed=false
        install_if_changed() {
          source_file=$1
          target_file=$2
          mode=$3
          if [ ! -f "$target_file" ] || ! cmp -s "$source_file" "$target_file"; then
            install -m "$mode" "$source_file" "$target_file"
            changed=true
          fi
        }
        umask 022
        mkdir -p /etc/vzctl /etc/sysctl.d
        routes_tmp=$(mktemp)
        sysctl_tmp=$(mktemp)
        nft_tmp=$(mktemp)
        load_tmp=$(mktemp)
        trap 'rm -f "$routes_tmp" "$sysctl_tmp" "$nft_tmp" "$load_tmp"' EXIT
        cat >"$routes_tmp"
        printf '%s\\n' "$1" >"$nft_tmp"
        printf 'net.ipv4.ip_forward=1\\n' >"$sysctl_tmp"
        if ! command -v nft >/dev/null 2>&1; then
          echo 'nftables backend is required' >&2
          exit 1
        fi
        if [ ! -f /etc/vzctl/routes.json ] ||
           ! cmp -s "$routes_tmp" /etc/vzctl/routes.json ||
           [ ! -f /etc/vzctl/vzctl.nft ] ||
           ! cmp -s "$nft_tmp" /etc/vzctl/vzctl.nft ||
           ! nft list table inet vzctl >/dev/null 2>&1; then
          if nft list table inet vzctl >/dev/null 2>&1; then
            printf 'delete table inet vzctl\\n' >"$load_tmp"
          fi
          cat "$nft_tmp" >>"$load_tmp"
          nft -f "$load_tmp"
          install -m 0644 "$routes_tmp" /etc/vzctl/routes.json
          install -m 0644 "$nft_tmp" /etc/vzctl/vzctl.nft
          changed=true
        fi
        install_if_changed "$sysctl_tmp" /etc/sysctl.d/90-vzctl-router.conf 0644
        if [ "$(sysctl -n net.ipv4.ip_forward)" != 1 ]; then
          changed=true
        fi
        sysctl -q -w net.ipv4.ip_forward=1
        printf 'changed=%s\\n' "$changed"
        """
}
