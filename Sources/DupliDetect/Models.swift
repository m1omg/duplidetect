import Foundation

/// File extensions DupliDetect will consider audio.
public enum AudioFormats {
    /// Handled by CoreAudio / AVFoundation.
    public static let native: Set<String> = [
        "wav", "wave", "aif", "aiff", "aifc",
        "mp3", "m4a", "m4b", "m4r", "aac", "adts",
        "caf", "flac", "alac", "au", "snd", "amr", "ac3", "sd2"
    ]
    /// Handled by the bundled stb_vorbis decoder.
    public static let vorbis: Set<String> = ["ogg", "oga"]

    /// macOS ships no decoder for these, so their audio cannot be compared.
    /// They are still scanned, because byte-identical copies are found by hashing.
    public static let byteComparableOnly: Set<String> = ["opus", "wma", "wv", "ape", "mka", "ra"]

    public static let all: Set<String> = native.union(vorbis).union(byteComparableOnly)

    /// Best guess at losslessness from the extension alone, for files that
    /// cannot be decoded. `nil` means the extension is a container that says
    /// nothing about the codec inside it, so no guess is possible.
    public static func losslessByExtension(_ url: URL) -> Bool? {
        switch url.pathExtension.lowercased() {
        case "flac", "alac", "wv", "ape", "wav", "wave", "aif", "aiff", "aifc":
            return true
        case "mp3", "aac", "adts", "opus", "wma", "ra", "amr", "ac3", "ogg", "oga":
            return false
        default:
            // caf, m4a, m4b, m4r, au, snd, mka: container only, contents unknown.
            return nil
        }
    }

    /// A human-readable name for the codec a file extension implies.
    public static func label(for url: URL) -> String {
        switch url.pathExtension.lowercased() {
        case "opus": return "Opus"
        case "wma": return "Windows Media Audio"
        case "wv": return "WavPack"
        case "ape": return "Monkey's Audio"
        case "mka": return "Matroska Audio"
        case "ra": return "RealAudio"
        default: return url.pathExtension.uppercased()
        }
    }

    public static func isAudio(_ url: URL) -> Bool {
        all.contains(url.pathExtension.lowercased())
    }
}

/// Everything DupliDetect knows about one file on disk.
public struct AudioFile: Identifiable, Hashable {
    public let id = UUID()
    public let url: URL
    public let byteSize: Int64
    public let modified: Date

    /// Filled in during probing; nil when the file could not be decoded.
    public var duration: Double?
    /// Seconds of actual audio, ignoring silent padding at either end.
    /// This is what "does this copy hold all the music?" should be judged on —
    /// container duration counts padding as content.
    public var contentDuration: Double?
    public var sampleRate: Double?
    public var bitDepth: Int?
    /// Set from the decoded codec. Nil until the file has been decoded.
    public var codecIsLossless: Bool?
    public var channelCount: Int?
    public var formatName: String?

    /// Filled in during the exact-match pass.
    public var contentHash: String?
    /// Filled in during the acoustic pass.
    public var fingerprint: Fingerprint?

    public init(url: URL, byteSize: Int64, modified: Date) {
        self.url = url
        self.byteSize = byteSize
        self.modified = modified
    }

    public var displayName: String { url.lastPathComponent }
    public var folderPath: String { url.deletingLastPathComponent().path }

    /// Approximate bitrate in kbps, derived from size and duration.
    public var bitrateKbps: Int? {
        guard let duration, duration > 0.05 else { return nil }
        return Int((Double(byteSize) * 8.0 / duration / 1000.0).rounded())
    }

    /// How the fidelity of this file should be described to a person.
    /// For lossless audio the bitrate is meaningless as a quality signal — it
    /// only reflects compression — so the sample rate and bit depth are shown.
    public var qualitySummary: String {
        if isLossless == true {
            var parts: [String] = []
            if let rate = sampleRate, rate > 0 {
                parts.append(String(format: "%.4g kHz", rate / 1000))
            }
            if let depth = bitDepth, depth > 0 {
                parts.append("\(depth)-bit")
            }
            if !parts.isEmpty { return parts.joined(separator: " · ") }
        }
        if let rate = bitrateKbps { return "\(rate) kbps" }
        return "—"
    }

