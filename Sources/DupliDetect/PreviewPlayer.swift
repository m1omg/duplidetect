import Foundation
import AVFoundation
import Combine

/// Plays a short excerpt of a file so you can hear which copy to keep.
/// Everything goes through DupliDetect's own decoders, so Ogg Vorbis —
/// which AVAudioPlayer cannot open — previews just like the rest.
@MainActor
public final class PreviewPlayer: ObservableObject {

    @Published public private(set) var playingFile: UUID?
    @Published public private(set) var lastError: String?

    /// The engine graph is wired for exactly this format. It has to match the
    /// buffers that get scheduled — connecting with a `nil` format instead
    /// binds the hardware's stereo format and mono buffers are then dropped.
    private static let playbackFormat = AVAudioFormat(commonFormat: .pcmFormatFloat32,
                                                      sampleRate: 44100,
                                                      channels: 1,
                                                      interleaved: false)!

    private let engine = AVAudioEngine()
    private let node = AVAudioPlayerNode()
    private var isWired = false

    public init() {}

    public func toggle(file: AudioFile) {
        if playingFile == file.id {
            stop()
        } else {
            play(file: file)
        }
    }

    public func play(file: AudioFile) {
        stop()

        let buffer: AVAudioPCMBuffer
        do {
            buffer = try AudioDecoder.decodeForPlayback(url: file.url)
        } catch {
            lastError = "Could not play \(file.displayName): this format cannot be decoded on this Mac."
            return
        }

        do {
            try startEngine()
        } catch {
            lastError = "Could not start audio playback: \(error.localizedDescription)"
            return
        }

        playingFile = file.id
        let id = file.id
        node.scheduleBuffer(buffer, at: nil, options: []) { [weak self] in
            Task { @MainActor in
                guard let self, self.playingFile == id else { return }
                self.playingFile = nil
            }
        }
        node.play()
    }

    private func startEngine() throws {
        if !isWired {
            engine.attach(node)
            engine.connect(node, to: engine.mainMixerNode, format: Self.playbackFormat)
            isWired = true
        }
        if !engine.isRunning {
            engine.prepare()
            try engine.start()
        }
    }

    public func stop() {
        if node.isPlaying { node.stop() }
        playingFile = nil
    }

    public func clearError() { lastError = nil }
}
