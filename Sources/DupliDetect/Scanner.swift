import Foundation

public struct ScanOptions {
    public var includeSubfolders = true
    public var findExactDuplicates = true
    public var findSimilarAudio = true
    public var matchLevel: FingerprintMatcher.Level = .perfect
    /// Ignore files shorter than this; very short clips match each other easily.
    public var minimumDuration: Double = 2.0
    /// Only fingerprint the first N seconds of each file.
    public var fingerprintSeconds: Double = 120
    public var skipHiddenFiles = true

    public init() {}
}

public enum ScanPhase: Equatable {
    case idle
    case collecting
    case hashing(done: Int, total: Int)
    case fingerprinting(done: Int, total: Int)
    case matching
    case finished

    public var describes: String {
        switch self {
        case .idle: return ""
        case .collecting: return "Looking for audio files…"
        case .hashing(let done, let total): return "Checking for identical files… \(done) of \(total)"
        case .fingerprinting(let done, let total): return "Listening to audio… \(done) of \(total)"
        case .matching: return "Comparing fingerprints…"
        case .finished: return "Done"
        }
    }

    public var fraction: Double? {
        switch self {
        case .hashing(let done, let total),
             .fingerprinting(let done, let total):
            return total > 0 ? Double(done) / Double(total) : nil
        default: return nil
        }
    }
}

public struct ScanResult {
    public var groups: [DuplicateGroup] = []
    public var filesScanned = 0
    public var filesSkipped: [(url: URL, reason: String)] = []
    /// The level this scan actually ran at, so the results keep describing
    /// themselves correctly even if the setting is changed afterwards.
    public var matchLevel: FingerprintMatcher.Level = .perfect

    public var reclaimableBytes: Int64 { groups.reduce(0) { $0 + $1.reclaimableBytes } }
}

public final class Scanner {

    private let cancelled = Atomic(false)

    public init() {}

    public func cancel() { cancelled.value = true }

    public func run(roots: [URL],
                    options: ScanOptions,
                    progress: @escaping (ScanPhase) -> Void) -> ScanResult {
        cancelled.value = false
        var result = ScanResult()
        result.matchLevel = options.matchLevel

        progress(.collecting)
        var files = collect(roots: roots, options: options)
        result.filesScanned = files.count
        if cancelled.value || files.isEmpty { return result }

        // Groups are accumulated as index lists and only turned into
        // DuplicateGroups at the very end, once every file carries its metadata.
        var exactGroups: [[Int]] = []
        var similarGroups: [(indices: [Int], confidence: Double)] = []

        // Representative index -> every file byte-identical to it.
        var exactMembers: [Int: [Int]] = [:]

        // ---- Pass 1: byte-identical files -------------------------------
        if options.findExactDuplicates {
            exactGroups = findExactDuplicates(in: &files, progress: progress)
            for indices in exactGroups {
                guard let representative = indices.first else { continue }
                exactMembers[representative] = indices
            }
        }
        if cancelled.value { return result }

        // ---- Pass 2: same audio, different bytes ------------------------
        if options.findSimilarAudio {
            // Hashing already proved the members of an exact group identical,
            // so only one of each needs to be decoded and fingerprinted.
            let collapsed = Set(exactMembers.values.flatMap { $0 }).subtracting(exactMembers.keys)
            let representatives = files.indices.filter { !collapsed.contains($0) }

            let fingerprints = computeFingerprints(for: representatives,
                                                   files: &files,
                                                   options: options,
                                                   result: &result,
                                                   progress: progress)
            if cancelled.value { return result }

            // Byte-identical files share the representative's audio properties.
            for (representative, members) in exactMembers {
                for member in members where member != representative {
                    files[member].duration = files[representative].duration
                    files[member].contentDuration = files[representative].contentDuration
                    files[member].sampleRate = files[representative].sampleRate
                    files[member].bitDepth = files[representative].bitDepth
                    files[member].codecIsLossless = files[representative].codecIsLossless
                    files[member].channelCount = files[representative].channelCount
                    files[member].formatName = files[representative].formatName
                    files[member].fingerprint = files[representative].fingerprint
                }
            }

            progress(.matching)
            let matcherOptions = FingerprintMatcher.Options.forLevel(options.matchLevel)
            let matches = FingerprintMatcher.groups(fingerprints: fingerprints,
                                                    durations: files.map(\.contentDuration),
                                                    options: matcherOptions,
                                                    shouldCancel: { self.cancelled.value })

            // Expand each representative back into its exact-duplicate members.
            similarGroups = matches.map { (indices: $0.indices.flatMap { exactMembers[$0] ?? [$0] },
                                           confidence: $0.confidence) }
        }

        // An exact group wholly contained in a similar group is already
        // represented there; keeping both would double-count the wasted space.
        let absorbed = similarGroups.map { Set($0.indices) }
        for indices in exactGroups where !absorbed.contains(where: { $0.isSuperset(of: indices) }) {
            result.groups.append(DuplicateGroup(kind: .exact,
                                                files: indices.map { files[$0] },
                                                confidence: 1.0))
        }
        for group in similarGroups {
            result.groups.append(DuplicateGroup(kind: .similar,
                                                files: group.indices.sorted().map { files[$0] },
                                                confidence: group.confidence))
        }

        result.groups.sort { $0.reclaimableBytes > $1.reclaimableBytes }
        progress(.finished)
        return result
    }

