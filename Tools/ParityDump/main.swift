import Foundation

// Parity harness: dumps the reference values the Rust port must reproduce.
// Floating-point values are emitted as raw bit patterns so comparison is exact
// and never routed through decimal formatting.

func bits(_ values: [Float]) -> [UInt32] { values.map { $0.bitPattern } }
func bits(_ value: Double) -> UInt64 { value.bitPattern }
func bits(_ value: Float) -> UInt32 { value.bitPattern }

/// Deterministic LCG, mirrored exactly in the Rust test suite.
struct LCG {
    var state: UInt64
    init(seed: UInt64) { state = seed }
    mutating func next() -> UInt32 {
        state = state &* 6364136223846793005 &+ 1442695040888963407
        return UInt32(truncatingIfNeeded: state >> 32)
    }
    /// Uniform in [-1, 1).
    mutating func sample() -> Float {
        Float(next()) / Float(UInt32.max) * 2 - 1
    }
}

func jsonString(_ any: Any) -> String {
    let data = try! JSONSerialization.data(withJSONObject: any, options: [.sortedKeys])
    return String(data: data, encoding: .utf8)!
}

// MARK: - primitives

func dumpPrimitives() {
    let engine = FingerprintEngine()

    var spectra: [[String: Any]] = []
    for (name, seed) in [("lcg1", UInt64(1)), ("lcg2", UInt64(0xDEADBEEF))] {
        var rng = LCG(seed: seed)
        let raw = (0..<FingerprintEngine.frameSize).map { _ in rng.sample() }
        // Exactly what fingerprint() does per frame: multiply by the window,
        // then take the spectrum. Element-wise f32 multiply is IEEE-exact, so a
        // plain loop matches vDSP_vmul bit for bit.
        let windowed = zip(raw, engine.window).map { $0 * $1 }
        var bands = [Float](repeating: 0, count: FingerprintEngine.bandCount)
        engine.spectrum(of: windowed, into: &bands)
        spectra.append(["name": name,
                        "seed": String(seed),
                        "inputBits": bits(raw).map(String.init),
                        "bandEnergyBits": bits(bands).map(String.init)])
    }

    // contentRange over shapes that exercise the silence trimmer.
    var ranges: [[String: Any]] = []
    func addRange(_ name: String, _ samples: [Float]) {
        let r = FingerprintEngine.contentRange(samples)
        ranges.append(["name": name, "count": samples.count,
                       "lower": r.lowerBound, "upper": r.upperBound])
    }
    addRange("allZero", [Float](repeating: 0, count: 1000))
    addRange("dc", [Float](repeating: 0.5, count: 1000))
    var padded = [Float](repeating: 0, count: 1000)
    for i in 300..<700 { padded[i] = 0.8 }
    addRange("padded", padded)
    var ramp = [Float](repeating: 0, count: 1000)
    for i in 0..<1000 { ramp[i] = Float(i) / 1000 }
    addRange("ramp", ramp)

    // shape() and shapeDistance() on fixed inputs.
    var rng = LCG(seed: 7)
    let totalsA = (0..<FingerprintEngine.bandCount).map { _ in Double(rng.sample()) * 5 - 5 }
    let totalsB = (0..<FingerprintEngine.bandCount).map { _ in Double(rng.sample()) * 5 - 5 }
    let shapeA = FingerprintEngine.shape(totals: totalsA, frames: 1)
    let shapeB = FingerprintEngine.shape(totals: totalsB, frames: 1)

    let out: [String: Any] = [
        "frameSize": FingerprintEngine.frameSize,
        "hopSize": FingerprintEngine.hopSize,
        "bandCount": FingerprintEngine.bandCount,
        "lowFrequency": FingerprintEngine.lowFrequency,
        "highFrequency": FingerprintEngine.highFrequency,
        "minimumFrames": FingerprintEngine.minimumFrames,
        "stationaryFlux": FingerprintEngine.stationaryFlux,
        "fingerprintRate": AudioDecoder.fingerprintRate,
        "windowBits": bits(engine.window).map(String.init),
        "bandEdges": engine.bandEdges,
        "spectra": spectra,
        "contentRanges": ranges,
        "shapeTotalsABits": totalsA.map { bits($0) }.map(String.init),
        "shapeTotalsBBits": totalsB.map { bits($0) }.map(String.init),
        "shapeA": shapeA.map { Int($0) },
        "shapeB": shapeB.map { Int($0) },
        "shapeDistanceBits": String(bits(FingerprintEngine.shapeDistance(shapeA, shapeB))),
        "medianOdd": String(bits(FingerprintEngine.median([3, 1, 2]))),
        "medianEven": String(bits(FingerprintEngine.median([4, 1, 3, 2]))),
    ]
    print(jsonString(out))
}

