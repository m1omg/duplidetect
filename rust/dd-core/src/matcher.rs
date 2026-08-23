//! Fingerprint matching, ported from Sources/DupliDetect/Matcher.swift.
//!
//! Three places in the Swift original depend on `Dictionary` iteration order,
//! which Swift randomises per process: the offset tie-break, the transitive
//! confidence fallback, and the final group sort. They do not currently change
//! its output on any test corpus, but they could. This port fixes an explicit
//! order in each case so results are reproducible across runs and platforms.

use crate::fingerprint::{seconds_per_frame, shape_distance, Fingerprint};
use crate::unionfind::UnionFind;
use std::collections::HashMap;

/// Sub-fingerprints are indexed in two 16-bit halves, so a pair only has to
/// agree on one half to become a candidate.
const INDEX_STRIDE: usize = 2;
/// Aligned frames a pair needs before it is scored at all.
const MINIMUM_VOTES: usize = 6;
/// A hash shared by this many entries carries no information.
const MAXIMUM_ENTRIES_PER_HASH: usize = 400;

/// How demanding matching should be.
///
/// `Perfect` is not merely a tighter bit-error threshold. Bit-error rate alone
/// cannot separate a genuine duplicate from an excerpt: measured across the
/// corpus, real duplicates of one master span 0.000 (any lossless conversion)
/// up to 0.086 (Ogg Vorbis q5), while a four-second excerpt of a ten-second
/// recording of the same tune scores 0.101 — the two populations nearly touch.
/// So `Perfect` adds a structural requirement instead: the matching region must
/// cover essentially the whole of *both* files. That is what "1:1" means, and
/// it rules out excerpts however well they align.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchLevel {
    Perfect,
    VeryStrict,
    Strict,
    Relaxed,
    VeryRelaxed,
}

impl MatchLevel {
    pub const ALL: [MatchLevel; 5] = [
        MatchLevel::Perfect,
        MatchLevel::VeryStrict,
        MatchLevel::Strict,
        MatchLevel::Relaxed,
        MatchLevel::VeryRelaxed,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            MatchLevel::Perfect => "Perfect match",
            MatchLevel::VeryStrict => "Very strict",
            MatchLevel::Strict => "Strict",
            MatchLevel::Relaxed => "Relaxed",
            MatchLevel::VeryRelaxed => "Very relaxed",
        }
    }

    pub fn explanation(&self) -> &'static str {
        match self {
            MatchLevel::Perfect => "The same recording end to end — true 1:1 duplicates. A shorter excerpt is not grouped.",
            MatchLevel::VeryStrict => "Near-identical recordings, including a clip taken from a longer one.",
            MatchLevel::Strict => "Near-identical recordings, with a little more tolerance.",
            MatchLevel::Relaxed => "Also groups heavily re-encoded copies. Review before deleting.",
            MatchLevel::VeryRelaxed => "Groups loosely similar audio. Expect false positives.",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            MatchLevel::Perfect => "perfect",
            MatchLevel::VeryStrict => "very-strict",
            MatchLevel::Strict => "strict",
            MatchLevel::Relaxed => "relaxed",
            MatchLevel::VeryRelaxed => "very-relaxed",
        }
    }

    pub fn parse(text: &str) -> Option<MatchLevel> {
        MatchLevel::ALL.into_iter().find(|l| l.as_str() == text)
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub maximum_bit_error_rate: f64,
    pub minimum_overlap_fraction: f64,
    pub minimum_overlap_seconds: f64,
    pub maximum_shape_distance: f64,
    pub stationary_duration_tolerance: f64,
    pub stationary_duration_slack: f64,
    /// When set, the matching region must cover essentially the whole of both
    /// files rather than a fraction of the shorter one.
    pub require_whole_file: bool,
    /// The fraction of the *longer* file the overlap must then cover.
    pub whole_file_fraction: f64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            maximum_bit_error_rate: 0.22,
            minimum_overlap_fraction: 0.25,
            minimum_overlap_seconds: 3.0,
            maximum_shape_distance: 0.27,
            stationary_duration_tolerance: 0.10,
            stationary_duration_slack: 0.5,
            require_whole_file: false,
            whole_file_fraction: 0.95,
        }
    }
}

