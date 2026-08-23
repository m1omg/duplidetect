//! Audio decoding, replacing the AVFoundation path in AudioDecoder.swift.
//!
//! Everything goes through Symphonia, so there is nothing to install on any
//! platform. Files are decoded to mono at the fingerprinter's rate, which is
//! all the fingerprinter needs.

use dd_core::fingerprint::FINGERPRINT_RATE;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{CodecType, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One AAC-LC frame. The standard decoder delay for AAC-LC is a single frame;
/// encoders that use 2112 samples signal it via iTunSMPB, which is the case
/// Symphonia would report through `delay`.
const AAC_PRIMING_SAMPLES: usize = 1024;

pub struct DecodedAudio {
    /// Mono samples at `sample_rate`.
    pub samples: Vec<f32>,
    pub sample_rate: f64,
    /// Full duration of the source file, even if decoding was capped.
    pub source_duration: f64,
    pub source_sample_rate: f64,
    pub source_channels: u32,
    /// Bits per sample for uncompressed formats; None where it means nothing.
    pub source_bit_depth: Option<u32>,
    /// Whether the codec stores audio without loss. Determined from the codec
    /// itself, never the file extension — a .caf or .m4a can hold either kind.
    pub is_lossless: bool,
    pub format_name: String,
    /// True when `max_seconds` cut the decode short, so the tail was never seen.
    pub was_truncated: bool,
}

#[derive(Debug)]
pub enum DecodeError {
    Unsupported(String),
    EmptyAudio,
    Failed(String),
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Unsupported(d) => write!(f, "could not be decoded ({d})"),
            DecodeError::EmptyAudio => write!(f, "contains no audio"),
            DecodeError::Failed(d) => write!(f, "decode failed ({d})"),
        }
    }
}

/// Only these codecs reconstruct the original samples exactly.
fn is_lossless_codec(codec: CodecType) -> bool {
    use symphonia::core::codecs::*;
    matches!(
        codec,
        CODEC_TYPE_PCM_S16LE | CODEC_TYPE_PCM_S16BE | CODEC_TYPE_PCM_S24LE | CODEC_TYPE_PCM_S24BE
            | CODEC_TYPE_PCM_S32LE | CODEC_TYPE_PCM_S32BE | CODEC_TYPE_PCM_U8
            | CODEC_TYPE_PCM_S8 | CODEC_TYPE_PCM_F32LE | CODEC_TYPE_PCM_F32BE
            | CODEC_TYPE_PCM_F64LE | CODEC_TYPE_PCM_F64BE
            | CODEC_TYPE_PCM_ALAW | CODEC_TYPE_PCM_MULAW
            | CODEC_TYPE_FLAC | CODEC_TYPE_ALAC
    ) && !matches!(codec, CODEC_TYPE_PCM_ALAW | CODEC_TYPE_PCM_MULAW)
}

fn codec_name(codec: CodecType, extension: &str) -> String {
    use symphonia::core::codecs::*;
    match codec {
        CODEC_TYPE_MP3 => "MP3".into(),
        CODEC_TYPE_AAC => "AAC".into(),
        CODEC_TYPE_ALAC => "Apple Lossless".into(),
        CODEC_TYPE_FLAC => "FLAC".into(),
        CODEC_TYPE_VORBIS => "Ogg Vorbis".into(),
        CODEC_TYPE_PCM_ALAW => "A-law".into(),
        CODEC_TYPE_PCM_MULAW => "µ-law".into(),
        c if is_lossless_codec(c) => format!("{} (PCM)", extension.to_uppercase()),
        _ => extension.to_uppercase(),
    }
}

/// Bit depth is only meaningful for uncompressed audio.
///
/// CoreAudio reports 0 for FLAC and ALAC, which the macOS app turns into
/// "unknown" and then skips in its quality comparison. Symphonia does report a
/// value for those codecs, so it is deliberately discarded here — otherwise a
/// FLAC would start losing to a 24-bit WAV holding the very same audio, and the
/// two platforms would delete different files.
fn meaningful_bit_depth(codec: CodecType, bits: Option<u32>) -> Option<u32> {
    use symphonia::core::codecs::*;
    if matches!(codec, CODEC_TYPE_FLAC | CODEC_TYPE_ALAC) {
        return None;
    }
    bits.filter(|b| *b > 0)
}

