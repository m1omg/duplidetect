//! Folder walking and the two-pass scan, ported from Scanner.swift.

use crate::decode;
use dd_core::fingerprint::{FingerprintEngine, FINGERPRINT_RATE};
use dd_core::hash::content_hash;
use dd_core::matcher;
use dd_core::model::{AudioFile, DuplicateGroup, MatchKind};
use dd_core::{formats, fingerprint::Fingerprint};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Clone, Debug)]
pub struct ScanOptions {
    pub include_subfolders: bool,
    pub find_exact_duplicates: bool,
    pub find_similar_audio: bool,
    pub level: matcher::MatchLevel,
    pub minimum_duration: f64,
    pub fingerprint_seconds: f64,
    pub skip_hidden_files: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        ScanOptions {
            include_subfolders: true,
            find_exact_duplicates: true,
            find_similar_audio: true,
            level: matcher::MatchLevel::Perfect,
            minimum_duration: 2.0,
            fingerprint_seconds: 120.0,
            skip_hidden_files: true,
        }
    }
}

#[derive(Default)]
pub struct ScanResult {
    pub groups: Vec<DuplicateGroup>,
    pub files_scanned: usize,
    pub files_skipped: Vec<(PathBuf, String)>,
}

impl ScanResult {
    pub fn reclaimable_bytes(&self) -> u64 {
        self.groups.iter().map(|g| g.reclaimable_bytes()).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Phase {
    Collecting,
    Hashing { done: usize, total: usize },
    Fingerprinting { done: usize, total: usize },
    Matching,
    Finished,
}

/// Hidden-file rules differ by platform: a leading dot on Unix, the hidden
/// attribute on Windows.
#[cfg(windows)]
fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with('.'))
        .unwrap_or(false)
        || std::fs::metadata(path)
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
            .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).map(|n| n.starts_with('.')).unwrap_or(false)
}

/// A stable identity for a file, so the same one reached by two paths is only
/// counted once. Windows paths are case-insensitive.
fn canonical_identity(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = resolved.display().to_string();
    if cfg!(windows) { s.to_lowercase() } else { s }
}

