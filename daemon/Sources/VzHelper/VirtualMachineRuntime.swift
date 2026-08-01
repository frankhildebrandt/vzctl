import Darwin
import Dispatch
import Foundation
import VzDaemonKit
import vmnet
@preconcurrency import Virtualization

enum VirtualMachineEvent: Sendable {
    case stopped
    case failed(String)
}

final class VirtualMachineRuntime: NSObject, VZVirtualMachineDelegate, @unchecked Sendable {
    let serialLogURL: URL
    let bundleURL: URL

    private let queue: DispatchQueue
    private let virtualMachine: VZVirtualMachine
    private let eventStream: AsyncStream<VirtualMachineEvent>
    private let eventContinuation: AsyncStream<VirtualMachineEvent>.Continuation
    private let consoleLock = NSLock()
    private let mountsLock = NSLock()
    private var mounts: [VirtioFSMountSpec]
    private var serialInputWriter: FileHandle?
    private var serialOutputRead: FileHandle?
    private var serialLogWriter: FileHandle?
    private var consoleListener: Int32 = -1
    private var consoleOwnsSocket = false
    private var consoleSocketPath: String?
    private var consoleClients: [Int32] = []
    /// Keep process-local vmnet refs alive for the VM lifetime (ADR 0002 helper side).
    private let vmnetNetworks: [vmnet_network_ref]

