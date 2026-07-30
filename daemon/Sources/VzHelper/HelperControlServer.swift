import Darwin
import Dispatch
import Foundation
import VzDaemonKit

final class HelperControlServer: @unchecked Sendable {
    let socketPath: String
    private let handler: @Sendable (RouterPlan) async throws -> Bool
    private let lock = NSLock()
    private var listener: Int32 = -1
    private var ownsSocket = false

    init(
        vmID: String,
        stateDirectory: URL,
        handler: @escaping @Sendable (RouterPlan) async throws -> Bool
    ) {
        socketPath = stateDirectory
            .appendingPathComponent("helpers", isDirectory: true)
            .appendingPathComponent("\(StateFileName.component(vmID)).sock")
            .path
        self.handler = handler
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
            guard request.method == "route.apply" else {
                response = JSONRPCResponse(
                    error: JSONRPCError(code: -32601, message: "Method not found"),
                    id: request.id ?? .null
                )
                write(response, to: fd)
                return
            }
            let plan = try Self.plan(from: request.params)
            let box = AsyncResultBox()
            Task {
                do {
                    box.finish(.success(try await handler(plan)))
                } catch {
                    box.finish(.failure(error))
                }
            }
            switch box.wait() {
            case let .success(changed):
                response = JSONRPCResponse(
                    result: .object(["changed": .bool(changed)]),
                    id: request.id ?? .null
                )
            case let .failure(error):
                response = JSONRPCResponse(
                    error: JSONRPCError(code: -32019, message: String(describing: error)),
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
        guard networks.count >= 2 else {
            throw RouteApplyError.invalid("router plan requires at least two networks")
        }
        guard Set(networks.map(\.name)).count == networks.count else {
            throw RouteApplyError.invalid("router plan contains duplicate networks")
        }
        for network in networks {
            _ = try IPv4CIDR(network.cidr)
            guard network.address == IPv4CIDR.router(for: network.cidr) else {
                throw RouteApplyError.invalid(
                    "router address for \(network.name) must be \(IPv4CIDR.router(for: network.cidr))"
                )
            }
        }
        return RouterPlan(vmID: vmID, networks: networks)
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
    private var result: Result<Bool, Error>?

    func finish(_ value: Result<Bool, Error>) {
        lock.withLock { result = value }
        semaphore.signal()
    }

    func wait() -> Result<Bool, Error> {
        semaphore.wait()
        return lock.withLock { result! }
    }
}

enum RouterGuestConfigurator {
    static func apply(
        _ plan: RouterPlan,
        runtime: VirtualMachineRuntime,
        token: String
    ) async throws -> Bool {
        let client = try await runtime.connectToGuestAgent(timeout: 5)
        defer { client.close() }
        _ = try client.hello(token: token, helperVersion: VzDaemonKit.version)
        let payload = try JSONSerialization.data(
            withJSONObject: [
                "apiVersion": "vzctl.dev/router/v1",
                "vm_id": plan.vmID,
                "networks": plan.networks.map {
                    [
                        "name": $0.name,
                        "cidr": $0.cidr,
                        "address": $0.address,
                        "host_gateway_dns": IPv4CIDR.gateway(for: $0.cidr),
                        "router_gateway": IPv4CIDR.router(for: $0.cidr),
                    ]
                },
                "forward_policy": "drop",
            ],
            options: [.sortedKeys]
        )
        let result = try client.exec(
            argv: ["/bin/sh", "-ceu", routerApplyScript],
            stdin: payload,
            timeoutMilliseconds: 30_000
        )
        guard result.exit == 0, !result.truncated else {
            throw RouteApplyError.guest(
                "router apply failed (exit \(result.exit)): \(result.stderr)"
            )
        }
        return result.stdout.contains("changed=true")
    }

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
        trap 'rm -f "$routes_tmp" "$sysctl_tmp"' EXIT
        cat >"$routes_tmp"
        printf 'net.ipv4.ip_forward=1\\n' >"$sysctl_tmp"
        install_if_changed "$routes_tmp" /etc/vzctl/routes.json 0644
        install_if_changed "$sysctl_tmp" /etc/sysctl.d/90-vzctl-router.conf 0644
        if [ "$(sysctl -n net.ipv4.ip_forward)" != 1 ]; then
          changed=true
        fi
        sysctl -q -w net.ipv4.ip_forward=1
        if command -v iptables >/dev/null 2>&1; then
          current=$(iptables -S FORWARD | sed -n '1s/^-P FORWARD //p')
          if [ "$current" != DROP ]; then
            iptables -P FORWARD DROP
            changed=true
          fi
        elif command -v nft >/dev/null 2>&1; then
          if ! nft list table inet vzctl >/dev/null 2>&1; then
            nft add table inet vzctl
            nft 'add chain inet vzctl forward { type filter hook forward priority 0; policy drop; }'
            changed=true
          fi
        else
          echo 'no nftables or iptables backend available' >&2
          exit 1
        fi
        printf 'changed=%s\\n' "$changed"
        """
}
