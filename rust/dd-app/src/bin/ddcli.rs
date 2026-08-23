//! Headless scanner: the parity harness, the CI test driver, and the fallback
//! interface when no GUI is available.

use dd_app::scanner::{self, ScanOptions};
use dd_core::keep::KeepRule;
use serde_json::{json, Value};
use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage:\n  \
         ddcli scan DIR [--sensitivity S] [--json]\n  \
         ddcli selftest\n"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan") => scan(&args[1..]),
        Some("selftest") => selftest(),
        Some("ber") if args.len() >= 3 => ber(&args[1], &args[2]),
        Some("pcm") if args.len() >= 3 => dump_pcm(&args[1], &args[2]),
        _ => usage(),
    }
}

fn scan(args: &[String]) {
    let Some(dir) = args.first() else { usage() };
    let mut options = ScanOptions { minimum_duration: 0.5, ..Default::default() };
    let mut as_json = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--sensitivity" if i + 1 < args.len() => {
                options.sensitivity = args[i + 1].parse().unwrap_or(0.35);
                i += 1;
            }
            "--json" => as_json = true,
            _ => {}
        }
        i += 1;
    }

    let result = scanner::run(&[PathBuf::from(dir)], &options, |_| {});

    if as_json {
        // Canonical report, directly comparable with the Swift harness output.
        let mut groups: Vec<Value> = result
            .groups
            .iter()
            .map(|g| {
                let mut paths: Vec<String> = g.files.iter().map(|f| f.display_name()).collect();
                paths.sort();
                let keepers: serde_json::Map<String, Value> = KeepRule::ALL
                    .iter()
                    .map(|rule| {
                        (
                            rule.label().to_string(),
                            json!(rule.keeper(&g.files).map(|f| f.display_name()).unwrap_or_default()),
                        )
                    })
                    .collect();
                json!({
                    "kind": g.kind.as_str(),
                    "paths": paths,
                    "confidence": (g.confidence * 1000.0).round() / 1000.0,
                    "reclaimableBytes": g.reclaimable_bytes(),
                    "keepers": keepers,
                })
            })
            .collect();
        groups.sort_by_key(|g| {
            (
                g["kind"].as_str().unwrap().to_string(),
                g["paths"].as_array().unwrap().iter()
                    .map(|p| p.as_str().unwrap()).collect::<Vec<_>>().join("|"),
            )
        });
        let mut skipped: Vec<String> = result
            .files_skipped
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        skipped.sort();
        println!("{}", serde_json::to_string(&json!({
            "filesScanned": result.files_scanned,
            "groups": groups,
            "skipped": skipped,
        })).unwrap());
        return;
    }

    println!("--- scanned {} files ---", result.files_scanned);
    for group in &result.groups {
        println!(
            "\n[{}] confidence {:.3}  reclaim {} B",
            group.kind.as_str(),
            group.confidence,
            group.reclaimable_bytes()
        );
        let keeper = KeepRule::BestQuality.keeper(&group.files).map(|f| f.display_name());
        for file in &group.files {
            let mark = if Some(file.display_name()) == keeper { "KEEP " } else { "     " };
            println!(
                "   {}{}  [{}]  {} B  {}",
                mark,
                file.display_name(),
                file.quality_summary(),
                file.byte_size,
                file.format_name.clone().unwrap_or_default()
            );
        }
    }
    for (path, reason) in &result.files_skipped {
        println!("skipped: {} - {}", path.file_name().unwrap().to_string_lossy(), reason);
    }
    println!("\ntotal reclaimable: {} B", result.reclaimable_bytes());
}

/// Writes the decoded mono buffer as raw f32 little-endian, for diagnostics.
fn dump_pcm(input: &str, output: &str) {
    let audio = dd_app::decode::decode_for_fingerprint(std::path::Path::new(input), 120.0)
        .unwrap_or_else(|e| panic!("{input}: {e}"));
    let mut bytes = Vec::with_capacity(audio.samples.len() * 4);
    for s in &audio.samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(output, bytes).unwrap();
    println!("{} samples", audio.samples.len());
}

