//! Which file extensions DupliDetect considers audio, and what they imply.
//!
//! The sets differ from the macOS build because the decoder differs. Symphonia
//! has no AC-3, AMR or Sound Designer II support, so those move from "decoded
//! and audio-matched" to "exact copies only" — a documented regression against
//! the Mac app. Opus, WMA, WavPack, Monkey's Audio and Matroska were already in
//! that column on macOS and stay there.

use std::path::Path;

/// Decoded by Symphonia, so eligible for acoustic matching.
pub const DECODABLE: &[&str] = &[
    "wav", "wave", "aif", "aiff", "aifc", "mp3", "m4a", "m4b", "m4r", "aac", "adts", "caf", "flac",
    "alac", "ogg", "oga",
];

/// Scanned and hashed, but the audio cannot be compared on this platform.
pub const BYTE_COMPARABLE_ONLY: &[&str] =
    &["opus", "wma", "wv", "ape", "mka", "ra", "au", "snd", "amr", "ac3", "sd2"];

pub fn extension_of(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase()
}

pub fn is_audio(path: &Path) -> bool {
    let ext = extension_of(path);
    DECODABLE.contains(&ext.as_str()) || BYTE_COMPARABLE_ONLY.contains(&ext.as_str())
}

pub fn is_byte_comparable_only(path: &Path) -> bool {
    BYTE_COMPARABLE_ONLY.contains(&extension_of(path).as_str())
}

/// Best guess at losslessness from the extension alone, for files that cannot
/// be decoded. `None` means the extension is a container that says nothing
/// about the codec inside it, so no guess is possible.
pub fn lossless_by_extension(path: &Path) -> Option<bool> {
    match extension_of(path).as_str() {
        "flac" | "alac" | "wv" | "ape" | "wav" | "wave" | "aif" | "aiff" | "aifc" => Some(true),
        "mp3" | "aac" | "adts" | "opus" | "wma" | "ra" | "amr" | "ac3" | "ogg" | "oga" => Some(false),
        // caf, m4a, m4b, m4r, au, snd, mka, sd2: container only, contents unknown.
        _ => None,
    }
}

/// A human-readable name for the codec a file extension implies.
pub fn label(path: &Path) -> String {
    match extension_of(path).as_str() {
        "opus" => "Opus".into(),
        "wma" => "Windows Media Audio".into(),
        "wv" => "WavPack".into(),
        "ape" => "Monkey's Audio".into(),
        "mka" => "Matroska Audio".into(),
        "ra" => "RealAudio".into(),
        "ac3" => "AC-3".into(),
        "amr" => "AMR".into(),
        "sd2" => "Sound Designer II".into(),
        other => other.to_uppercase(),
    }
}