    init(options: RunOptions, vmnetNICs: [HelperVmnetNIC] = []) throws {
        let logsDirectory = try StatePaths.logsDirectory()
        try FileManager.default.createDirectory(
            at: logsDirectory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        guard chmod(logsDirectory.path, 0o700) == 0 else {
            throw HelperError.system("chmod log directory", errno)
        }
        serialLogURL = logsDirectory.appendingPathComponent(
            "\(StateFileName.component(options.vmID)).serial.log"
        )
        bundleURL = options.bundleURL
        mounts = options.mounts
        queue = DispatchQueue(label: "vzctl.helper.\(StateFileName.component(options.vmID))")
        vmnetNetworks = vmnetNICs.map(\.network)

        let pair = AsyncStream<VirtualMachineEvent>.makeStream()
        eventStream = pair.stream
        eventContinuation = pair.continuation

        let resources = try Self.makeConfiguration(
            options: options,
            serialLogURL: serialLogURL,
            vmnetNICs: vmnetNICs
        )
        virtualMachine = VZVirtualMachine(configuration: resources.configuration, queue: queue)
        serialInputWriter = resources.serialInputWriter
        serialOutputRead = resources.serialOutputRead
        serialLogWriter = resources.serialLogWriter
        super.init()
        virtualMachine.delegate = self
        startSerialTee()
    }

    deinit {
        stopConsoleServer()
        serialOutputRead?.readabilityHandler = nil
        for network in vmnetNetworks {
            releaseOpaqueCF(network)
        }
    }

    func start() async throws {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [self] in
                virtualMachine.start { continuation.resume(with: $0) }
            }
        }
    }

    func stop() async {
        stopConsoleServer()
        let requested = await withCheckedContinuation { continuation in
            queue.async { [self] in
                guard virtualMachine.canRequestStop else {
                    continuation.resume(returning: false)
                    return
                }
                do {
                    try virtualMachine.requestStop()
                    continuation.resume(returning: true)
                } catch {
                    continuation.resume(returning: false)
                }
            }
        }
        if requested {
            try? await Task.sleep(for: .seconds(5))
        }

        await withCheckedContinuation { continuation in
            queue.async { [self] in
                guard virtualMachine.canStop else {
                    continuation.resume()
                    return
                }
                virtualMachine.stop { _ in continuation.resume() }
            }
        }
    }

    func waitForStop() async -> VirtualMachineEvent {
        var iterator = eventStream.makeAsyncIterator()
        return await iterator.next() ?? .stopped
    }

    func connectToGuestAgent(timeout: TimeInterval = 5) async throws -> GuestAgentClient {
        try await Task.detached { [self] in
            let waiter = SocketConnectionWaiter()
            queue.async { [self] in
                guard let socket = virtualMachine.socketDevices.first as? VZVirtioSocketDevice else {
                    waiter.complete(
                        .failure(GuestAgentError.unavailable("virtio socket device is missing"))
                    )
                    return
                }
                socket.connect(toPort: guestAgentPort) { result in
                    waiter.complete(result)
                }
            }
            guard waiter.wait(timeout: timeout) else {
                waiter.expire()
                throw GuestAgentError.timeout("connect")
            }
            return try GuestAgentClient(fileDescriptor: waiter.takeFileDescriptor())
        }.value
    }

    func currentMounts() -> [VirtioFSMountSpec] {
        mountsLock.withLock { mounts }
    }

    func applyShare(_ next: [VirtioFSMountSpec]) async throws {
        try VirtioFSShare.ensureHostDirectories(next)
        let share = try VirtioFSShare.makeShare(mounts: next, bundleURL: bundleURL)
        // VZDirectoryShare is not Sendable; confine use to the VM serial queue.
        nonisolated(unsafe) let shareToApply = share
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
            queue.async { [self] in
                guard
                    let device = virtualMachine.directorySharingDevices
                    .compactMap({ $0 as? VZVirtioFileSystemDevice })
                    .first(where: { $0.tag == VirtioFSShare.deviceTag })
                else {
                    continuation.resume(
                        throwing: HelperError.invalid("virtiofs device \(VirtioFSShare.deviceTag) is missing")
                    )
                    return
                }
                device.share = shareToApply
                mountsLock.withLock { mounts = next }
                continuation.resume()
            }
        }
    }

    func writeToGuest(_ data: Data) {
        guard !data.isEmpty else { return }
        serialInputWriter?.write(data)
    }

    func startConsoleServer(stateDirectory: URL, vmID: String) throws {
        let helpersDirectory = stateDirectory.appendingPathComponent("helpers", isDirectory: true)
        try FileManager.default.createDirectory(
            at: helpersDirectory,
            withIntermediateDirectories: true,
            attributes: [.posixPermissions: 0o700]
        )
        let path = helpersDirectory
            .appendingPathComponent("\(StateFileName.component(vmID)).console.sock")
            .path
        stopConsoleServer()
        if FileManager.default.fileExists(atPath: path) {
            guard Darwin.unlink(path) == 0 else {
                throw HelperError.system("unlink stale console socket", errno)
            }
        }
        let fd = Darwin.socket(AF_UNIX, SOCK_STREAM, 0)
        guard fd >= 0 else { throw HelperError.system("console socket", errno) }
        var address = sockaddr_un()
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        address.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: address.sun_path) else {
            Darwin.close(fd)
            throw HelperError.invalid("console socket path is too long")
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
            throw HelperError.system("bind console socket", code)
        }
        guard chmod(path, 0o600) == 0, Darwin.listen(fd, 4) == 0 else {
            let code = errno
            Darwin.close(fd)
            Darwin.unlink(path)
            throw HelperError.system("listen console socket", code)
        }
        consoleLock.withLock {
            consoleListener = fd
            consoleOwnsSocket = true
            consoleSocketPath = path
        }
        DispatchQueue.global().async { [weak self] in self?.consoleAcceptLoop(fd) }
    }

    func stopConsoleServer() {
        let state = consoleLock.withLock { () -> (Int32, Bool, String?, [Int32]) in
            let state = (consoleListener, consoleOwnsSocket, consoleSocketPath, consoleClients)
            consoleListener = -1
            consoleOwnsSocket = false
            consoleSocketPath = nil
            consoleClients = []
            return state
        }
        if state.0 >= 0 {
            Darwin.shutdown(state.0, SHUT_RDWR)
            Darwin.close(state.0)
        }
        for client in state.3 {
            Darwin.shutdown(client, SHUT_RDWR)
            Darwin.close(client)
        }
        if state.1, let path = state.2 {
            Darwin.unlink(path)
        }
    }

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        eventContinuation.yield(.stopped)
        eventContinuation.finish()
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        eventContinuation.yield(.failed(String(describing: error)))
        eventContinuation.finish()
    }

    private func startSerialTee() {
        serialOutputRead?.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty else {
                handle.readabilityHandler = nil
                return
            }
            self?.fanOutSerialOutput(data)
        }
    }

    private func fanOutSerialOutput(_ data: Data) {
        guard !data.isEmpty else { return }
        serialLogWriter?.write(data)
        let failed = consoleLock.withLock { () -> [Int32] in
            var failed: [Int32] = []
            for fd in consoleClients {
                if !Self.writeAll(data, to: fd) {
                    failed.append(fd)
                }
            }
            if !failed.isEmpty {
                consoleClients.removeAll { failed.contains($0) }
            }
            return failed
        }
        for fd in failed {
            Darwin.shutdown(fd, SHUT_RDWR)
        }
    }

    private func consoleAcceptLoop(_ fd: Int32) {
        while consoleLock.withLock({ consoleListener == fd }) {
            let client = Darwin.accept(fd, nil, nil)
            if client < 0 {
                if errno == EINTR { continue }
                return
            }
            guard peerUID(client) == geteuid() else {
                Darwin.close(client)
                continue
            }
            consoleLock.withLock { consoleClients.append(client) }
            DispatchQueue.global().async { [weak self] in
                self?.consoleClientLoop(client)
            }
        }
    }

    private func consoleClientLoop(_ fd: Int32) {
        var buffer = [UInt8](repeating: 0, count: 4_096)
        while true {
            let count = buffer.withUnsafeMutableBufferPointer { raw in
                Darwin.read(fd, raw.baseAddress, raw.count)
            }
            if count <= 0 { break }
            writeToGuest(Data(buffer[0..<count]))
        }
        consoleLock.withLock {
            consoleClients.removeAll { $0 == fd }
        }
        Darwin.close(fd)
    }

    private func peerUID(_ fd: Int32) -> uid_t? {
        var credentials = xucred()
        var length = socklen_t(MemoryLayout<xucred>.size)
        let result = withUnsafeMutablePointer(to: &credentials) {
            getsockopt(fd, SOL_LOCAL, LOCAL_PEERCRED, $0, &length)
        }
        return result == 0 ? credentials.cr_uid : nil
    }

    private static func writeAll(_ data: Data, to fd: Int32) -> Bool {
        data.withUnsafeBytes { raw in
            guard let base = raw.baseAddress else { return true }
            var offset = 0
            while offset < raw.count {
                let count = Darwin.write(fd, base.advanced(by: offset), raw.count - offset)
                if count <= 0 { return false }
                offset += count
            }
            return true
        }
    }

    private static func makeConfiguration(
        options: RunOptions,
        serialLogURL: URL,
        vmnetNICs: [HelperVmnetNIC]
    ) throws -> ConfigurationResources {
        guard FileManager.default.fileExists(atPath: options.diskURL.path) else {
            throw HelperError.invalid("missing raw boot disk: \(options.diskURL.path)")
        }
        try FileManager.default.createDirectory(
            at: options.bundleURL,
            withIntermediateDirectories: true
        )

        let configuration = VZVirtualMachineConfiguration()
        let minCPUs = VZVirtualMachineConfiguration.minimumAllowedCPUCount
        let maxCPUs = VZVirtualMachineConfiguration.maximumAllowedCPUCount
        guard options.cpuCount >= minCPUs, options.cpuCount <= maxCPUs else {
            throw HelperError.invalid(
                "cpuCount \(options.cpuCount) outside allowed range \(minCPUs)...\(maxCPUs)"
            )
        }
        let minMemory = VZVirtualMachineConfiguration.minimumAllowedMemorySize
        let maxMemory = VZVirtualMachineConfiguration.maximumAllowedMemorySize
        guard options.memorySize >= minMemory, options.memorySize <= maxMemory else {
            throw HelperError.invalid(
                "memorySize \(options.memorySize) outside allowed range \(minMemory)...\(maxMemory)"
            )
        }
        configuration.cpuCount = options.cpuCount
        configuration.memorySize = options.memorySize
        configuration.platform = VZGenericPlatformConfiguration()

        let bootLoader = VZEFIBootLoader()
        let nvramURL = options.bundleURL.appendingPathComponent("nvram.bin")
        if FileManager.default.fileExists(atPath: nvramURL.path) {
            bootLoader.variableStore = VZEFIVariableStore(url: nvramURL)
        } else {
            bootLoader.variableStore = try VZEFIVariableStore(creatingVariableStoreAt: nvramURL)
        }
        configuration.bootLoader = bootLoader

        let root = try VZDiskImageStorageDeviceAttachment(
            url: options.diskURL,
            readOnly: false,
            cachingMode: .cached,
            synchronizationMode: .fsync
        )
        var storage: [VZStorageDeviceConfiguration] = [
            VZVirtioBlockDeviceConfiguration(attachment: root),
        ]
        if let dataDiskURL = options.dataDiskURL {
            guard FileManager.default.fileExists(atPath: dataDiskURL.path) else {
                throw HelperError.invalid("missing data disk: \(dataDiskURL.path)")
            }
            let data = try VZDiskImageStorageDeviceAttachment(
                url: dataDiskURL,
                readOnly: false,
                cachingMode: .cached,
                synchronizationMode: .fsync
            )
            storage.append(VZVirtioBlockDeviceConfiguration(attachment: data))
        }
        if let cidataURL = options.cidataURL {
            guard FileManager.default.fileExists(atPath: cidataURL.path) else {
                throw HelperError.invalid("missing cidata image: \(cidataURL.path)")
            }
            let cidata = try VZDiskImageStorageDeviceAttachment(url: cidataURL, readOnly: true)
            storage.append(VZVirtioBlockDeviceConfiguration(attachment: cidata))
        }
        configuration.storageDevices = storage

        if vmnetNICs.isEmpty {
            // Standalone helper (no supervisor attachments): keep legacy NAT.
            let network = VZVirtioNetworkDeviceConfiguration()
            network.attachment = VZNATNetworkDeviceAttachment()
            if let macString = options.macAddress {
                guard let macAddress = VZMACAddress(string: macString) else {
                    throw HelperError.invalid("invalid MAC address: \(macString)")
                }
                network.macAddress = macAddress
            }
            configuration.networkDevices = [network]
        } else {
            var devices: [VZVirtioNetworkDeviceConfiguration] = []
            devices.reserveCapacity(vmnetNICs.count)
            for nic in vmnetNICs {
                let device = VZVirtioNetworkDeviceConfiguration()
                device.attachment = VZVmnetNetworkDeviceAttachment(network: nic.network)
                guard let macAddress = VZMACAddress(string: nic.macAddress) else {
                    throw HelperError.invalid(
                        "invalid MAC address for \(nic.networkName): \(nic.macAddress)"
                    )
                }
                device.macAddress = macAddress
                devices.append(device)
            }
            configuration.networkDevices = devices
        }
        configuration.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
        configuration.socketDevices = [VZVirtioSocketDeviceConfiguration()]

        var inputDescriptors: [Int32] = [0, 0]
        guard pipe(&inputDescriptors) == 0 else {
            throw HelperError.system("serial input pipe", errno)
        }
        let serialInput = FileHandle(fileDescriptor: inputDescriptors[0], closeOnDealloc: true)
        let serialInputWriter = FileHandle(fileDescriptor: inputDescriptors[1], closeOnDealloc: true)

        var outputDescriptors: [Int32] = [0, 0]
        guard pipe(&outputDescriptors) == 0 else {
            throw HelperError.system("serial output pipe", errno)
        }
        let serialOutputRead = FileHandle(fileDescriptor: outputDescriptors[0], closeOnDealloc: true)
        let serialOutputWrite = FileHandle(fileDescriptor: outputDescriptors[1], closeOnDealloc: true)

        FileManager.default.createFile(atPath: serialLogURL.path, contents: nil)
        guard chmod(serialLogURL.path, 0o600) == 0 else {
            throw HelperError.system("chmod serial log", errno)
        }
        let serialLogWriter = try FileHandle(forWritingTo: serialLogURL)
        try serialLogWriter.seekToEnd()

        let console = VZVirtioConsoleDeviceConfiguration()
        let port = VZVirtioConsolePortConfiguration()
        port.isConsole = true
        port.attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: serialInput,
            fileHandleForWriting: serialOutputWrite
        )
        console.ports[0] = port
        configuration.consoleDevices = [console]

        let virtiofs = try VirtioFSShare.makeDeviceConfiguration(
            mounts: options.mounts,
            bundleURL: options.bundleURL
        )
        configuration.directorySharingDevices = [virtiofs]

        try configuration.validate()
        return ConfigurationResources(
            configuration: configuration,
            serialInputWriter: serialInputWriter,
            serialOutputRead: serialOutputRead,
            serialLogWriter: serialLogWriter
        )
    }
}