    // MARK: - Collecting

    private func collect(roots: [URL], options: ScanOptions) -> [AudioFile] {
        var seen = Set<String>()
        var files: [AudioFile] = []
        let keys: [URLResourceKey] = [.isRegularFileKey, .fileSizeKey, .contentModificationDateKey, .isSymbolicLinkKey]

        for root in roots {
            if cancelled.value { break }
            var isDirectory: ObjCBool = false
            guard FileManager.default.fileExists(atPath: root.path, isDirectory: &isDirectory) else { continue }

            if !isDirectory.boolValue {
                append(root, to: &files, seen: &seen, keys: keys)
                continue
            }

            var enumeratorOptions: FileManager.DirectoryEnumerationOptions = [.skipsPackageDescendants]
            if options.skipHiddenFiles { enumeratorOptions.insert(.skipsHiddenFiles) }
            if !options.includeSubfolders { enumeratorOptions.insert(.skipsSubdirectoryDescendants) }

            guard let enumerator = FileManager.default.enumerator(at: root,
                                                                  includingPropertiesForKeys: keys,
                                                                  options: enumeratorOptions) else { continue }
            for case let url as URL in enumerator {
                if cancelled.value { break }
                append(url, to: &files, seen: &seen, keys: keys)
            }
        }
        return files
    }

    private func append(_ url: URL, to files: inout [AudioFile], seen: inout Set<String>, keys: [URLResourceKey]) {
        guard AudioFormats.isAudio(url) else { return }
        guard let values = try? url.resourceValues(forKeys: Set(keys)),
              values.isRegularFile == true,
              let size = values.fileSize, size > 0 else { return }
        // Resolve so the same file reached by two paths is only counted once.
        let identity = url.resolvingSymlinksInPath().standardizedFileURL.path
        guard seen.insert(identity).inserted else { return }
        var file = AudioFile(url: url,
                             byteSize: Int64(size),
                             modified: values.contentModificationDate ?? .distantPast)
        // A sensible name to show even if the file is never decoded.
        file.formatName = AudioFormats.label(for: url)
        files.append(file)
    }

    // MARK: - Pass 1

    private func findExactDuplicates(in files: inout [AudioFile],
                                     progress: @escaping (ScanPhase) -> Void) -> [[Int]] {
        // Only files sharing a byte size can possibly be identical, so hash nothing else.
        var bySize: [Int64: [Int]] = [:]
        for index in files.indices { bySize[files[index].byteSize, default: []].append(index) }
        let candidates = bySize.values.filter { $0.count > 1 }.flatMap { $0 }.sorted()
        guard !candidates.isEmpty else { return [] }

        // Snapshot the URLs; the worker closures must not touch `files` itself.
        let urls = candidates.map { files[$0].url }
        let total = candidates.count
        progress(.hashing(done: 0, total: total))

        let done = Atomic(0)
        let hashes = Atomic([Int: String]())
        let group = DispatchGroup()
        let queue = DispatchQueue(label: "duplidetect.hash", attributes: .concurrent)
        let workerCount = max(1, min(total, ProcessInfo.processInfo.activeProcessorCount))

        for worker in 0..<workerCount {
            queue.async(group: group) {
                var slot = worker
                while slot < total {
                    defer { slot += workerCount }
                    if self.cancelled.value { return }
                    if let hash = try? FileHasher.contentHash(of: urls[slot]) {
                        hashes.mutate { $0[candidates[slot]] = hash }
                    }
                    let count = self.increment(done)
                    if count % 8 == 0 || count == total {
                        progress(.hashing(done: count, total: total))
                    }
                }
            }
        }
        group.wait()
        if cancelled.value { return [] }

        let resolved = hashes.value
        for (index, hash) in resolved { files[index].contentHash = hash }

        var byHash: [String: [Int]] = [:]
        for (index, hash) in resolved { byHash[hash, default: []].append(index) }
        return byHash.values.filter { $0.count > 1 }.map { $0.sorted() }
    }

