import Dispatch
import Foundation
import dnssd

struct SystemDNSRecord: Equatable, Sendable {
    var name: String
    var type: UInt16
    var dnsClass: UInt16
    var ttl: UInt32
    var rdata: Data
}

enum SystemDNSResolver {
    static func resolve(
        request: Data,
        question: DNSQuestion,
        timeout: TimeInterval = 2
    ) -> Data? {
        guard question.dnsClass == UInt16(kDNSServiceClass_IN) else { return nil }
        let box = SystemDNSQueryBox()
        let context = Unmanaged.passRetained(box)
        defer { context.release() }
        var service: DNSServiceRef?
        let callback: DNSServiceQueryRecordReply = {
            _, flags, _, errorCode, fullname, rrtype, rrclass, rdlen, rdata, ttl, rawContext in
            guard let rawContext else { return }
            let target = Unmanaged<SystemDNSQueryBox>.fromOpaque(rawContext).takeUnretainedValue()
            switch Int(errorCode) {
            case kDNSServiceErr_NoError:
                if flags & kDNSServiceFlagsAdd != 0 {
                    guard let fullname, let rdata else {
                        target.fail()
                        return
                    }
                    target.append(SystemDNSRecord(
                        name: String(cString: fullname),
                        type: rrtype,
                        dnsClass: rrclass,
                        ttl: ttl,
                        rdata: Data(bytes: rdata, count: Int(rdlen))
                    ))
                }
                if flags & kDNSServiceFlagsMoreComing == 0 {
                    target.finish()
                }
            case kDNSServiceErr_NoSuchRecord:
                // Negative answer (NXDOMAIN/NODATA). Complete with the records
                // collected so far instead of failing into SERVFAIL — clients
                // treat an empty NOERROR answer as NODATA and move on.
                if flags & kDNSServiceFlagsMoreComing == 0 {
                    target.finish()
                }
            default:
                target.fail()
            }
        }
        // ReturnIntermediates delivers negative answers (and CNAME chains)
        // through the callback instead of leaving the query pending until the
        // wait() timeout expires.
        let started = DNSServiceQueryRecord(
            &service,
            DNSServiceFlags(kDNSServiceFlagsReturnIntermediates),
            0,
            question.name,
            question.type,
            question.dnsClass,
            callback,
            context.toOpaque()
        )
        guard started == kDNSServiceErr_NoError, let service else { return nil }
        let queue = DispatchQueue(label: "vzctl.dns.system-query")
        guard DNSServiceSetDispatchQueue(service, queue) == kDNSServiceErr_NoError else {
            DNSServiceRefDeallocate(service)
            return nil
        }
        let records = box.wait(timeout: timeout)
        DNSServiceRefDeallocate(service)
        queue.sync {}
        guard let records else { return nil }
        return response(request: request, question: question, records: records)
    }

    static func response(
        request: Data,
        question: DNSQuestion,
        records: [SystemDNSRecord]
    ) -> Data {
        // DNS-SD may deliver CNAME intermediates and terminal addresses with
        // different owners. Repointing every RR to the question corrupts the chain.
        var owners: [Data] = []
        for record in records {
            guard let owner = encodeOwner(record.name), record.rdata.count <= Int(UInt16.max) else {
                return failureResponse(request: request, question: question)
            }
            owners.append(owner)
        }
        guard records.count <= Int(UInt16.max) else {
            return failureResponse(request: request, question: question)
        }
        var response = Data()
        append16(read16(request, 0) ?? 0, to: &response)
        let requestFlags = read16(request, 2) ?? 0
        append16(0x8080 | (requestFlags & 0x0100), to: &response)
        append16(1, to: &response)
        append16(UInt16(records.count), to: &response)
        append16(0, to: &response)
        append16(0, to: &response)
        if request.count >= question.endOffset {
            response.append(request[12 ..< question.endOffset])
        }
        for (record, owner) in zip(records, owners) {
            response.append(owner)
            append16(record.type, to: &response)
            append16(record.dnsClass, to: &response)
            append32(record.ttl, to: &response)
            append16(UInt16(record.rdata.count), to: &response)
            response.append(record.rdata)
        }
        return response
    }

    private static func failureResponse(request: Data, question: DNSQuestion) -> Data {
        var packet = response(request: request, question: question, records: [])
        packet[3] |= 2 // SERVFAIL, never send a partially encoded answer.
        return packet
    }

    /// DNSServiceQueryRecord supplies escaped UTF-8 presentation names: \ddd
    /// encodes one decimal byte; any other escaped character is literal.
    private static func encodeOwner(_ name: String) -> Data? {
        if name == "." { return Data([0]) }
        let bytes = Array(name.utf8)
        guard !bytes.isEmpty else { return nil }
        var wire = Data()
        var label: [UInt8] = []
        var index = 0
        while index < bytes.count {
            let byte = bytes[index]
            if byte == 46 { // Unescaped label separator.
                guard !label.isEmpty else { return nil }
                wire.append(UInt8(label.count))
                wire.append(contentsOf: label)
                label.removeAll(keepingCapacity: true)
            } else if byte == 92 {
                index += 1
                guard index < bytes.count else { return nil }
                var escaped = bytes[index]
                if index + 2 < bytes.count,
                   (48...57).contains(bytes[index]),
                   (48...57).contains(bytes[index + 1]),
                   (48...57).contains(bytes[index + 2]) {
                    let value = Int(bytes[index] - 48) * 100
                        + Int(bytes[index + 1] - 48) * 10 + Int(bytes[index + 2] - 48)
                    if value <= 255 {
                        escaped = UInt8(value)
                        index += 2
                    }
                }
                label.append(escaped)
            } else {
                guard byte != 0 else { return nil }
                label.append(byte)
            }
            guard label.count <= 63, wire.count + label.count <= 254 else { return nil }
            index += 1
        }
        if !label.isEmpty {
            wire.append(UInt8(label.count))
            wire.append(contentsOf: label)
        }
        wire.append(0)
        guard wire.count <= 255 else { return nil }
        return wire
    }

    private static func read16(_ data: Data, _ offset: Int) -> UInt16? {
        guard offset + 2 <= data.count else { return nil }
        return UInt16(data[offset]) << 8 | UInt16(data[offset + 1])
    }

    private static func append16(_ value: UInt16, to data: inout Data) {
        data.append(UInt8((value >> 8) & 0xFF))
        data.append(UInt8(value & 0xFF))
    }

    private static func append32(_ value: UInt32, to data: inout Data) {
        data.append(UInt8((value >> 24) & 0xFF))
        data.append(UInt8((value >> 16) & 0xFF))
        data.append(UInt8((value >> 8) & 0xFF))
        data.append(UInt8(value & 0xFF))
    }
}

private final class SystemDNSQueryBox: @unchecked Sendable {
    private let lock = NSLock()
    private let semaphore = DispatchSemaphore(value: 0)
    private var records: [SystemDNSRecord] = []
    private var completed = false
    private var failed = false

    func append(_ record: SystemDNSRecord) {
        lock.withLock {
            guard !completed else { return }
            records.append(record)
        }
    }

    func finish() {
        let signal = lock.withLock { () -> Bool in
            guard !completed else { return false }
            completed = true
            return true
        }
        if signal { semaphore.signal() }
    }

    func fail() {
        lock.withLock { failed = true }
        finish()
    }

    func wait(timeout: TimeInterval) -> [SystemDNSRecord]? {
        guard semaphore.wait(timeout: .now() + timeout) == .success else { return nil }
        return lock.withLock { failed ? nil : records }
    }
}
