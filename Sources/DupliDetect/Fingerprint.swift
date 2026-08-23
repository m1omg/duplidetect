import Foundation
import Accelerate

/// A compact acoustic summary of a file: one 32-bit sub-fingerprint per frame.
///
/// The scheme is the Haitsma-Kalker robust hash. For each frame the spectrum is
/// folded into 33 logarithmically spaced bands, and each bit records whether the
/// energy difference between neighbouring bands rose or fell relative to the
/// previous frame. Because every bit comes from a *difference of differences*,
/// the result is invariant to overall volume and largely unaffected by the
/// artefacts of lossy encoding — two encodes of the same master produce nearly
/// the same bits even at very different bitrates.
public struct Fingerprint: Hashable {
    public let values: [UInt32]

    /// The average spectral shape of the file: one quantised value per band,
    /// measured relative to the loudest band so it is volume-invariant.
    /// Describes *what* the audio sounds like rather than how it changes, which
    /// is the only thing left to compare when the audio never changes.
    public let shapeProfile: [UInt8]

    /// Median frame-to-frame change in log band energy. Near zero for a held
    /// tone or a drone; comfortably above it for music and speech.
    public let flux: Double

    /// True when the audio barely changes from frame to frame, which makes the
    /// difference-based bits in `values` pure rounding noise.
    public var isStationary: Bool { flux < FingerprintEngine.stationaryFlux }
    /// Seconds of audio each sub-fingerprint advances.
    public static let secondsPerFrame = Double(FingerprintEngine.hopSize) / AudioDecoder.fingerprintRate

    public var duration: Double { Double(values.count) * Fingerprint.secondsPerFrame }
    public var isUsable: Bool { values.count >= FingerprintEngine.minimumFrames }
}

/// A fingerprint plus what the engine learned about the audio while making it.
public struct AudioAnalysis {
    public let fingerprint: Fingerprint
    /// Seconds of near-silence trimmed from the head and tail before fingerprinting.
    public let leadingSilence: Double
    public let trailingSilence: Double
}

public final class FingerprintEngine {

    static let frameSize = 4096
    static let hopSize = 512
    static let bandCount = 33          // 33 bands -> 32 bits per frame
    static let lowFrequency = 300.0
    static let highFrequency = 3000.0
    /// Roughly 1.5 seconds of audio; below this there is nothing to compare.
    static let minimumFrames = 64

    /// Below this median spectral flux the audio is treated as stationary.
    /// Measured with a wide margin either side: a held tone reads about 0.011
    /// as PCM and 0.044 once lossily encoded, while music and speech sit at
    /// 0.23 and above.
    static let stationaryFlux = 0.10

    private let log2n: vDSP_Length
    private let setup: FFTSetup
    private let window: [Float]
    private let bandEdges: [Int]

    private var realPart: [Float]
    private var imagPart: [Float]
    private var magnitudes: [Float]

    public init() {
        let n = FingerprintEngine.frameSize
        log2n = vDSP_Length(log2(Double(n)).rounded())
        setup = vDSP_create_fftsetup(log2n, FFTRadix(kFFTRadix2))!

        var hann = [Float](repeating: 0, count: n)
        vDSP_hann_window(&hann, vDSP_Length(n), Int32(vDSP_HANN_NORM))
        window = hann

        bandEdges = FingerprintEngine.makeBandEdges(frameSize: n,
                                                    sampleRate: AudioDecoder.fingerprintRate)

        realPart = [Float](repeating: 0, count: n / 2)
        imagPart = [Float](repeating: 0, count: n / 2)
        magnitudes = [Float](repeating: 0, count: n / 2)
    }

    deinit { vDSP_destroy_fftsetup(setup) }