    private func increment(_ counter: Atomic<Int>) -> Int {
        var value = 0
        counter.mutate { $0 += 1; value = $0 }
        return value
    }

    // MARK: - Pass 2

    private func computeFingerprints(for indices: [Int],
                                     files: inout [AudioFile],
                                     options: ScanOptions,
                                     result: inout ScanResult,
                                     progress: @escaping (ScanPhase) -> Void) -> [Fingerprint?] {

        var fingerprints = [Fingerprint?](repeating: nil, count: files.count)
        let total = indices.count
        guard total > 0 else { return fingerprints }
        progress(.fingerprinting(done: 0, total: total))

        let done = Atomic(0)
        let collected = Atomic([Int: (AudioAnalysis, DecodedAudio)]())
        let skipped = Atomic([(URL, String)]())

        let workerCount = max(1, min(indices.count, ProcessInfo.processInfo.activeProcessorCount))
        let queue = DispatchQueue(label: "duplidetect.fingerprint", attributes: .concurrent)
        let group = DispatchGroup()
        let urls = indices.map { files[$0].url }

        for worker in 0..<workerCount {
            queue.async(group: group) {
                // One FFT setup per worker; the setup owns scratch buffers.
                let engine = FingerprintEngine()
                var slot = worker
                while slot < total {
                    defer { slot += workerCount }
                    if self.cancelled.value { return }

                    let index = indices[slot]
                    do {
                        let audio = try AudioDecoder.decodeMono(url: urls[slot],
                                                                maxSeconds: options.fingerprintSeconds)
                        if audio.sourceDuration >= options.minimumDuration {
                            let analysis = engine.analyze(samples: audio.samples)
                            collected.mutate { $0[index] = (analysis, audio) }
                        } else {
                            skipped.mutate { $0.append((urls[slot], "shorter than \(Int(options.minimumDuration))s")) }
                        }
                    } catch {
                        skipped.mutate { $0.append((urls[slot], Self.describe(error, url: urls[slot]))) }
                    }

                    let count = self.increment(done)
                    if count % 4 == 0 || count == total {
                        progress(.fingerprinting(done: count, total: total))
                    }
                }
            }
        }
        group.wait()

        for (index, payload) in collected.value {
            let (analysis, audio) = payload
            fingerprints[index] = analysis.fingerprint
            files[index].fingerprint = analysis.fingerprint
            files[index].duration = audio.sourceDuration
            files[index].sampleRate = audio.sourceSampleRate
            files[index].bitDepth = audio.sourceBitDepth
            files[index].codecIsLossless = audio.isLossless
            files[index].channelCount = audio.sourceChannels
            files[index].formatName = audio.formatName

            // Padding is not content. When the decode stopped early the tail was
            // never seen, so only the head silence can be discounted.
            let trailing = audio.wasTruncated ? 0 : analysis.trailingSilence
            files[index].contentDuration = max(0, audio.sourceDuration - analysis.leadingSilence - trailing)
        }
        result.filesSkipped.append(contentsOf: skipped.value.map { (url: $0.0, reason: $0.1) })
        return fingerprints
    }

    private static func describe(_ error: Error, url: URL) -> String {
        if AudioFormats.byteComparableOnly.contains(url.pathExtension.lowercased()) {
            return "macOS has no \(AudioFormats.label(for: url)) decoder — only exact copies of this file can be found"
        }
        switch error {
        case AudioDecodeError.unsupported(let detail): return "could not be decoded (\(detail))"
        case AudioDecodeError.emptyAudio: return "contains no audio"
        case AudioDecodeError.decodeFailed(let detail): return "decode failed (\(detail))"
        default: return error.localizedDescription
        }
    }
}

/// Small lock-protected box; the scan runs across several queues.
public final class Atomic<Value> {
    private var storage: Value
    private let lock = NSLock()

    public init(_ value: Value) { storage = value }

    public var value: Value {
        get { lock.lock(); defer { lock.unlock() }; return storage }
        set { lock.lock(); storage = newValue; lock.unlock() }
    }

    public func mutate(_ body: (inout Value) -> Void) {
        lock.lock(); body(&storage); lock.unlock()
    }
}
