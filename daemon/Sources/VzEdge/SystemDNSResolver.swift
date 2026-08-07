import Dispatch
import Foundation
import dnssd

struct SystemDNSRecord: Equatable, Sendable {
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
            _, flags, _, errorCode, _, rrtype, rrclass, rdlen, rdata, ttl, rawContext in
            guard let rawContext else { return }
            let target = Unmanaged<SystemDNSQueryBox>.fromOpaque(rawContext).takeUnretainedValue()
            guard errorCode == kDNSServiceErr_NoError else {
                target.fail()
                return
            }
            if flags & kDNSServiceFlagsAdd != 0, let rdata {
                target.append(SystemDNSRecord(
                    type: rrtype,
                    dnsClass: rrclass,
                    ttl: ttl,
                    rdata: Data(bytes: rdata, count: Int(rdlen))
                ))
            }
            if flags & kDNSServiceFlagsMoreComing == 0 {
                target.finish()
            }
        }
        let started = DNSServiceQueryRecord(
            &service,
            0,
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
        var response = Data()
        append16(read16(request, 0) ?? 0, to: &response)
        let requestFlags = read16(request, 2) ?? 0
        append16(0x8080 | (requestFlags & 0x0100), to: &response)
        append16(1, to: &response)
        append16(UInt16(clamping: records.count), to: &response)
        append16(0, to: &response)
        append16(0, to: &response)
        if request.count >= question.endOffset {
            response.append(request[12 ..< question.endOffset])
        }
        for record in records.prefix(Int(UInt16.max)) {
            append16(0xC00C, to: &response)
            append16(record.type, to: &response)
            append16(record.dnsClass, to: &response)
            append32(record.ttl, to: &response)
            append16(UInt16(clamping: record.rdata.count), to: &response)
            response.append(record.rdata.prefix(Int(UInt16.max)))
        }
        return response
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