    /// Logarithmically spaced FFT bin boundaries covering 300 Hz - 3 kHz.
    private static func makeBandEdges(frameSize: Int, sampleRate: Double) -> [Int] {
        let binsPerHz = Double(frameSize) / sampleRate
        var edges: [Int] = []
        edges.reserveCapacity(bandCount + 1)
        let ratio = log(highFrequency / lowFrequency)
        for i in 0...bandCount {
            let frequency = lowFrequency * exp(ratio * Double(i) / Double(bandCount))
            var bin = Int((frequency * binsPerHz).rounded())
            bin = min(max(bin, 1), frameSize / 2 - 1)
            // Guarantee every band owns at least one bin.
            if let last = edges.last, bin <= last { bin = last + 1 }
            edges.append(min(bin, frameSize / 2 - 1))
        }
        return edges
    }

    /// Computes the fingerprint of already-decoded mono audio, and reports the
    /// silence trimmed off each end so callers can tell how much real audio
    /// a file actually holds.
    public func analyze(samples: [Float], sampleRate: Double = AudioDecoder.fingerprintRate) -> AudioAnalysis {
        let range = FingerprintEngine.contentRange(samples)
        let leading = Double(range.lowerBound) / sampleRate
        let trailing = Double(samples.count - range.upperBound) / sampleRate
        return AudioAnalysis(fingerprint: fingerprint(samples: samples),
                             leadingSilence: leading,
                             trailingSilence: trailing)
    }

    /// Computes the fingerprint of already-decoded mono audio.
    public func fingerprint(samples: [Float]) -> Fingerprint {
        let trimmed = FingerprintEngine.trimSilence(samples)
        let n = FingerprintEngine.frameSize
        let empty = Fingerprint(values: [], shapeProfile: [], flux: .greatestFiniteMagnitude)
        guard trimmed.count >= n + FingerprintEngine.hopSize else { return empty }

        let frameCount = (trimmed.count - n) / FingerprintEngine.hopSize + 1
        var bandEnergies = [Float](repeating: 0, count: FingerprintEngine.bandCount)
        var previousEnergies: [Float]? = nil
        var values: [UInt32] = []
        var frameFlux: [Float] = []
        var bandTotals = [Double](repeating: 0, count: FingerprintEngine.bandCount)
        var shapeFrames = 0
        values.reserveCapacity(frameCount)

        var windowed = [Float](repeating: 0, count: n)

        for frame in 0..<frameCount {
            let start = frame * FingerprintEngine.hopSize
            trimmed.withUnsafeBufferPointer { source in
                vDSP_vmul(source.baseAddress! + start, 1, window, 1, &windowed, 1, vDSP_Length(n))
            }
            spectrum(of: windowed, into: &bandEnergies)

            // Spectral shape: which of two neighbouring bands is louder, within
            // this one frame. Scaling every band by the same factor cannot
            // change the answer, so this is volume-invariant like `values` —
            // but it survives audio that never changes.
            for m in 0..<FingerprintEngine.bandCount {
                bandTotals[m] += Double(bandEnergies[m])
            }
            shapeFrames += 1

            if let previous = previousEnergies {
                var bits: UInt32 = 0
                var change: Float = 0
                for m in 0..<32 {
                    let current = bandEnergies[m] - bandEnergies[m + 1]
                    let earlier = previous[m] - previous[m + 1]
                    if current - earlier > 0 { bits |= (1 << UInt32(m)) }
                }
                for m in 0..<FingerprintEngine.bandCount {
                    change += abs(bandEnergies[m] - previous[m])
                }
                values.append(bits)
                frameFlux.append(change / Float(FingerprintEngine.bandCount))
            }
            previousEnergies = bandEnergies
        }

        return Fingerprint(values: values,
                           shapeProfile: FingerprintEngine.shape(totals: bandTotals, frames: shapeFrames),
                           flux: Double(FingerprintEngine.median(frameFlux)))
    }

    /// Builds the spectral-shape template.
    ///
    /// Each band's mean log energy is expressed relative to the loudest band,
    /// which removes overall volume, then clamped to an 80 dB window and
    /// quantised. Clamping matters as much as the rest: bands holding nothing
    /// but noise floor land on the same floor value in every file, so the
    /// differences between one codec's idea of silence and another's stop
    /// counting as differences in the audio.
    static func shape(totals: [Double], frames: Int) -> [UInt8] {
        guard frames > 0, !totals.isEmpty else { return [] }

        let means = totals.map { $0 / Double(frames) }
        guard let peak = means.max() else { return [] }

        // Band energies are log10 of power, so 8.0 spans 80 dB.
        let window = 8.0
        return means.map { mean in
            let relative = min(0, max(-window, mean - peak))
            return UInt8(((relative + window) / window * 255).rounded())
        }
    }