private final class SocketConnectionWaiter: @unchecked Sendable {
    private let semaphore = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var result: Result<Int32, Error>?
    private var expired = false

    func complete(_ connectionResult: Result<VZVirtioSocketConnection, Error>) {
        lock.lock()
        defer { lock.unlock() }
        switch connectionResult {
        case let .success(connection):
            let duplicated = dup(connection.fileDescriptor)
            connection.close()
            if expired {
                if duplicated >= 0 { Darwin.close(duplicated) }
                return
            }
            if duplicated < 0 {
                result = .failure(GuestAgentError.unavailable("cannot duplicate vsock descriptor"))
            } else {
                result = .success(duplicated)
            }
        case let .failure(error):
            if expired { return }
            result = .failure(
                GuestAgentError.unavailable("connect failed: \(String(describing: error))")
            )
        }
        semaphore.signal()
    }

    func wait(timeout: TimeInterval) -> Bool {
        semaphore.wait(timeout: .now() + timeout) == .success
    }

    func expire() {
        lock.lock()
        expired = true
        if case let .success(fileDescriptor)? = result {
            Darwin.close(fileDescriptor)
            result = .failure(GuestAgentError.timeout("connect"))
        }
        lock.unlock()
    }

    func takeFileDescriptor() throws -> Int32 {
        lock.lock()
        defer { lock.unlock() }
        guard let result else {
            throw GuestAgentError.unavailable("connect completed without a result")
        }
        return try result.get()
    }
}

private struct ConfigurationResources {
    let configuration: VZVirtualMachineConfiguration
    let serialInputWriter: FileHandle
    let serialOutputRead: FileHandle
    let serialLogWriter: FileHandle
}

private func releaseOpaqueCF(_ pointer: OpaquePointer) {
    Unmanaged<AnyObject>.fromOpaque(UnsafeRawPointer(pointer)).release()
}
