import Foundation

/// Finds files whose fingerprints line up, allowing for different encodings,
/// different amounts of leading silence, and one file being a clip of another.
public enum FingerprintMatcher {

    /// Sub-fingerprints are indexed in two 16-bit halves. A pair only has to
    /// agree on one half to become a candidate, which keeps recall high even
    /// when a lossy re-encode flips a scattering of bits.
    private static let indexStride = 2
    /// Aligned frames a pair needs before it is even scored.
    private static let minimumVotes = 6

    public struct Options {
        /// Highest bit-error rate still considered the same audio.
        /// Same master re-encoded typically lands under 0.15; unrelated audio sits near 0.5.
        public var maximumBitErrorRate: Double = 0.22
        /// Overlap required, as a fraction of the shorter fingerprint.
        public var minimumOverlapFraction: Double = 0.25
        /// Overlap required in absolute terms.
        public var minimumOverlapSeconds: Double = 3.0
        /// Largest spectral-shape difference still considered the same audio,
        /// used only for stationary files. Measured across codecs, two encodes
        /// of one tone stay under 0.14 while different tones exceed 0.60.
        public var maximumShapeDistance: Double = 0.27
        /// How much two stationary files may differ in length, as a fraction of
        /// the longer one. The shape template describes timbre and says nothing
        /// about duration, so without this a ten-second tone matches a
        /// ten-minute one.
        public var stationaryDurationTolerance: Double = 0.10
        /// Absolute slack, so encoder padding never breaks a short clip.
        public var stationaryDurationSlack: Double = 0.5

        public init() {}

        public static func forSensitivity(_ sensitivity: Double) -> Options {
            var options = Options()
            // sensitivity 0 = only near-identical audio, 1 = catch loose matches.
            let amount = max(0, min(1, sensitivity))
            options.maximumBitErrorRate = 0.12 + 0.18 * amount
            options.maximumShapeDistance = 0.20 + 0.20 * amount
            return options
        }
    }

    /// Whether two files may be called duplicates given their lengths.
    ///
    /// Only stationary audio is constrained. Normal recordings are allowed to
    /// differ freely in length, because matching a short excerpt against the
    /// full recording is a genuine result. For a held tone it is not: every
    /// second sounds like every other, so a ten-second tone aligns perfectly
    /// inside a ten-minute one and "excerpt of" stops meaning anything.
    static func lengthsAllowMatch(_ a: Fingerprint, _ b: Fingerprint,
                                  lengthA: Double, lengthB: Double,
                                  options: Options) -> Bool {
        guard a.isStationary, b.isStationary else { return true }
        let longer = max(lengthA, lengthB), shorter = min(lengthA, lengthB)
        let allowed = max(options.stationaryDurationSlack,
                          longer * options.stationaryDurationTolerance)
        return longer - shorter <= allowed
    }

    struct Candidate: Hashable {
        let a: Int
        let b: Int
    }