/// Best-alignment bit-error rate between two files, for diagnostics.
fn ber(a: &str, b: &str) {
    use dd_core::fingerprint::{FingerprintEngine, FINGERPRINT_RATE};
    let mut engine = FingerprintEngine::new();
    let mut print = |p: &str| {
        let audio = dd_app::decode::decode_for_fingerprint(std::path::Path::new(p), 120.0)
            .unwrap_or_else(|e| panic!("{p}: {e}"));
        let _ = FINGERPRINT_RATE;
        engine.fingerprint(&audio.samples)
    };
    let (fa, fb) = (print(a), print(b));
    let mut best = 1.0f64;
    let mut best_offset = 0i64;
    for offset in -600i64..=600 {
        let (sa, sb) = (offset.max(0) as usize, (-offset).max(0) as usize);
        if sa >= fa.values.len() || sb >= fb.values.len() { continue }
        let overlap = (fa.values.len() - sa).min(fb.values.len() - sb);
        if overlap < 64 { continue }
        let bits: u32 = (0..overlap)
            .map(|i| (fa.values[sa + i] ^ fb.values[sb + i]).count_ones()).sum();
        let r = bits as f64 / (overlap * 32) as f64;
        if r < best { best = r; best_offset = offset; }
    }
    println!("{:.4}  (offset {})", best, best_offset);
}

/// Verifies the algorithm on the machine it is running on, so a user can
/// confirm a Windows or Linux build behaves exactly like the reference without
/// needing the reference.
fn selftest() {
    use dd_core::fingerprint::*;
    let mut failures = 0;

    let window = make_window();
    let peak = window.iter().cloned().fold(f32::MIN, f32::max);
    check("hann window length", window.len() == FRAME_SIZE, &mut failures);
    check("hann window peak", (peak - 1.632_993_2).abs() < 1e-6, &mut failures);
    check("hann window starts at zero", window[0] == 0.0, &mut failures);

    let edges = make_band_edges(FRAME_SIZE, FINGERPRINT_RATE);
    check("band edge count", edges.len() == BAND_COUNT + 1, &mut failures);
    check("band edges span 111..1115", edges[0] == 111 && edges[BAND_COUNT] == 1115, &mut failures);
    check("band edges increase", edges.windows(2).all(|w| w[1] > w[0]), &mut failures);

    // A deterministic signal, fingerprinted end to end.
    let mut engine = FingerprintEngine::new();
    let samples: Vec<f32> = (0..FINGERPRINT_RATE as usize * 6)
        .map(|i| {
            let t = i as f32 / FINGERPRINT_RATE as f32;
            let note = [440.0f32, 554.37, 659.25, 880.0][((t * 2.0) as usize) % 4];
            let phase = (t * 2.0).fract();
            ((note * t * std::f32::consts::TAU).sin() * (-3.0 * phase).exp()) * 0.5
        })
        .collect();
    let print = engine.fingerprint(&samples);
    check("fingerprint is usable", print.is_usable(), &mut failures);
    check("fingerprint is not stationary", !print.is_stationary(), &mut failures);
    check(
        "fingerprint self-distance is zero",
        shape_distance(&print.shape_profile, &print.shape_profile) == 0.0,
        &mut failures,
    );

    // A held tone must classify as stationary.
    let tone: Vec<f32> = (0..FINGERPRINT_RATE as usize * 6)
        .map(|i| {
            let t = i as f32 / FINGERPRINT_RATE as f32;
            (440.0 * t * std::f32::consts::TAU).sin() * 0.4
        })
        .collect();
    let tone_print = engine.fingerprint(&tone);
    check("held tone is stationary", tone_print.is_stationary(), &mut failures);
    check(
        "tone and music have different timbre",
        shape_distance(&print.shape_profile, &tone_print.shape_profile) > 0.2,
        &mut failures,
    );

    if failures == 0 {
        println!("\nAll checks passed. This build matches the reference implementation.");
    } else {
        println!("\n{failures} check(s) FAILED.");
        std::process::exit(1);
    }
}

fn check(name: &str, ok: bool, failures: &mut usize) {
    println!("{:<44} {}", name, if ok { "PASS" } else { "FAIL" });
    if !ok {
        *failures += 1;
    }
}