pub fn collect(roots: &[PathBuf], options: &ScanOptions) -> Vec<AudioFile> {
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();

    for root in roots {
        let walker = WalkDir::new(root)
            .follow_links(false)
            .max_depth(if options.include_subfolders { usize::MAX } else { 1 });
        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if !entry.file_type().is_file() || !formats::is_audio(path) {
                continue;
            }
            if options.skip_hidden_files
                && path.components().any(|c| is_hidden(Path::new(c.as_os_str())))
            {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if meta.len() == 0 {
                continue;
            }
            if !seen.insert(canonical_identity(path)) {
                continue;
            }
            files.push(AudioFile::new(
                path.to_path_buf(),
                meta.len(),
                meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH),
            ));
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

pub fn run<F: Fn(Phase) + Sync>(
    roots: &[PathBuf],
    options: &ScanOptions,
    progress: F,
) -> ScanResult {
    let mut result = ScanResult::default();
    progress(Phase::Collecting);
    let mut files = collect(roots, options);
    result.files_scanned = files.len();
    if files.is_empty() {
        return result;
    }

    // Representative index -> every file byte-identical to it.
    let mut exact_members: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut exact_groups: Vec<Vec<usize>> = Vec::new();

    // ---- Pass 1: byte-identical files -------------------------------
    if options.find_exact_duplicates {
        exact_groups = find_exact_duplicates(&mut files, &progress);
        for indices in &exact_groups {
            if let Some(&first) = indices.first() {
                exact_members.insert(first, indices.clone());
            }
        }
    }

    let mut similar_groups: Vec<(Vec<usize>, f64)> = Vec::new();

    // ---- Pass 2: same audio, different bytes ------------------------
    if options.find_similar_audio {
        // Hashing already proved the members of an exact group identical, so
        // only one of each needs to be decoded and fingerprinted.
        let collapsed: std::collections::HashSet<usize> = exact_members
            .values()
            .flat_map(|v| v.iter().cloned())
            .filter(|i| !exact_members.contains_key(i))
            .collect();
        let representatives: Vec<usize> =
            (0..files.len()).filter(|i| !collapsed.contains(i)).collect();

        let fingerprints = compute_fingerprints(&representatives, &mut files, options, &mut result, &progress);

        // Byte-identical files share the representative's audio properties.
        let pairs: Vec<(usize, Vec<usize>)> =
            exact_members.iter().map(|(k, v)| (*k, v.clone())).collect();
        for (representative, members) in pairs {
            for member in members {
                if member == representative {
                    continue;
                }
                files[member].duration = files[representative].duration;
                files[member].content_duration = files[representative].content_duration;
                files[member].sample_rate = files[representative].sample_rate;
                files[member].bit_depth = files[representative].bit_depth;
                files[member].channel_count = files[representative].channel_count;
                files[member].format_name = files[representative].format_name.clone();
                files[member].codec_is_lossless = files[representative].codec_is_lossless;
                files[member].fingerprint = files[representative].fingerprint.clone();
            }
        }

        progress(Phase::Matching);
        let durations: Vec<Option<f64>> = files.iter().map(|f| f.content_duration).collect();
        let matches = matcher::groups(
            &fingerprints,
            &durations,
            &matcher::Options::for_level(options.level),
        );
        similar_groups = matches
            .into_iter()
            .map(|g| {
                let members: Vec<usize> = g
                    .indices
                    .iter()
                    .flat_map(|i| exact_members.get(i).cloned().unwrap_or_else(|| vec![*i]))
                    .collect();
                (members, g.confidence)
            })
            .collect();
    }

    // An exact group wholly contained in a similar group is already represented
    // there; keeping both would double-count the wasted space.
    let absorbed: Vec<std::collections::HashSet<usize>> = similar_groups
        .iter()
        .map(|(m, _)| m.iter().cloned().collect())
        .collect();
    for indices in &exact_groups {
        let set: std::collections::HashSet<usize> = indices.iter().cloned().collect();
        if absorbed.iter().any(|a| a.is_superset(&set)) {
            continue;
        }
        result.groups.push(DuplicateGroup {
            kind: MatchKind::Exact,
            files: indices.iter().map(|&i| files[i].clone()).collect(),
            confidence: 1.0,
        });
    }
    for (members, confidence) in similar_groups {
        let mut sorted = members;
        sorted.sort_unstable();
        result.groups.push(DuplicateGroup {
            kind: MatchKind::Similar,
            files: sorted.iter().map(|&i| files[i].clone()).collect(),
            confidence,
        });
    }

    result.groups.sort_by(|a, b| b.reclaimable_bytes().cmp(&a.reclaimable_bytes()));
    progress(Phase::Finished);
    result
}

fn find_exact_duplicates<F: Fn(Phase) + Sync>(
    files: &mut [AudioFile],
    progress: &F,
) -> Vec<Vec<usize>> {
    // Only files sharing a byte size can possibly be identical.
    let mut by_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (i, f) in files.iter().enumerate() {
        by_size.entry(f.byte_size).or_default().push(i);
    }
    let mut candidates: Vec<usize> =
        by_size.values().filter(|v| v.len() > 1).flatten().cloned().collect();
    candidates.sort_unstable();
    if candidates.is_empty() {
        return Vec::new();
    }

    let total = candidates.len();
    progress(Phase::Hashing { done: 0, total });
    let done = std::sync::atomic::AtomicUsize::new(0);

    let hashes: Vec<(usize, String)> = candidates
        .par_iter()
        .filter_map(|&i| {
            let h = content_hash(&files[i].path).ok().map(|h| (i, h));
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n % 8 == 0 || n == total {
                progress(Phase::Hashing { done: n, total });
            }
            h
        })
        .collect();

    let mut by_hash: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, h) in hashes {
        files[i].content_hash = Some(h.clone());
        by_hash.entry(h).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = by_hash
        .into_values()
        .filter(|v| v.len() > 1)
        .map(|mut v| {
            v.sort_unstable();
            v
        })
        .collect();
    groups.sort();
    groups
}

fn compute_fingerprints<F: Fn(Phase) + Sync>(
    indices: &[usize],
    files: &mut [AudioFile],
    options: &ScanOptions,
    result: &mut ScanResult,
    progress: &F,
) -> Vec<Option<Fingerprint>> {
    let total = indices.len();
    let mut fingerprints: Vec<Option<Fingerprint>> = vec![None; files.len()];
    if total == 0 {
        return fingerprints;
    }
    progress(Phase::Fingerprinting { done: 0, total });
    let done = std::sync::atomic::AtomicUsize::new(0);

    let paths: Vec<(usize, PathBuf)> = indices.iter().map(|&i| (i, files[i].path.clone())).collect();

    enum Outcome {
        Ok(usize, Box<decode::DecodedAudio>, dd_core::fingerprint::AudioAnalysis),
        Skip(PathBuf, String),
    }

    let outcomes: Vec<Outcome> = paths
        .par_iter()
        .map_init(FingerprintEngine::new, |engine, (index, path)| {
            let outcome = match decode::decode_for_fingerprint(path, options.fingerprint_seconds) {
                Ok(audio) => {
                    if audio.source_duration >= options.minimum_duration {
                        let analysis = engine.analyze(&audio.samples, FINGERPRINT_RATE);
                        Outcome::Ok(*index, Box::new(audio), analysis)
                    } else {
                        Outcome::Skip(
                            path.clone(),
                            format!("shorter than {}s", options.minimum_duration as i64),
                        )
                    }
                }
                Err(e) => Outcome::Skip(path.clone(), describe(&e, path)),
            };
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            if n % 4 == 0 || n == total {
                progress(Phase::Fingerprinting { done: n, total });
            }
            outcome
        })
        .collect();

    for outcome in outcomes {
        match outcome {
            Outcome::Ok(index, audio, analysis) => {
                fingerprints[index] = Some(analysis.fingerprint.clone());
                files[index].fingerprint = Some(analysis.fingerprint);
                files[index].duration = Some(audio.source_duration);
                files[index].sample_rate = Some(audio.source_sample_rate);
                files[index].bit_depth = audio.source_bit_depth;
                files[index].channel_count = Some(audio.source_channels);
                files[index].format_name = Some(audio.format_name.clone());
                files[index].codec_is_lossless = Some(audio.is_lossless);

                // Padding is not content. When the decode stopped early the
                // tail was never seen, so only the head silence is discounted.
                let trailing = if audio.was_truncated { 0.0 } else { analysis.trailing_silence };
                files[index].content_duration =
                    Some((audio.source_duration - analysis.leading_silence - trailing).max(0.0));
            }
            Outcome::Skip(path, reason) => result.files_skipped.push((path, reason)),
        }
    }
    fingerprints
}

fn describe(error: &decode::DecodeError, path: &Path) -> String {
    if formats::is_byte_comparable_only(path) {
        return format!(
            "no {} decoder available — only exact copies of this file can be found",
            formats::label(path)
        );
    }
    error.to_string()
}