pub fn decode_mono(
    path: &Path,
    target_rate: f64,
    max_seconds: Option<f64>,
) -> Result<DecodedAudio, DecodeError> {
    let file = File::open(path).map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !extension.is_empty() {
        hint.with_extension(&extension);
    }

    let probed = symphonia::default::get_probe()
        .format(&hint, stream,
                &FormatOptions {
                    enable_gapless: std::env::var("DD_GAPLESS").as_deref() != Ok("0"),
                    ..Default::default()
                },
                &MetadataOptions::default())
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;
    let mut format = probed.format;

    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| DecodeError::Unsupported("no audio track".into()))?;
    let track_id = track.id;
    let params = track.codec_params.clone();

    let source_rate = params.sample_rate.ok_or(DecodeError::EmptyAudio)? as f64;
    if source_rate <= 0.0 {
        return Err(DecodeError::EmptyAudio);
    }
    // AAC and ALAC in an MP4 container do not declare their channel count in
    // the container metadata; it is only known once a packet has been decoded.
    // So this is read from the first decoded buffer rather than required here.
    let mut channels = params.channels.map(|c| c.count() as u32).unwrap_or(0);
    let source_duration = params
        .n_frames
        .map(|n| n as f64 / source_rate)
        .unwrap_or(0.0);

    let mut decoder = symphonia::default::get_codecs()
        .make(&params, &DecoderOptions::default())
        .map_err(|e| DecodeError::Unsupported(e.to_string()))?;

    // Decode to mono at the source rate first, mirroring the order the Swift
    // Vorbis path uses (downmix, then resample) for every format.
    let frame_limit = max_seconds.map(|s| (s * source_rate) as usize).unwrap_or(usize::MAX);
    let mut mono: Vec<f32> = Vec::new();
    let mut truncated = false;
    let mut last_error: Option<String> = None;

    loop {
        if mono.len() >= frame_limit {
            truncated = true;
            break;
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(symphonia::core::errors::Error::ResetRequired) => break,
            Err(e) => { last_error = Some(format!("packet: {e}")); break }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buffer) => {
                if channels == 0 {
                    channels = buffer.spec().channels.count() as u32;
                }
                append_mono(&buffer, &mut mono)
            }
            Err(symphonia::core::errors::Error::DecodeError(e)) => {
                last_error = Some(format!("decode: {e}"));
                continue;
            }
            Err(e) => { last_error = Some(format!("decode-fatal: {e}")); break }
        }
    }

    if mono.is_empty() {
        return Err(match last_error {
            Some(e) => DecodeError::Failed(e),
            None => DecodeError::EmptyAudio,
        });
    }
    if mono.len() > frame_limit {
        mono.truncate(frame_limit);
        truncated = true;
    }

    // AAC in an MP4 container carries one frame of encoder priming that
    // Symphonia does not report or trim: it leaves `delay` unset, so gapless
    // handling never fires. CoreAudio does trim it, and the residue is not
    // silent — it rings, so the fingerprinter's silence trimming cannot absorb
    // it either. What is left is a fraction-of-a-frame misalignment, which
    // integer-frame offset search cannot correct, and which measurably degrades
    // matching (bit-error rate 0.108 against 0.060 on macOS for the same pair).
    // MP3 needs none of this because Symphonia does report its LAME delay.
    if params.codec == symphonia::core::codecs::CODEC_TYPE_AAC
        && params.delay.is_none()
        && mono.len() > AAC_PRIMING_SAMPLES
    {
        mono.drain(..AAC_PRIMING_SAMPLES);
    }

    let decoded_seconds = mono.len() as f64 / source_rate;
    let source_duration = if source_duration > 0.0 { source_duration } else { decoded_seconds };
    let resampled = resample(&mono, source_rate, target_rate)?;

    Ok(DecodedAudio {
        samples: resampled,
        sample_rate: target_rate,
        source_duration,
        source_sample_rate: source_rate,
        source_channels: channels,
        source_bit_depth: meaningful_bit_depth(params.codec, params.bits_per_sample),
        is_lossless: is_lossless_codec(params.codec),
        format_name: codec_name(params.codec, &extension),
        was_truncated: truncated && decoded_seconds < source_duration - 0.05,
    })
}

