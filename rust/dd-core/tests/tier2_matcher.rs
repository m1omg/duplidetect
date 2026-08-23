//! Tier 2: the matcher, fed the fingerprints the Swift implementation produced,
//! must arrive at the same groups. Running it on Swift-derived fingerprints
//! keeps the decoder and resampler out of the comparison, so a failure here is
//! a matching bug.

use dd_core::fingerprint::Fingerprint;
use dd_core::matcher::{groups, MatchLevel, Options};
use serde_json::Value;
use std::path::Path;

fn golden_dir() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden"))
}

fn load(corpus: &str) -> (Vec<String>, Vec<Option<Fingerprint>>, Vec<Option<f64>>) {
    let g: Value = serde_json::from_str(
        &std::fs::read_to_string(golden_dir().join(format!("fp-{corpus}.json"))).unwrap(),
    ).unwrap();

    let mut names = Vec::new();
    let mut prints = Vec::new();
    let mut durations = Vec::new();
    for entry in g["files"].as_array().unwrap() {
        names.push(entry["file"].as_str().unwrap().to_string());
        if entry.get("error").is_some() {
            prints.push(None);
            durations.push(None);
            continue;
        }
        prints.push(Some(Fingerprint {
            values: entry["values"].as_array().unwrap().iter()
                .map(|v| v.as_str().unwrap().parse::<u32>().unwrap()).collect(),
            shape_profile: entry["shapeProfile"].as_array().unwrap().iter()
                .map(|v| v.as_u64().unwrap() as u8).collect(),
            flux: f64::from_bits(entry["fluxBits"].as_str().unwrap().parse().unwrap()),
        }));
        // Content duration, as the scanner computes it: full duration minus the
        // silence trimmed from each end (the tail only when not truncated).
        let duration = entry["sourceDuration"].as_f64().unwrap();
        let lead = f64::from_bits(entry["leadingSilenceBits"].as_str().unwrap().parse().unwrap());
        let trail = if entry["wasTruncated"].as_bool().unwrap() {
            0.0
        } else {
            f64::from_bits(entry["trailingSilenceBits"].as_str().unwrap().parse().unwrap())
        };
        durations.push(Some((duration - lead - trail).max(0.0)));
    }
    (names, prints, durations)
}

/// The "similar" groups the Swift scanner reported, as sorted filename sets.
fn swift_similar_groups(corpus: &str) -> Vec<Vec<String>> {
    let g: Value = serde_json::from_str(
        &std::fs::read_to_string(golden_dir().join(format!("scan-{corpus}.json"))).unwrap(),
    ).unwrap();
    let mut out: Vec<Vec<String>> = g["groups"].as_array().unwrap().iter()
        .filter(|grp| grp["kind"].as_str().unwrap() == "similar")
        .map(|grp| {
            let mut paths: Vec<String> = grp["paths"].as_array().unwrap().iter()
                .map(|p| p.as_str().unwrap().to_string()).collect();
            paths.sort();
            paths
        })
        .collect();
    out.sort();
    out
}

fn check(corpus: &str) {
    let (names, prints, durations) = load(corpus);
    let found = groups(&prints, &durations, &Options::for_level(MatchLevel::Perfect));

    let mut actual: Vec<Vec<String>> = found.iter()
        .map(|g| {
            let mut p: Vec<String> = g.indices.iter().map(|&i| names[i].clone()).collect();
            p.sort();
            p
        })
        .collect();
    actual.sort();

    let expected = swift_similar_groups(corpus);
    eprintln!("  {corpus}: {} groups (swift {})", actual.len(), expected.len());
    for g in &actual {
        eprintln!("      {:?}", g);
    }
    assert_eq!(actual, expected, "{corpus} grouping differs from the Swift reference");
}

#[test]
fn grouping_matches_the_reference() {
    check("fixtures");
}
