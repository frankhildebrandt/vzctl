import CoreFoundation
import Dispatch
import Foundation
import vmnet

/// Process-local vmnet reservation + host bridge (ADR 0002 / vz-net).
///
/// G0: `.0` is only bindable after `vmnet_interface_start_with_network`.
public enum NativeVmnetError: Error, CustomStringConvertible, Sendable {
    case invalid(String)
    case runtime(String)

    public var description: String {
        switch self {
        case let .invalid(message), let .runtime(message):
            return message
        }
    }
}

public final class NativeVmnetHandle: @unchecked Sendable {
    public let network: vmnet_network_ref
    private let interface: interface_ref?

    public init(network: vmnet_network_ref, interface: interface_ref?) {
        self.network = network
        self.interface = interface
    }

    deinit {
        if let interface {
            let queue = DispatchQueue(label: "vzctl.vmnet.stop")
            let sem = DispatchSemaphore(value: 0)
            let status = vmnet_stop_interface(interface, queue) { _ in
                sem.signal()
            }
            if status == .VMNET_SUCCESS {
                _ = sem.wait(timeout: .now() + .seconds(5))
            }
        }
        // vmnet_network_ref is an opaque CF_RETURNS_RETAINED C handle; Swift ARC
        // does not release it automatically.
        releaseOpaqueCF(network)
    }
}

public enum NativeVmnetRuntime {
    public static func reserve(
        name: String,
        cidr rawCIDR: String,
        mode: String,
        natEgress: Bool
    ) throws -> NativeVmnetHandle {
        guard mode == "shared" else {
            throw NativeVmnetError.invalid(
                "bridged mode is unsupported in v0.1; use --mode shared"
            )
        }
        let cidr: IPv4CIDR
        do {
            cidr = try IPv4CIDR(rawCIDR)
        } catch {
            throw NativeVmnetError.invalid(String(describing: error))
        }

        let operationMode: operating_modes_t =
            natEgress ? .VMNET_SHARED_MODE : .VMNET_HOST_MODE

        var status: vmnet_return_t = .VMNET_SUCCESS
        guard let configuration = vmnet_network_configuration_create(operationMode, &status)
        else {
            throw NativeVmnetError.runtime(
                "vmnet configuration for \(name) failed (\(status.rawValue))"
            )
        }
        defer { releaseOpaqueCF(configuration) }
        var subnet = cidr.subnetAddress
        var mask = cidr.maskAddress
        status = vmnet_network_configuration_set_ipv4_subnet(configuration, &subnet, &mask)
        guard status == .VMNET_SUCCESS else {
            throw NativeVmnetError.runtime(
                "vmnet subnet \(cidr.canonical) failed (\(status.rawValue))"
            )
        }
        vmnet_network_configuration_disable_dhcp(configuration)
        vmnet_network_configuration_disable_dns_proxy(configuration)

        guard let network = vmnet_network_create(configuration, &status) else {
            var message =
                "vmnet reserve \(cidr.canonical) failed (\(status.rawValue))"
            // VMNET_FAILURE (1001) is the G0 orphan signature after unclean exit.
            if status == .VMNET_FAILURE {
                message +=
                    "; after an unclean exit this CIDR may remain orphaned until reboot"
            }
            throw NativeVmnetError.runtime(message)
        }
        do {
            let interface = try startHostInterface(network: network, name: name)
            return NativeVmnetHandle(network: network, interface: interface)
        } catch {
            releaseOpaqueCF(network)
            throw error
        }
    }

    private static func startHostInterface(
        network: vmnet_network_ref,
        name: String
    ) throws -> interface_ref {
        let desc = xpc_dictionary_create(nil, nil, 0)
        xpc_dictionary_set_bool(desc, vmnet_allocate_mac_address_key, true)

        let queue = DispatchQueue(label: "vzctl.vmnet.\(name)")
        let sem = DispatchSemaphore(value: 0)
        var completionStatus: vmnet_return_t = .VMNET_FAILURE

        guard let iface = vmnet_interface_start_with_network(network, desc, queue, { status, _ in
            completionStatus = status
            sem.signal()
        }) else {
            throw NativeVmnetError.runtime(
                "vmnet interface start for \(name) returned nil"
            )
        }

        let wait = sem.wait(timeout: .now() + .seconds(15))
        guard wait == .success else {
            _ = vmnet_stop_interface(iface, queue) { _ in }
            throw NativeVmnetError.runtime(
                "vmnet interface start for \(name) timed out"
            )
        }
        guard completionStatus == .VMNET_SUCCESS else {
            _ = vmnet_stop_interface(iface, queue) { _ in }
            throw NativeVmnetError.runtime(
                "vmnet interface start for \(name) failed (\(completionStatus.rawValue))"
            )
        }
        return iface
    }
}

private func releaseOpaqueCF(_ pointer: OpaquePointer) {
    Unmanaged<AnyObject>.fromOpaque(UnsafeRawPointer(pointer)).release()
}