impl Options {
    pub fn for_level(level: MatchLevel) -> Self {
        let base = Options::default();
        match level {
            // The same audio tolerance as Very strict, plus whole-file
            // correspondence. The structural rule excludes non-duplicates, so
            // the threshold stays generous enough for Ogg Vorbis at 0.086.
            MatchLevel::Perfect => Options {
                maximum_bit_error_rate: 0.12,
                maximum_shape_distance: 0.20,
                require_whole_file: true,
                stationary_duration_tolerance: 0.05,
                ..base
            },
            MatchLevel::VeryStrict => Options {
                maximum_bit_error_rate: 0.12,
                maximum_shape_distance: 0.20,
                ..base
            },
            MatchLevel::Strict => Options {
                maximum_bit_error_rate: 0.18,
                maximum_shape_distance: 0.27,
                ..base
            },
            MatchLevel::Relaxed => Options {
                maximum_bit_error_rate: 0.24,
                maximum_shape_distance: 0.33,
                ..base
            },
            MatchLevel::VeryRelaxed => Options {
                maximum_bit_error_rate: 0.30,
                maximum_shape_distance: 0.40,
                ..base
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct Group {
    pub indices: Vec<usize>,
    pub confidence: f64,
}

/// Whether two files may be called duplicates given their lengths.
///
/// Only stationary audio is constrained. Normal recordings may differ freely,
/// because matching a short excerpt against the full recording is a genuine
/// result. For a held tone it is not: every second sounds like every other, so
/// a ten-second tone aligns perfectly inside a ten-minute one.
fn lengths_allow_match(a: &Fingerprint, b: &Fingerprint, len_a: f64, len_b: f64, o: &Options) -> bool {
    if !a.is_stationary() || !b.is_stationary() {
        return true;
    }
    let longer = len_a.max(len_b);
    let shorter = len_a.min(len_b);
    let allowed = o.stationary_duration_slack.max(longer * o.stationary_duration_tolerance);
    longer - shorter <= allowed
}

/// Bit-error rate between two fingerprint sequences at a fixed frame offset.
/// `offset` is how far `a` leads `b`.
fn compare(a: &[u32], b: &[u32], offset: i64) -> (f64, usize) {
    let start_a = offset.max(0) as usize;
    let start_b = (-offset).max(0) as usize;
    if start_a >= a.len() || start_b >= b.len() {
        return (1.0, 0);
    }
    let overlap = (a.len() - start_a).min(b.len() - start_b);
    if overlap == 0 {
        return (1.0, 0);
    }
    let mut differing = 0u64;
    for i in 0..overlap {
        differing += (a[start_a + i] ^ b[start_b + i]).count_ones() as u64;
    }
    (differing as f64 / (overlap * 32) as f64, overlap)
}

pub fn groups(
    fingerprints: &[Option<Fingerprint>],
    durations: &[Option<f64>],
    options: &Options,
) -> Vec<Group> {
    let usable: Vec<usize> = (0..fingerprints.len())
        .filter(|&i| fingerprints[i].as_ref().map(|f| f.is_usable()).unwrap_or(false))
        .collect();
    if usable.len() < 2 {
        return Vec::new();
    }

    let length_of = |index: usize| -> f64 {
        durations
            .get(index)
            .and_then(|d| *d)
            .unwrap_or_else(|| fingerprints[index].as_ref().map(|f| f.duration()).unwrap_or(0.0))
    };

    // Inverted index over both halves of each sub-fingerprint.
    let mut index: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut owner: Vec<u32> = Vec::new();
    let mut position: Vec<u32> = Vec::new();

    for &file in &usable {
        let values = &fingerprints[file].as_ref().unwrap().values;
        let mut frame = 0;
        while frame < values.len() {
            let value = values[frame];
            let entry = owner.len() as u32;
            owner.push(file as u32);
            position.push(frame as u32);
            let low = value & 0xFFFF;
            let high = (value >> 16) | 0x1_0000; // tag so halves never collide
            index.entry(low).or_default().push(entry);
            index.entry(high).or_default().push(entry);
            frame += INDEX_STRIDE;
        }
    }

    // Vote for (pair, offset) combinations. Summation is order-independent.
    let mut votes: HashMap<(usize, usize), HashMap<i64, usize>> = HashMap::new();
    for entries in index.values() {
        if entries.len() < 2 || entries.len() > MAXIMUM_ENTRIES_PER_HASH {
            continue;
        }
        for i in 0..entries.len() {
            for j in (i + 1)..entries.len() {
                let (ea, eb) = (entries[i] as usize, entries[j] as usize);
                let (fa, fb) = (owner[ea] as usize, owner[eb] as usize);
                if fa == fb {
                    continue;
                }
                let (low, high, offset) = if fa < fb {
                    (fa, fb, position[ea] as i64 - position[eb] as i64)
                } else {
                    (fb, fa, position[eb] as i64 - position[ea] as i64)
                };
                *votes.entry((low, high)).or_default().entry(offset).or_insert(0) += 1;
            }
        }
    }

    let mut union = UnionFind::new(fingerprints.len());
    let mut pair_scores: HashMap<(usize, usize), f64> = HashMap::new();

    for (&(ia, ib), offsets) in &votes {
        // Deterministic: most votes, then the smallest offset.
        let best = offsets
            .iter()
            .max_by(|x, y| x.1.cmp(y.1).then(y.0.cmp(x.0)))
            .map(|(o, c)| (*o, *c));
        let Some((offset, count)) = best else { continue };
        if count < MINIMUM_VOTES {
            continue;
        }
        let (a, b) = (fingerprints[ia].as_ref().unwrap(), fingerprints[ib].as_ref().unwrap());
        if !lengths_allow_match(a, b, length_of(ia), length_of(ib), options) {
            continue;
        }

        let (error_rate, overlap) = compare(&a.values, &b.values, offset);
        if overlap == 0 {
            continue;
        }
        let overlap_seconds = overlap as f64 * seconds_per_frame();
        let shorter = a.values.len().min(b.values.len());
        let longer = a.values.len().max(b.values.len());
        // A 1:1 duplicate lines up across the whole of both files. Measuring the
        // overlap against the *longer* one is what rules out an excerpt, which
        // otherwise aligns perfectly over the whole of its own length.
        let enough_overlap = if options.require_whole_file {
            overlap as f64 >= longer as f64 * options.whole_file_fraction
        } else {
            overlap as f64 >= shorter as f64 * options.minimum_overlap_fraction
        };
        if overlap_seconds < options.minimum_overlap_seconds
            || !enough_overlap
            || error_rate > options.maximum_bit_error_rate
        {
            continue;
        }

        // Map bit-error rate onto a 0...1 confidence; 0.5 is pure chance.
        let confidence = (1.0 - error_rate / 0.5).clamp(0.0, 1.0);
        pair_scores.insert((ia, ib), confidence);
        union.merge(ia, ib);
    }

    // Stationary audio: `values` is rounding noise, so compare spectral shape.
    let stationary: Vec<usize> = usable
        .iter()
        .cloned()
        .filter(|&i| {
            let f = fingerprints[i].as_ref().unwrap();
            f.is_stationary() && !f.shape_profile.is_empty()
        })
        .collect();
    for i in 0..stationary.len() {
        for j in (i + 1)..stationary.len() {
            let (first, second) = (stationary[i], stationary[j]);
            let (a, b) = (fingerprints[first].as_ref().unwrap(), fingerprints[second].as_ref().unwrap());
            if !lengths_allow_match(a, b, length_of(first), length_of(second), options) {
                continue;
            }
            let distance = shape_distance(&a.shape_profile, &b.shape_profile);
            if distance > options.maximum_shape_distance {
                continue;
            }
            let key = (first.min(second), first.max(second));
            let confidence = (1.0 - distance / 0.6).clamp(0.0, 1.0);
            let slot = pair_scores.entry(key).or_insert(confidence);
            if confidence > *slot {
                *slot = confidence;
            }
            union.merge(first, second);
        }
    }

    let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
    for &file in &usable {
        if pair_scores.keys().any(|&(a, b)| a == file || b == file) {
            let root = union.find(file);
            buckets.entry(root).or_default().push(file);
        }
    }

    let mut out: Vec<Group> = Vec::new();
    for members in buckets.values() {
        if members.len() < 2 {
            continue;
        }
        let mut sorted = members.clone();
        sorted.sort_unstable();

        let mut weakest = 1.0f64;
        let mut found = false;
        for i in 0..sorted.len() {
            for j in (i + 1)..sorted.len() {
                if let Some(&score) = pair_scores.get(&(sorted[i], sorted[j])) {
                    weakest = weakest.min(score);
                    found = true;
                }
            }
        }
        if !found {
            // Groups linked transitively may have no direct score for any pair
            // listed here. Deterministic: take the lowest score involving a member.
            weakest = pair_scores
                .iter()
                .filter(|(&(a, b), _)| sorted.contains(&a) || sorted.contains(&b))
                .map(|(_, &v)| v)
                .fold(1.0f64, f64::min);
        }
        out.push(Group { indices: sorted, confidence: weakest });
    }

    // Canonical order, independent of hash iteration.
    out.sort_by(|a, b| a.indices.cmp(&b.indices));
    out
}
