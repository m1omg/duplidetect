import Foundation
import AVFoundation


public struct DecodedAudio {
    /// Mono samples at `sampleRate`.
    public let samples: [Float]
    public let sampleRate: Double
    /// Full duration of the source file, even if decoding was capped.
    public let sourceDuration: Double
    public let sourceSampleRate: Double
    public let sourceChannels: Int
    /// Bits per sample for lossless formats; nil where it means nothing.
    public let sourceBitDepth: Int?
    /// Whether the *codec* stores audio without loss. Determined from the codec
    /// itself, never the file extension — a .caf or .m4a can hold either kind.
    public let isLossless: Bool
    public let formatName: String
    /// True when `maxSeconds` cut the decode short, so the tail was never seen.
    public let wasTruncated: Bool
}

public enum AudioDecodeError: Error {
    case unsupported(String)
    case emptyAudio
    case decodeFailed(String)
}

/// Decodes any supported audio file down to mono PCM at a fixed rate,
/// which is all the fingerprinter needs.
public enum AudioDecoder {

    /// Rate the fingerprinter works at. Low enough to be fast, high enough
    /// to keep everything musically relevant (up to ~5.5 kHz).
    public static let fingerprintRate: Double = 11025

    public static func decodeMono(url: URL,
                                  targetRate: Double = fingerprintRate,
                                  maxSeconds: Double? = nil) throws -> DecodedAudio {
        let ext = url.pathExtension.lowercased()
        if AudioFormats.vorbis.contains(ext) {
            // AVFoundation has no Ogg Vorbis support on macOS; use the bundled decoder.
            if let decoded = try? decodeVorbis(url: url, targetRate: targetRate, maxSeconds: maxSeconds) {
                return decoded
            }
        }
        return try decodeNative(url: url, targetRate: targetRate, maxSeconds: maxSeconds)
    }

    // MARK: - CoreAudio / AVFoundation path

    private static func decodeNative(url: URL,
                                     targetRate: Double,
                                     maxSeconds: Double?) throws -> DecodedAudio {
        let file: AVAudioFile
        do {
            file = try AVAudioFile(forReading: url)
        } catch {
            throw AudioDecodeError.unsupported(error.localizedDescription)
        }

        let sourceFormat = file.processingFormat
        let sourceDuration = Double(file.length) / sourceFormat.sampleRate
        guard file.length > 0 else { throw AudioDecodeError.emptyAudio }

        guard let target = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                         sampleRate: targetRate,
                                         channels: 1,
                                         interleaved: false),
              let converter = AVAudioConverter(from: sourceFormat, to: target) else {
            throw AudioDecodeError.decodeFailed("no converter for \(sourceFormat)")
        }

        let frameLimit = maxSeconds.map { AVAudioFramePosition($0 * sourceFormat.sampleRate) }
        let framesToRead = frameLimit.map { min($0, file.length) } ?? file.length

        let chunkFrames: AVAudioFrameCount = 1 << 16
        var remaining = framesToRead
        var out: [Float] = []
        out.reserveCapacity(Int(Double(framesToRead) / sourceFormat.sampleRate * targetRate) + 1024)

        var readError: Error?
        var reachedEnd = false

