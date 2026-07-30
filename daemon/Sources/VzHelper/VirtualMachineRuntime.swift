import Darwin
import Dispatch
import Foundation
import VzDaemonKit
@preconcurrency import Virtualization

enum VirtualMachineEvent: Sendable {
    case stopped
    case failed(String)
}

final class VirtualMachineRuntime: NSObject, VZVirtualMachineDelegate, @unchecked Sendable {
    let serialLogURL: URL

    private let queue: DispatchQueue
    private let virtualMachine: VZVirtualMachine
    private let eventStream: AsyncStream<VirtualMachineEvent>
    private let eventContinuation: AsyncStream<VirtualMachineEvent>.Continuation
    private var serialInputWriter: FileHandle?
    private var serialOutput: FileHandle?

    init(options: RunOptions) throws {
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
        queue = DispatchQueue(label: "vzctl.helper.\(StateFileName.component(options.vmID))")

        let pair = AsyncStream<VirtualMachineEvent>.makeStream()
        eventStream = pair.stream
        eventContinuation = pair.continuation

        let resources = try Self.makeConfiguration(options: options, serialLogURL: serialLogURL)
        virtualMachine = VZVirtualMachine(configuration: resources.configuration, queue: queue)
        serialInputWriter = resources.serialInputWriter
        serialOutput = resources.serialOutput
        super.init()
        virtualMachine.delegate = self
    }

    func start() async throws {
        try await withCheckedThrowingContinuation { continuation in
            queue.async { [self] in
                virtualMachine.start { continuation.resume(with: $0) }
            }
        }
    }

    func stop() async {
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

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        eventContinuation.yield(.stopped)
        eventContinuation.finish()
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        eventContinuation.yield(.failed(String(describing: error)))
        eventContinuation.finish()
    }

    private static func makeConfiguration(
        options: RunOptions,
        serialLogURL: URL
    ) throws -> ConfigurationResources {
        guard FileManager.default.fileExists(atPath: options.diskURL.path) else {
            throw HelperError.invalid("missing raw boot disk: \(options.diskURL.path)")
        }
        try FileManager.default.createDirectory(
            at: options.bundleURL,
            withIntermediateDirectories: true
        )

        let configuration = VZVirtualMachineConfiguration()
        configuration.cpuCount = 2
        configuration.memorySize = 1 * 1024 * 1024 * 1024
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

        let network = VZVirtioNetworkDeviceConfiguration()
        network.attachment = VZNATNetworkDeviceAttachment()
        if let macString = options.macAddress {
            guard let macAddress = VZMACAddress(string: macString) else {
                throw HelperError.invalid("invalid MAC address: \(macString)")
            }
            network.macAddress = macAddress
        }
        configuration.networkDevices = [network]
        configuration.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
        configuration.socketDevices = [VZVirtioSocketDeviceConfiguration()]

        var descriptors: [Int32] = [0, 0]
        guard pipe(&descriptors) == 0 else {
            throw HelperError.system("serial pipe", errno)
        }
        let serialInput = FileHandle(fileDescriptor: descriptors[0], closeOnDealloc: true)
        let serialInputWriter = FileHandle(fileDescriptor: descriptors[1], closeOnDealloc: true)
        FileManager.default.createFile(atPath: serialLogURL.path, contents: nil)
        guard chmod(serialLogURL.path, 0o600) == 0 else {
            throw HelperError.system("chmod serial log", errno)
        }
        let serialOutput = try FileHandle(forWritingTo: serialLogURL)
        try serialOutput.seekToEnd()

        let console = VZVirtioConsoleDeviceConfiguration()
        let port = VZVirtioConsolePortConfiguration()
        port.isConsole = true
        port.attachment = VZFileHandleSerialPortAttachment(
            fileHandleForReading: serialInput,
            fileHandleForWriting: serialOutput
        )
        console.ports[0] = port
        configuration.consoleDevices = [console]

        try configuration.validate()
        return ConfigurationResources(
            configuration: configuration,
            serialInputWriter: serialInputWriter,
            serialOutput: serialOutput
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
    let serialOutput: FileHandle
}