// MARK: - fingerprints from decoded audio

func dumpFingerprints(files: [String], pcmDir: String?) {
    let engine = FingerprintEngine()
    var entries: [[String: Any]] = []

    for path in files.sorted() {
        let url = URL(fileURLWithPath: path)
        var entry: [String: Any] = ["file": url.lastPathComponent]
        do {
            let audio = try AudioDecoder.decodeMono(url: url, maxSeconds: 120)
            let analysis = engine.analyze(samples: audio.samples)

            if let pcmDir {
                // Raw f32 little-endian: the exact buffer analyze() consumed.
                let out = URL(fileURLWithPath: pcmDir)
                    .appendingPathComponent(url.lastPathComponent + ".f32le")
                var samples = audio.samples
                let data = Data(bytes: &samples, count: samples.count * 4)
                try data.write(to: out)
                entry["pcm"] = out.lastPathComponent
                entry["pcmCount"] = samples.count
            }

            entry["values"] = analysis.fingerprint.values.map { String($0) }
            entry["shapeProfile"] = analysis.fingerprint.shapeProfile.map { Int($0) }
            entry["fluxBits"] = String(bits(analysis.fingerprint.flux))
            entry["leadingSilenceBits"] = String(bits(analysis.leadingSilence))
            entry["trailingSilenceBits"] = String(bits(analysis.trailingSilence))
            entry["isStationary"] = analysis.fingerprint.isStationary

            // Tier 1.5 metadata — nil-ness is significant and must survive.
            entry["sourceDuration"] = audio.sourceDuration
            entry["sourceSampleRate"] = audio.sourceSampleRate
            entry["sourceChannels"] = audio.sourceChannels
            entry["sourceBitDepth"] = audio.sourceBitDepth.map { $0 as Any } ?? NSNull()
            entry["isLossless"] = audio.isLossless
            entry["formatName"] = audio.formatName
            entry["wasTruncated"] = audio.wasTruncated
        } catch {
            entry["error"] = "\(error)"
        }
        entries.append(entry)
    }
    print(jsonString(["files": entries]))
}

// MARK: - canonical scan report

func dumpScan(directory: String, sensitivity: Double) {
    var options = ScanOptions()
    options.sensitivity = sensitivity
    options.minimumDuration = 0.5

    let result = Scanner().run(roots: [URL(fileURLWithPath: directory)], options: options) { _ in }

    var groups: [[String: Any]] = []
    for group in result.groups {
        let paths = group.files.map(\.displayName).sorted()
        groups.append([
            "kind": group.kind.rawValue,
            "paths": paths,
            "confidence": (group.confidence * 1000).rounded() / 1000,
            "reclaimableBytes": group.reclaimableBytes,
            "keepers": Dictionary(uniqueKeysWithValues: KeepRule.allCases.map {
                ($0.rawValue, $0.keeper(from: group.files)?.displayName ?? "")
            }),
        ])
    }
    // Canonical ordering so the report is comparable across runs.
    groups.sort {
        let a = ($0["kind"] as! String, ($0["paths"] as! [String]).joined(separator: "|"))
        let b = ($1["kind"] as! String, ($1["paths"] as! [String]).joined(separator: "|"))
        return a < b
    }

    print(jsonString([
        "filesScanned": result.filesScanned,
        "groups": groups,
        "skipped": result.filesSkipped.map { $0.url.lastPathComponent }.sorted(),
    ]))
}

// MARK: - entry point

let args = Array(CommandLine.arguments.dropFirst())
guard let mode = args.first else {
    FileHandle.standardError.write("""
    usage:
      paritydump primitives
      paritydump fp [--pcm-dir DIR] FILE...
      paritydump scan DIR SENSITIVITY

    """.data(using: .utf8)!)
    exit(2)
}

switch mode {
case "primitives":
    dumpPrimitives()
case "fp":
    var rest = Array(args.dropFirst())
    var pcmDir: String? = nil
    if rest.first == "--pcm-dir", rest.count >= 2 {
        pcmDir = rest[1]
        rest = Array(rest.dropFirst(2))
        try? FileManager.default.createDirectory(atPath: pcmDir!, withIntermediateDirectories: true)
    }
    dumpFingerprints(files: rest, pcmDir: pcmDir)
case "scan":
    guard args.count >= 3, let sensitivity = Double(args[2]) else { exit(2) }
    dumpScan(directory: args[1], sensitivity: sensitivity)
default:
    exit(2)
}