    /// Returns groups of indices into `fingerprints` that match one another,
    /// paired with the weakest similarity inside each group.
    /// `durations` holds the seconds of real audio in each file, used to keep
    /// stationary matches from ignoring length. Pass nil where it is unknown.
    public static func groups(fingerprints: [Fingerprint?],
                              durations: [Double?] = [],
                              options: Options,
                              shouldCancel: () -> Bool = { false }) -> [(indices: [Int], confidence: Double)] {

        let usable = fingerprints.indices.filter { fingerprints[$0]?.isUsable == true }
        guard usable.count > 1 else { return [] }

        // Seconds of real audio per file, falling back to the fingerprint's own
        // span when the caller did not supply it.
        func length(of index: Int) -> Double {
            if durations.indices.contains(index), let known = durations[index] { return known }
            return fingerprints[index]?.duration ?? 0
        }

        // Inverted index over both halves of each sub-fingerprint.
        var index: [UInt32: [Int32]] = [:]
        var owner: [Int32] = []          // entry -> file index
        var position: [Int32] = []       // entry -> frame index
        owner.reserveCapacity(usable.count * 256)
        position.reserveCapacity(usable.count * 256)

        for file in usable {
            guard let values = fingerprints[file]?.values else { continue }
            var frame = 0
            while frame < values.count {
                let value = values[frame]
                let entry = Int32(owner.count)
                owner.append(Int32(file))
                position.append(Int32(frame))
                let low = value & 0xFFFF
                let high = (value >> 16) | 0x1_0000   // tag so halves never collide
                index[low, default: []].append(entry)
                index[high, default: []].append(entry)
                frame += indexStride
            }
        }

        // Vote for (pair, offset) combinations.
        var votes: [Candidate: [Int32: Int]] = [:]
        for (_, entries) in index {
            if shouldCancel() { return [] }
            // A hash shared by half the library carries no information.
            guard entries.count > 1, entries.count <= 400 else { continue }
            for i in 0..<entries.count {
                let entryA = entries[i]
                for j in (i + 1)..<entries.count {
                    let entryB = entries[j]
                    let fileA = owner[Int(entryA)], fileB = owner[Int(entryB)]
                    if fileA == fileB { continue }
                    let (low, high, offset) = fileA < fileB
                        ? (fileA, fileB, position[Int(entryA)] - position[Int(entryB)])
                        : (fileB, fileA, position[Int(entryB)] - position[Int(entryA)])
                    votes[Candidate(a: Int(low), b: Int(high)), default: [:]][offset, default: 0] += 1
                }
            }
        }

        // Verify each candidate at its best-supported offset.
        var union = UnionFind(count: fingerprints.count)
        var pairScores: [Candidate: Double] = [:]

        for (candidate, offsets) in votes {
            if shouldCancel() { return [] }
            guard let best = offsets.max(by: { $0.value < $1.value }), best.value >= minimumVotes else { continue }
            guard let a = fingerprints[candidate.a], let b = fingerprints[candidate.b] else { continue }

            guard lengthsAllowMatch(a, b,
                                    lengthA: length(of: candidate.a),
                                    lengthB: length(of: candidate.b),
                                    options: options) else { continue }

            let (errorRate, overlap) = compare(a.values, b.values, offset: Int(best.key))
            guard overlap > 0 else { continue }

            let overlapSeconds = Double(overlap) * Fingerprint.secondsPerFrame
            let shorter = min(a.values.count, b.values.count)
            guard overlapSeconds >= options.minimumOverlapSeconds,
                  Double(overlap) >= Double(shorter) * options.minimumOverlapFraction,
                  errorRate <= options.maximumBitErrorRate else { continue }

            // Map bit-error rate onto a 0...1 confidence; 0.5 is pure chance.
            let confidence = max(0, min(1, 1 - errorRate / 0.5))
            pairScores[candidate] = confidence
            union.merge(candidate.a, candidate.b)
        }

        // Stationary audio — a held tone, a drone, a test signal — defeats the
        // pass above: with nothing changing between frames, every bit in
        // `values` is decided by rounding noise rather than by the audio, and
        // two encodes of one file score no better than two unrelated tracks.
        // Those files are compared on spectral shape instead.
        let stationary = usable.filter {
            guard let print = fingerprints[$0] else { return false }
            return print.isStationary && !print.shapeProfile.isEmpty
        }
        if stationary.count > 1 {
            for i in 0..<stationary.count {
                if shouldCancel() { return [] }
                for j in (i + 1)..<stationary.count {
                    let first = stationary[i], second = stationary[j]
                    guard let a = fingerprints[first], let b = fingerprints[second] else { continue }

                    // The template describes timbre and says nothing about how
                    // long the sound is held, so length is checked separately.
                    guard lengthsAllowMatch(a, b,
                                            lengthA: length(of: first),
                                            lengthB: length(of: second),
                                            options: options) else { continue }

                    let distance = FingerprintEngine.shapeDistance(a.shapeProfile, b.shapeProfile)
                    guard distance <= options.maximumShapeDistance else { continue }

                    let candidate = Candidate(a: min(first, second), b: max(first, second))
                    let confidence = max(0, min(1, 1 - distance / 0.6))
                    pairScores[candidate] = max(pairScores[candidate] ?? 0, confidence)
                    union.merge(first, second)
                }
            }
        }

        var buckets: [Int: [Int]] = [:]
        for file in usable where pairScores.keys.contains(where: { $0.a == file || $0.b == file }) {
            buckets[union.find(file), default: []].append(file)
        }

        return buckets.values.compactMap { members in
            guard members.count > 1 else { return nil }
            let sorted = members.sorted()
            var weakest = 1.0
            for i in 0..<sorted.count {
                for j in (i + 1)..<sorted.count {
                    if let score = pairScores[Candidate(a: sorted[i], b: sorted[j])] {
                        weakest = min(weakest, score)
                    }
                }
            }
            // Groups linked transitively may have no direct score for every pair.
            if weakest == 1.0, let any = pairScores.first(where: { sorted.contains($0.key.a) })?.value {
                weakest = any
            }
            return (sorted, weakest)
        }
    }

    /// Bit-error rate between two fingerprint sequences at a fixed frame offset.
    /// `offset` is how far `a` leads `b`.
    private static func compare(_ a: [UInt32], _ b: [UInt32], offset: Int) -> (errorRate: Double, overlap: Int) {
        let startA = max(0, offset)
        let startB = max(0, -offset)
        let overlap = min(a.count - startA, b.count - startB)
        guard overlap > 0 else { return (1, 0) }

        var differingBits = 0
        for i in 0..<overlap {
            differingBits += (a[startA + i] ^ b[startB + i]).nonzeroBitCount
        }
        return (Double(differingBits) / Double(overlap * 32), overlap)
    }
}

/// Standard union-find, used to fold matching pairs into groups.
struct UnionFind {
    private var parent: [Int]
    private var rank: [Int]

    init(count: Int) {
        parent = Array(0..<count)
        rank = [Int](repeating: 0, count: count)
    }

    mutating func find(_ x: Int) -> Int {
        var root = x
        while parent[root] != root { root = parent[root] }
        var current = x
        while parent[current] != root {
            let next = parent[current]
            parent[current] = root
            current = next
        }
        return root
    }

    mutating func merge(_ x: Int, _ y: Int) {
        let a = find(x), b = find(y)
        guard a != b else { return }
        if rank[a] < rank[b] { parent[a] = b }
        else if rank[a] > rank[b] { parent[b] = a }
        else { parent[b] = a; rank[a] += 1 }
    }
}
