//! Tier 0: the primitives must match the macOS implementation exactly.
//! Golden values come from `Tools/ParityDump` run against the real Swift code.

use dd_core::fingerprint::*;
use serde_json::Value;

fn golden() -> Value {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../testdata/golden/primitives.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("golden primitives.json")).unwrap()
}

fn f32_from_bits(s: &str) -> f32 {
    f32::from_bits(s.parse::<u32>().unwrap())
}
fn f64_from_bits(s: &str) -> f64 {
    f64::from_bits(s.parse::<u64>().unwrap())
}

#[test]
fn constants_match() {
    let g = golden();
    assert_eq!(g["frameSize"].as_u64().unwrap() as usize, FRAME_SIZE);
    assert_eq!(g["hopSize"].as_u64().unwrap() as usize, HOP_SIZE);
    assert_eq!(g["bandCount"].as_u64().unwrap() as usize, BAND_COUNT);
    assert_eq!(g["minimumFrames"].as_u64().unwrap() as usize, MINIMUM_FRAMES);
    assert_eq!(g["lowFrequency"].as_f64().unwrap(), LOW_FREQUENCY);
    assert_eq!(g["highFrequency"].as_f64().unwrap(), HIGH_FREQUENCY);
    assert_eq!(g["stationaryFlux"].as_f64().unwrap(), STATIONARY_FLUX);
    assert_eq!(g["fingerprintRate"].as_f64().unwrap(), FINGERPRINT_RATE);
}

/// vDSP's f32 `1 - cos(x)` cancellation cannot — and should not — be
/// reproduced (see `make_window`). What must hold is that the peak is exact and
/// that no coefficient differs enough to move a band energy.
#[test]
fn hann_window_matches_within_tolerance() {
    let g = golden();
    let expected: Vec<f32> = g["windowBits"].as_array().unwrap().iter()
        .map(|v| f32_from_bits(v.as_str().unwrap())).collect();
    let actual = make_window();
    assert_eq!(expected.len(), actual.len());
    let peak_expected = expected.iter().cloned().fold(f32::MIN, f32::max);
    let peak_actual = actual.iter().cloned().fold(f32::MIN, f32::max);
    assert_eq!(peak_actual.to_bits(), peak_expected.to_bits(), "window peak must be exact");

    let worst = (0..actual.len())
        .map(|i| (actual[i] - expected[i]).abs())
        .fold(0.0f32, f32::max);
    assert!(
        worst / peak_expected < 1e-6,
        "worst window difference {:e} is {:e} of the peak",
        worst, worst / peak_expected
    );
    eprintln!("  window: peak exact, worst absolute difference {:e} ({:e} of peak)",
              worst, worst / peak_expected);
}

#[test]
fn band_edges_match_exactly() {
    let g = golden();
    let expected: Vec<usize> = g["bandEdges"].as_array().unwrap().iter()
        .map(|v| v.as_u64().unwrap() as usize).collect();
    assert_eq!(make_band_edges(FRAME_SIZE, FINGERPRINT_RATE), expected);
}

#[test]
fn band_energies_match_within_tolerance() {
    let g = golden();
    let mut engine = FingerprintEngine::new();
    for spec in g["spectra"].as_array().unwrap() {
        let name = spec["name"].as_str().unwrap();
        let raw: Vec<f32> = spec["inputBits"].as_array().unwrap().iter()
            .map(|v| f32_from_bits(v.as_str().unwrap())).collect();
        let expected: Vec<f32> = spec["bandEnergyBits"].as_array().unwrap().iter()
            .map(|v| f32_from_bits(v.as_str().unwrap())).collect();

        let window = make_window();
        let windowed: Vec<f32> = raw.iter().zip(&window).map(|(a, b)| a * b).collect();
        let mut bands = [0.0f32; BAND_COUNT];
        engine.spectrum(&windowed, &mut bands);

        let mut worst = 0.0f64;
        for m in 0..BAND_COUNT {
            let diff = (bands[m] - expected[m]).abs() as f64;
            let rel = diff / expected[m].abs().max(1e-12) as f64;
            worst = worst.max(rel);
        }
        // A different FFT algorithm rounds differently; 1e-6 relative is the
        // budget the plan set for this.
        assert!(worst < 1e-6, "{}: worst relative band-energy error {:e}", name, worst);
        eprintln!("  spectrum {}: worst relative error {:e}", name, worst);
    }
}

#[test]
fn content_range_matches() {
    let g = golden();
    for case in g["contentRanges"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let count = case["count"].as_u64().unwrap() as usize;
        let samples: Vec<f32> = match name {
            "allZero" => vec![0.0; count],
            "dc" => vec![0.5; count],
            "padded" => (0..count).map(|i| if (300..700).contains(&i) { 0.8 } else { 0.0 }).collect(),
            "ramp" => (0..count).map(|i| i as f32 / 1000.0).collect(),
            other => panic!("unknown case {other}"),
        };
        let (lower, upper) = content_range(&samples);
        assert_eq!(lower, case["lower"].as_u64().unwrap() as usize, "{name} lower");
        assert_eq!(upper, case["upper"].as_u64().unwrap() as usize, "{name} upper");
    }
}

#[test]
fn shape_and_distance_match() {
    let g = golden();
    let totals_a: Vec<f64> = g["shapeTotalsABits"].as_array().unwrap().iter()
        .map(|v| f64_from_bits(v.as_str().unwrap())).collect();
    let totals_b: Vec<f64> = g["shapeTotalsBBits"].as_array().unwrap().iter()
        .map(|v| f64_from_bits(v.as_str().unwrap())).collect();
    let expected_a: Vec<u8> = g["shapeA"].as_array().unwrap().iter()
        .map(|v| v.as_u64().unwrap() as u8).collect();
    let expected_b: Vec<u8> = g["shapeB"].as_array().unwrap().iter()
        .map(|v| v.as_u64().unwrap() as u8).collect();

    let a = shape(&totals_a, 1);
    let b = shape(&totals_b, 1);
    assert_eq!(a, expected_a, "shape A");
    assert_eq!(b, expected_b, "shape B");

    let expected_distance = f64_from_bits(g["shapeDistanceBits"].as_str().unwrap());
    let actual = shape_distance(&a, &b);
    assert!((actual - expected_distance).abs() < 1e-12,
            "shapeDistance {} vs {}", actual, expected_distance);
}

#[test]
fn median_matches() {
    let g = golden();
    assert_eq!(median(&[3.0, 1.0, 2.0]).to_bits(),
               f32_from_bits(g["medianOdd"].as_str().unwrap()).to_bits());
    assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]).to_bits(),
               f32_from_bits(g["medianEven"].as_str().unwrap()).to_bits());
}