    /// Whether this file's audio is stored without loss.
    ///
    /// Taken from the codec that was actually decoded, falling back to the
    /// extension only when the file could not be opened. The extension alone is
    /// not trustworthy: `.caf` and `.m4a` are containers that hold either lossy
    /// or lossless audio, so a 98 kbps AAC in a `.caf` would otherwise be
    /// ranked as though it were lossless. `nil` means genuinely unknown.
    public var isLossless: Bool? {
        codecIsLossless ?? AudioFormats.losslessByExtension(url)
    }

    /// Orders two files worst-to-best on audio fidelity alone.
    ///
    /// File size deliberately plays no part until everything else ties: a FLAC
    /// and a WAV made from the same master are the same audio, and the WAV
    /// being three times larger does not make it better. When fidelity really
    /// is equal, the smaller file is the more sensible copy to keep.
    public static func isLowerQuality(_ a: AudioFile, _ b: AudioFile) -> Bool {
        // A file that could not be decoded tells us nothing about its own
        // quality, so it never outranks one we were able to inspect.
        let aInspected = a.duration != nil, bInspected = b.duration != nil
        if aInspected != bInspected { return !aInspected }

        // Lossless beats lossy, whenever the codec is known for both files.
        if let aLossless = a.isLossless, let bLossless = b.isLossless, aLossless != bLossless {
            return !aLossless
        }

        // Bitrate only means anything for lossy audio; for lossless it just
        // reflects how well the encoder packed identical samples. Skip it as
        // soon as either file is known to be lossless.
        if a.isLossless != true && b.isLossless != true {
            let aRate = a.bitrateKbps ?? 0, bRate = b.bitrateKbps ?? 0
            if aRate != bRate { return aRate < bRate }
        }

        // Only compare an attribute when it is known for both files. Compressed
        // lossless formats report no bit depth, and treating "unknown" as zero
        // would make a FLAC lose to a WAV holding the very same audio.
        if let aRate = a.sampleRate, let bRate = b.sampleRate, aRate != bRate {
            return aRate < bRate
        }
        if let aDepth = a.bitDepth, let bDepth = b.bitDepth, aDepth != bDepth {
            return aDepth < bDepth
        }
        if let aChannels = a.channelCount, let bChannels = b.channelCount, aChannels != bChannels {
            return aChannels < bChannels
        }

        // Identical fidelity: prefer the copy that wastes less space, then
        // the one nearer the top of the folder tree, so the result is stable.
        if a.byteSize != b.byteSize { return a.byteSize > b.byteSize }
        return a.url.path.count > b.url.path.count
    }

    public func hash(into hasher: inout Hasher) { hasher.combine(id) }
    public static func == (a: AudioFile, b: AudioFile) -> Bool { a.id == b.id }
}

/// Why a set of files was grouped together.
public enum MatchKind: String {
    /// Byte-for-byte identical files.
    case exact
    /// Different bytes, same audio content (re-encode, different container, retagged).
    case similar

    public var label: String {
        switch self {
        case .exact: return "Identical files"
        case .similar: return "Same audio"
        }
    }
}

/// A set of files DupliDetect believes are duplicates of one another.
public struct DuplicateGroup: Identifiable {
    public let id = UUID()
    public let kind: MatchKind
    public var files: [AudioFile]
    /// Lowest pairwise confidence inside the group, 0...1.
    public let confidence: Double

    public init(kind: MatchKind, files: [AudioFile], confidence: Double) {
        self.kind = kind
        self.files = files
        self.confidence = confidence
    }

    /// Bytes that would be freed by keeping only one file.
    public var reclaimableBytes: Int64 {
        guard files.count > 1 else { return 0 }
        let keep = files.map(\.byteSize).max() ?? 0
        return files.reduce(0) { $0 + $1.byteSize } - keep
    }
}
