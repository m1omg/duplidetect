import Foundation
import CryptoKit

/// Byte-level hashing used for the exact-duplicate pass.
public enum FileHasher {

    /// SHA-256 of the whole file, read in chunks so large files never land in memory.
    public static func contentHash(of url: URL) throws -> String {
        let handle = try FileHandle(forReadingFrom: url)
        defer { try? handle.close() }

        var hasher = SHA256()
        let chunkSize = 1 << 20
        while true {
            let chunk = try handle.read(upToCount: chunkSize) ?? Data()
            if chunk.isEmpty { break }
            hasher.update(data: chunk)
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }
}
