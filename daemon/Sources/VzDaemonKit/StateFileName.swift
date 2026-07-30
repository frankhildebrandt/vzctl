import Foundation

public enum StateFileName {
    public static func component(_ value: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "._-"))
        let prefix = value.unicodeScalars.map {
            allowed.contains($0) ? Character(String($0)) : "_"
        }
        var hash: UInt64 = 0xcbf29ce484222325
        for byte in value.utf8 {
            hash ^= UInt64(byte)
            hash &*= 0x100000001b3
        }
        return "\(String(prefix).prefix(64))-\(String(hash, radix: 16))"
    }
}