/// Averages all channels into one, matching the Swift downmix.
fn append_mono(buffer: &AudioBufferRef, out: &mut Vec<f32>) {
    macro_rules! mix {
        ($buf:expr, $conv:expr) => {{
            let b = $buf;
            let channels = b.spec().channels.count();
            let frames = b.frames();
            let scale = 1.0f32 / channels as f32;
            out.reserve(frames);
            for frame in 0..frames {
                let mut sum = 0.0f32;
                for ch in 0..channels {
                    sum += $conv(b.chan(ch)[frame]);
                }
                out.push(sum * scale);
            }
        }};
    }
    match buffer {
        AudioBufferRef::F32(b) => mix!(b, |v: f32| v),
        AudioBufferRef::F64(b) => mix!(b, |v: f64| v as f32),
        AudioBufferRef::S32(b) => mix!(b, |v: i32| v as f32 / 2147483648.0),
        AudioBufferRef::S24(b) => {
            mix!(b, |v: symphonia::core::sample::i24| v.inner() as f32 / 8388608.0)
        }
        AudioBufferRef::S16(b) => mix!(b, |v: i16| v as f32 / 32768.0),
        AudioBufferRef::S8(b) => mix!(b, |v: i8| v as f32 / 128.0),
        AudioBufferRef::U32(b) => mix!(b, |v: u32| (v as f32 - 2147483648.0) / 2147483648.0),
        AudioBufferRef::U24(b) => {
            mix!(b, |v: symphonia::core::sample::u24| (v.inner() as f32 - 8388608.0) / 8388608.0)
        }
        AudioBufferRef::U16(b) => mix!(b, |v: u16| (v as f32 - 32768.0) / 32768.0),
        AudioBufferRef::U8(b) => mix!(b, |v: u8| (v as f32 - 128.0) / 128.0),
    }
}

fn resample(samples: &[f32], source_rate: f64, target_rate: f64) -> Result<Vec<f32>, DecodeError> {
    if (source_rate - target_rate).abs() < 0.5 {
        return Ok(samples.to_vec());
    }
    let chunk = 4096usize;
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler =
        SincFixedIn::<f32>::new(target_rate / source_rate, 2.0, params, chunk, 1)
            .map_err(|e| DecodeError::Failed(e.to_string()))?;

    let mut out: Vec<f32> = Vec::with_capacity((samples.len() as f64 * target_rate / source_rate) as usize + chunk);
    let mut position = 0usize;
    while position + chunk <= samples.len() {
        let block = vec![samples[position..position + chunk].to_vec()];
        let done = resampler.process(&block, None).map_err(|e| DecodeError::Failed(e.to_string()))?;
        out.extend_from_slice(&done[0]);
        position += chunk;
    }
    if position < samples.len() {
        let mut tail = samples[position..].to_vec();
        tail.resize(chunk, 0.0);
        let done = resampler.process(&[tail], None).map_err(|e| DecodeError::Failed(e.to_string()))?;
        let wanted = ((samples.len() - position) as f64 * target_rate / source_rate).round() as usize;
        out.extend_from_slice(&done[0][..wanted.min(done[0].len())]);
    }
    Ok(out)
}

/// Convenience wrapper at the fingerprinter's rate.
pub fn decode_for_fingerprint(path: &Path, max_seconds: f64) -> Result<DecodedAudio, DecodeError> {
    decode_mono(path, FINGERPRINT_RATE, Some(max_seconds))
}
