//! File records and the quality ordering, ported from Models.swift.

use crate::fingerprint::Fingerprint;
use crate::formats;
use std::cmp::Ordering;
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug)]
pub struct AudioFile {
    pub path: PathBuf,
    pub byte_size: u64,
    pub modified: SystemTime,

    /// Filled in during probing; None when the file could not be decoded.
    pub duration: Option<f64>,
    /// Seconds of actual audio, ignoring silent padding at either end.
    pub content_duration: Option<f64>,
    pub sample_rate: Option<f64>,
    pub bit_depth: Option<u32>,
    pub channel_count: Option<u32>,
    pub format_name: Option<String>,
    /// Set from the decoded codec. None until the file has been decoded.
    pub codec_is_lossless: Option<bool>,

    pub content_hash: Option<String>,
    pub fingerprint: Option<Fingerprint>,
}

impl AudioFile {
    pub fn new(path: PathBuf, byte_size: u64, modified: SystemTime) -> Self {
        let format_name = Some(formats::label(&path));
        AudioFile {
            path,
            byte_size,
            modified,
            duration: None,
            content_duration: None,
            sample_rate: None,
            bit_depth: None,
            channel_count: None,
            format_name,
            codec_is_lossless: None,
            content_hash: None,
            fingerprint: None,
        }
    }

    pub fn display_name(&self) -> String {
        self.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string()
    }

    pub fn folder_path(&self) -> String {
        self.path.parent().map(|p| p.display().to_string()).unwrap_or_default()
    }

    /// Approximate bitrate in kbps, derived from size and duration.
    pub fn bitrate_kbps(&self) -> Option<u32> {
        let duration = self.duration?;
        if duration <= 0.05 {
            return None;
        }
        Some((self.byte_size as f64 * 8.0 / duration / 1000.0).round() as u32)
    }

    /// Whether this file's audio is stored without loss. Taken from the codec
    /// that was actually decoded, falling back to the extension only when the
    /// file could not be opened — `.caf` and `.m4a` are containers that hold
    /// either kind, so the extension alone is not trustworthy.
    pub fn is_lossless(&self) -> Option<bool> {
        self.codec_is_lossless.or_else(|| formats::lossless_by_extension(&self.path))
    }

    /// How the fidelity of this file should be described to a person.
    pub fn quality_summary(&self) -> String {
        if self.is_lossless() == Some(true) {
            let mut parts: Vec<String> = Vec::new();
            if let Some(rate) = self.sample_rate {
                if rate > 0.0 {
                    parts.push(format!("{} kHz", trim_number(rate / 1000.0)));
                }
            }
            if let Some(depth) = self.bit_depth {
                if depth > 0 {
                    parts.push(format!("{depth}-bit"));
                }
            }
            if !parts.is_empty() {
                return parts.join(" · ");
            }
        }
        match self.bitrate_kbps() {
            Some(rate) => format!("{rate} kbps"),
            None => "—".into(),
        }
    }
}

fn trim_number(value: f64) -> String {
    let s = format!("{:.4}", value);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.is_empty() { "0".into() } else { s }
}

/// Orders two files worst-to-best on audio fidelity alone.
///
/// File size deliberately plays no part until everything else ties: a FLAC and
/// a WAV made from the same master are the same audio, and the WAV being three
/// times larger does not make it better. When fidelity really is equal, the
/// smaller file is the more sensible copy to keep.
pub fn quality_cmp(a: &AudioFile, b: &AudioFile) -> Ordering {
    // A file that could not be decoded tells us nothing about its own quality,
    // so it never outranks one we were able to inspect.
    let (a_inspected, b_inspected) = (a.duration.is_some(), b.duration.is_some());
    if a_inspected != b_inspected {
        return if a_inspected { Ordering::Greater } else { Ordering::Less };
    }

    // Lossless beats lossy, whenever the codec is known for both files.
    if let (Some(al), Some(bl)) = (a.is_lossless(), b.is_lossless()) {
        if al != bl {
            return if al { Ordering::Greater } else { Ordering::Less };
        }
    }

    // Bitrate only means anything for lossy audio; for lossless it just
    // reflects how well the encoder packed identical samples.
    if a.is_lossless() != Some(true) && b.is_lossless() != Some(true) {
        let (ar, br) = (a.bitrate_kbps().unwrap_or(0), b.bitrate_kbps().unwrap_or(0));
        if ar != br {
            return ar.cmp(&br);
        }
    }

    // Only compare an attribute when it is known for both files. Compressed
    // lossless formats report no bit depth, and treating "unknown" as zero
    // would make a FLAC lose to a WAV holding the very same audio.
    if let (Some(ar), Some(br)) = (a.sample_rate, b.sample_rate) {
        if ar != br {
            return ar.partial_cmp(&br).unwrap_or(Ordering::Equal);
        }
    }
    if let (Some(ad), Some(bd)) = (a.bit_depth, b.bit_depth) {
        if ad != bd {
            return ad.cmp(&bd);
        }
    }
    if let (Some(ac), Some(bc)) = (a.channel_count, b.channel_count) {
        if ac != bc {
            return ac.cmp(&bc);
        }
    }

    // Identical fidelity: prefer the copy that wastes less space, then the one
    // nearer the top of the folder tree, so the result is stable.
    if a.byte_size != b.byte_size {
        return b.byte_size.cmp(&a.byte_size);
    }
    let (al, bl) = (a.path.display().to_string().len(), b.path.display().to_string().len());
    bl.cmp(&al)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchKind {
    Exact,
    Similar,
}

impl MatchKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchKind::Exact => "exact",
            MatchKind::Similar => "similar",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            MatchKind::Exact => "Identical files",
            MatchKind::Similar => "Same audio",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DuplicateGroup {
    pub kind: MatchKind,
    pub files: Vec<AudioFile>,
    pub confidence: f64,
}

impl DuplicateGroup {
    /// Bytes that would be freed by keeping only one file.
    pub fn reclaimable_bytes(&self) -> u64 {
        if self.files.len() < 2 {
            return 0;
        }
        let total: u64 = self.files.iter().map(|f| f.byte_size).sum();
        let keep = self.files.iter().map(|f| f.byte_size).max().unwrap_or(0);
        total - keep
    }
}