        let inputBlock: AVAudioConverterInputBlock = { _, status in
            if reachedEnd || remaining <= 0 {
                status.pointee = .endOfStream
                return nil
            }
            let want = AVAudioFrameCount(min(AVAudioFramePosition(chunkFrames), remaining))
            guard let buffer = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: want) else {
                status.pointee = .endOfStream
                return nil
            }
            do {
                try file.read(into: buffer, frameCount: want)
            } catch {
                readError = error
                status.pointee = .endOfStream
                return nil
            }
            if buffer.frameLength == 0 {
                reachedEnd = true
                status.pointee = .endOfStream
                return nil
            }
            remaining -= AVAudioFramePosition(buffer.frameLength)
            status.pointee = .haveData
            return buffer
        }

        let outCapacity: AVAudioFrameCount = 1 << 15
        guard let outBuffer = AVAudioPCMBuffer(pcmFormat: target, frameCapacity: outCapacity) else {
            throw AudioDecodeError.decodeFailed("output buffer allocation failed")
        }

        loop: while true {
            outBuffer.frameLength = 0
            var conversionError: NSError?
            let status = converter.convert(to: outBuffer, error: &conversionError, withInputFrom: inputBlock)
            if let readError { throw AudioDecodeError.decodeFailed(readError.localizedDescription) }
            if let conversionError { throw AudioDecodeError.decodeFailed(conversionError.localizedDescription) }

            if outBuffer.frameLength > 0, let channel = outBuffer.floatChannelData?[0] {
                out.append(contentsOf: UnsafeBufferPointer(start: channel, count: Int(outBuffer.frameLength)))
            }
            switch status {
            case .haveData: continue
            case .inputRanDry where outBuffer.frameLength > 0: continue
            case .endOfStream, .inputRanDry, .error: break loop
            @unknown default: break loop
            }
        }

        guard !out.isEmpty else { throw AudioDecodeError.emptyAudio }

        let description = file.fileFormat.streamDescription.pointee
        let depth = Int(description.mBitsPerChannel)

        return DecodedAudio(samples: out,
                            sampleRate: targetRate,
                            sourceDuration: sourceDuration,
                            sourceSampleRate: file.fileFormat.sampleRate,
                            sourceChannels: Int(file.fileFormat.channelCount),
                            sourceBitDepth: depth > 0 ? depth : nil,
                            isLossless: isLosslessCodec(description.mFormatID),
                            formatName: formatName(for: url, format: file.fileFormat),
                            wasTruncated: framesToRead < file.length)
    }

    // MARK: - Ogg Vorbis path

    private static func decodeVorbis(url: URL,
                                     targetRate: Double,
                                     maxSeconds: Double?) throws -> DecodedAudio {
        var channels: Int32 = 0
        var rate: Int32 = 0
        var duration: Double = 0

        guard let handle = url.withUnsafeFileSystemRepresentation({ path -> OpaquePointer? in
            guard let path else { return nil }
            return dd_vorbis_open(path, &channels, &rate, &duration)
        }) else {
            throw AudioDecodeError.unsupported("not a readable Ogg Vorbis stream")
        }
        defer { dd_vorbis_close(handle) }

        guard channels > 0, rate > 0 else { throw AudioDecodeError.emptyAudio }

        let sourceRate = Double(rate)
        let channelCount = Int(channels)
        let frameLimit = maxSeconds.map { Int($0 * sourceRate) } ?? Int.max

        // Decode to mono at the source rate first.
        let chunkFrames = 8192
        var interleaved = [Float](repeating: 0, count: chunkFrames * channelCount)
        var mono: [Float] = []
        mono.reserveCapacity(min(frameLimit, Int(duration * sourceRate) + 1024))

        while mono.count < frameLimit {
            let want = min(chunkFrames, frameLimit - mono.count)
            let got = interleaved.withUnsafeMutableBufferPointer { buffer -> Int in
                Int(dd_vorbis_read(handle, buffer.baseAddress, Int32(want)))
            }
            if got <= 0 { break }
            if channelCount == 1 {
                mono.append(contentsOf: interleaved[0..<got])
            } else {
                let scale = 1.0 / Float(channelCount)
                for frame in 0..<got {
                    var sum: Float = 0
                    for channel in 0..<channelCount {
                        sum += interleaved[frame * channelCount + channel]
                    }
                    mono.append(sum * scale)
                }
            }
        }

        guard !mono.isEmpty else { throw AudioDecodeError.emptyAudio }

        let resampled = resample(mono, from: sourceRate, to: targetRate)

        let fullDuration = duration > 0 ? duration : Double(mono.count) / sourceRate

        return DecodedAudio(samples: resampled,
                            sampleRate: targetRate,
                            sourceDuration: fullDuration,
                            sourceSampleRate: sourceRate,
                            sourceChannels: channelCount,
                            sourceBitDepth: nil,
                            isLossless: false,
                            formatName: "Ogg Vorbis",
                            wasTruncated: Double(mono.count) / sourceRate < fullDuration - 0.05)
    }

    /// Resamples mono audio through AVAudioConverter so the Vorbis path matches
    /// the CoreAudio path bit-for-bit closely enough for fingerprinting.
    private static func resample(_ samples: [Float], from sourceRate: Double, to targetRate: Double) -> [Float] {
        if abs(sourceRate - targetRate) < 0.5 { return samples }

        guard let sourceFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                               sampleRate: sourceRate, channels: 1, interleaved: false),
              let targetFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                               sampleRate: targetRate, channels: 1, interleaved: false),
              let converter = AVAudioConverter(from: sourceFormat, to: targetFormat),
              let input = AVAudioPCMBuffer(pcmFormat: sourceFormat, frameCapacity: AVAudioFrameCount(samples.count))
        else {
            return decimate(samples, from: sourceRate, to: targetRate)
        }

        input.frameLength = AVAudioFrameCount(samples.count)
        if let channel = input.floatChannelData?[0] {
            samples.withUnsafeBufferPointer { channel.assign(from: $0.baseAddress!, count: samples.count) }
        }

        let outFrames = AVAudioFrameCount(Double(samples.count) * targetRate / sourceRate) + 4096
        guard let output = AVAudioPCMBuffer(pcmFormat: targetFormat, frameCapacity: outFrames) else {
            return decimate(samples, from: sourceRate, to: targetRate)
        }

        var consumed = false
        var error: NSError?
        converter.convert(to: output, error: &error) { _, status in
            if consumed {
                status.pointee = .endOfStream
                return nil
            }
            consumed = true
            status.pointee = .haveData
            return input
        }
        if error != nil { return decimate(samples, from: sourceRate, to: targetRate) }

        guard let channel = output.floatChannelData?[0], output.frameLength > 0 else {
            return decimate(samples, from: sourceRate, to: targetRate)
        }
        return Array(UnsafeBufferPointer(start: channel, count: Int(output.frameLength)))
    }

    /// Plain linear-interpolation fallback.
    private static func decimate(_ samples: [Float], from sourceRate: Double, to targetRate: Double) -> [Float] {
        let ratio = sourceRate / targetRate
        let count = Int(Double(samples.count) / ratio)
        guard count > 1 else { return samples }
        var out = [Float](repeating: 0, count: count)
        for i in 0..<count {
            let position = Double(i) * ratio
            let index = Int(position)
            let fraction = Float(position - Double(index))
            let a = samples[min(index, samples.count - 1)]
            let b = samples[min(index + 1, samples.count - 1)]
            out[i] = a + (b - a) * fraction
        }
        return out
    }

    /// Only these codecs reconstruct the original samples exactly.
    private static func isLosslessCodec(_ id: AudioFormatID) -> Bool {
        switch id {
        case kAudioFormatLinearPCM, kAudioFormatAppleLossless, kAudioFormatFLAC:
            return true
        default:
            return false
        }
    }

    private static func formatName(for url: URL, format: AVAudioFormat) -> String {
        var formatID = format.streamDescription.pointee.mFormatID
        let readable = withUnsafeBytes(of: &formatID) { bytes -> String in
            let characters = bytes.reversed().map { Character(UnicodeScalar($0)) }
            return String(characters).trimmingCharacters(in: .whitespaces)
        }
        switch readable {
        case "lpcm": return url.pathExtension.uppercased() + " (PCM)"
        case ".mp3": return "MP3"
        // CoreAudio spells AAC differently per profile and container.
        case "aac", "aace", "aacf", "aacg", "aach", "aacl", "aacp", "paac", "qaac": return "AAC"
        case "alac", "qlac": return "Apple Lossless"
        case "flac", "qflc": return "FLAC"
        case "opus": return "Opus"
        case "ac-3", "qac3": return "AC-3"
        case "ima4": return "IMA ADPCM"
        case "ulaw": return "µ-law"
        case "alaw": return "A-law"
        default: return readable.isEmpty ? url.pathExtension.uppercased() : readable.uppercased()
        }
    }
}

public extension AudioDecoder {
    /// Decodes a short excerpt at playback quality, for auditioning a file.
    /// Uses the same decoders as fingerprinting, so Ogg Vorbis previews too.
    static func decodeForPlayback(url: URL, seconds: Double = 30) throws -> AVAudioPCMBuffer {
        let audio = try decodeMono(url: url, targetRate: 44100, maxSeconds: seconds)
        guard let format = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                         sampleRate: 44100, channels: 1, interleaved: false),
              let buffer = AVAudioPCMBuffer(pcmFormat: format,
                                            frameCapacity: AVAudioFrameCount(audio.samples.count)),
              let channel = buffer.floatChannelData?[0] else {
            throw AudioDecodeError.decodeFailed("could not build playback buffer")
        }
        buffer.frameLength = AVAudioFrameCount(audio.samples.count)
        audio.samples.withUnsafeBufferPointer { channel.assign(from: $0.baseAddress!, count: audio.samples.count) }
        return buffer
    }
}
