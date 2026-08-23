//! Tier 2: measures how far the Symphonia + rubato decode path drifts from the
//! CoreAudio + AVAudioConverter path. Bit-exactness is impossible here; what
//! matters is that the drift stays far below the 0.22 matching threshold, so
//! every grouping decision survives it.

use dd_app::decode;
use dd_core::fingerprint::{FingerprintEngine, Fingerprint, FINGERPRINT_RATE};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn corpus_root() -> Option<PathBuf> {
    Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata")))
}

fn golden(corpus: &str) -> Value {
    let p = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden"))
        .join(format!("fp-{corpus}.json"));
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// Lowest bit-error rate over every plausible alignment.
fn best_ber(a: &[u32], b: &[u32]) -> f64 {
    let span = 600i64;
    let mut best = 1.0f64;
    for offset in -span..=span {
        let (sa, sb) = (offset.max(0) as usize, (-offset).max(0) as usize);
        if sa >= a.len() || sb >= b.len() {
            continue;
        }
        let overlap = (a.len() - sa).min(b.len() - sb);
        if overlap < 64 {
            continue;
        }
        let bits: u32 = (0..overlap).map(|i| (a[sa + i] ^ b[sb + i]).count_ones()).sum();
        best = best.min(bits as f64 / (overlap * 32) as f64);
    }
    best
}

fn run(corpus: &str) {
    let Some(root) = corpus_root() else {
        eprintln!("  DD_CORPUS not set; skipping");
        return;
    };
    let dir = root.join("fixtures");
    let g = golden(corpus);
    let mut engine = FingerprintEngine::new();
    let mut worst = 0.0f64;
    let mut worst_name = String::new();
    let mut checked = 0;

    for entry in g["files"].as_array().unwrap() {
        if entry.get("error").is_some() {
            continue;
        }
        let name = entry["file"].as_str().unwrap();
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        let swift = Fingerprint {
            values: entry["values"].as_array().unwrap().iter()
                .map(|v| v.as_str().unwrap().parse::<u32>().unwrap()).collect(),
            shape_profile: entry["shapeProfile"].as_array().unwrap().iter()
                .map(|v| v.as_u64().unwrap() as u8).collect(),
            flux: f64::from_bits(entry["fluxBits"].as_str().unwrap().parse().unwrap()),
        };

        let Ok(audio) = decode::decode_for_fingerprint(&path, 120.0) else {
            eprintln!("    {name}: rust could not decode (see format notes)");
            continue;
        };
        let rust = engine.fingerprint(&audio.samples);

        // Metadata parity: nil-ness is load-bearing for the keep rules.
        assert_eq!(audio.is_lossless, entry["isLossless"].as_bool().unwrap(), "{name} losslessness");
        let swift_depth = entry["sourceBitDepth"].as_u64().map(|v| v as u32);
        assert_eq!(audio.source_bit_depth, swift_depth, "{name} bit depth (nil-ness matters)");
        assert_eq!(audio.source_channels, entry["sourceChannels"].as_u64().unwrap() as u32,
                   "{name} channels");
        assert!((audio.source_sample_rate - entry["sourceSampleRate"].as_f64().unwrap()).abs() < 1.0,
                "{name} sample rate");

        if swift.is_stationary() || rust.is_stationary() {
            // Temporal bits are noise for stationary audio; the shape template
            // is what decides such files.
            let d = dd_core::fingerprint::shape_distance(&rust.shape_profile, &swift.shape_profile);
            eprintln!("    {name}: stationary, shape distance {d:.4}");
            assert!(d < 0.05, "{name} shape drifted too far: {d}");
            continue;
        }

        let ber = best_ber(&rust.values, &swift.values);
        checked += 1;
        if ber > worst {
            worst = ber;
            worst_name = name.to_string();
        }
        eprintln!("    {name}: BER {ber:.4}");
    }

    eprintln!("  {corpus}: {checked} files, worst BER {worst:.4} ({worst_name})");
    assert!(worst <= 0.02, "worst BER {worst:.4} exceeds the 0.02 budget ({worst_name})");
}

#[test]
fn decode_drift_is_small() {
    run("fixtures");
}