    /// Distance between two shape templates, 0 (identical) to 1.
    /// Only meaningful when both files are stationary.
    ///
    /// Each band is weighted by how much energy either file puts there, so the
    /// comparison is driven by the bands that actually carry the sound. Without
    /// that weighting, two quite different tones look similar merely because
    /// most of the spectrum is empty in both of them.
    public static func shapeDistance(_ a: [UInt8], _ b: [UInt8]) -> Double {
        guard a.count == b.count, !a.isEmpty else { return 1 }

        var weightedDifference = 0.0
        var totalWeight = 0.0
        for i in 0..<a.count {
            let valueA = Double(a[i]), valueB = Double(b[i])
            let weight = max(valueA, valueB) / 255.0
            weightedDifference += weight * abs(valueA - valueB)
            totalWeight += weight
        }
        guard totalWeight > 0 else { return 0 }
        return weightedDifference / totalWeight / 255.0
    }

    static func median(_ values: [Float]) -> Float {
        guard !values.isEmpty else { return .greatestFiniteMagnitude }
        let sorted = values.sorted()
        return sorted[sorted.count / 2]
    }

    /// Real FFT of one windowed frame, accumulated into log band energies.
    private func spectrum(of windowed: [Float], into bands: inout [Float]) {
        let n = FingerprintEngine.frameSize
        let half = n / 2

        realPart.withUnsafeMutableBufferPointer { realBuffer in
            imagPart.withUnsafeMutableBufferPointer { imagBuffer in
                var split = DSPSplitComplex(realp: realBuffer.baseAddress!,
                                            imagp: imagBuffer.baseAddress!)
                windowed.withUnsafeBufferPointer { source in
                    source.baseAddress!.withMemoryRebound(to: DSPComplex.self, capacity: half) { complex in
                        vDSP_ctoz(complex, 2, &split, 1, vDSP_Length(half))
                    }
                }
                vDSP_fft_zrip(setup, &split, 1, log2n, FFTDirection(FFT_FORWARD))
                // Power spectrum; the 1/4 factor undoes vDSP's zrip scaling.
                vDSP_zvmags(&split, 1, &magnitudes, 1, vDSP_Length(half))
            }
        }

        var scale: Float = 0.25
        vDSP_vsmul(magnitudes, 1, &scale, &magnitudes, 1, vDSP_Length(half))

        for band in 0..<FingerprintEngine.bandCount {
            let lower = bandEdges[band]
            let upper = max(bandEdges[band + 1], lower + 1)
            var sum: Float = 0
            magnitudes.withUnsafeBufferPointer { buffer in
                vDSP_sve(buffer.baseAddress! + lower, 1, &sum, vDSP_Length(upper - lower))
            }
            // Log energy keeps quiet passages as informative as loud ones.
            bands[band] = log10f(sum + 1e-9)
        }
    }

    /// Drops near-silent head and tail so padding differences do not shift alignment.
    static func trimSilence(_ samples: [Float]) -> ArraySlice<Float> {
        let range = contentRange(samples)
        return samples[range]
    }

    /// The span of `samples` that actually carries audio.
    static func contentRange(_ samples: [Float]) -> Range<Int> {
        guard !samples.isEmpty else { return 0..<0 }

        var peak: Float = 0
        vDSP_maxmgv(samples, 1, &peak, vDSP_Length(samples.count))
        guard peak > 1e-5 else { return 0..<samples.count }
        let threshold = peak * 0.005

        var start = 0
        while start < samples.count && abs(samples[start]) < threshold { start += 1 }
        var end = samples.count - 1
        while end > start && abs(samples[end]) < threshold { end -= 1 }
        guard end > start else { return 0..<samples.count }
        return start..<(end + 1)
    }
}
