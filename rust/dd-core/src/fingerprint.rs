//! Acoustic fingerprinting, ported from Sources/DupliDetect/Fingerprint.swift.
//!
//! Every constant here is chosen to reproduce the macOS implementation exactly,
//! not because it reads well. Three in particular are load-bearing and are
//! explained where they appear: the Hann window's normalisation, the absence of
//! a spectrum scale factor, and the fact that FFT bin 0 is never read.

use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

/// Rate the fingerprinter works at.
pub const FINGERPRINT_RATE: f64 = 11025.0;

pub const FRAME_SIZE: usize = 4096;
pub const HOP_SIZE: usize = 512;
pub const BAND_COUNT: usize = 33; // 33 bands -> 32 bits per frame
pub const LOW_FREQUENCY: f64 = 300.0;
pub const HIGH_FREQUENCY: f64 = 3000.0;
/// Roughly 1.5 seconds of audio; below this there is nothing to compare.
pub const MINIMUM_FRAMES: usize = 64;
/// Below this median spectral flux the audio is treated as stationary.
pub const STATIONARY_FLUX: f64 = 0.10;

/// Seconds of audio each sub-fingerprint advances.
pub fn seconds_per_frame() -> f64 {
    HOP_SIZE as f64 / FINGERPRINT_RATE
}

#[derive(Clone, Debug, PartialEq)]
pub struct Fingerprint {
    pub values: Vec<u32>,
    pub shape_profile: Vec<u8>,
    pub flux: f64,
}

impl Fingerprint {
    pub fn empty() -> Self {
        Fingerprint { values: Vec::new(), shape_profile: Vec::new(), flux: f64::MAX }
    }
    pub fn duration(&self) -> f64 {
        self.values.len() as f64 * seconds_per_frame()
    }
    pub fn is_usable(&self) -> bool {
        self.values.len() >= MINIMUM_FRAMES
    }
    pub fn is_stationary(&self) -> bool {
        self.flux < STATIONARY_FLUX
    }
}

#[derive(Clone, Debug)]
pub struct AudioAnalysis {
    pub fingerprint: Fingerprint,
    pub leading_silence: f64,
    pub trailing_silence: f64,
}

pub struct FingerprintEngine {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    band_edges: Vec<usize>,
    scratch: Vec<realfft::num_complex::Complex<f32>>,
    spectrum_out: Vec<realfft::num_complex::Complex<f32>>,
    magnitudes: Vec<f32>,
}

impl Default for FingerprintEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl FingerprintEngine {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(FRAME_SIZE);
        let scratch = fft.make_scratch_vec();
        let spectrum_out = fft.make_output_vec();
        FingerprintEngine {
            fft,
            window: make_window(),
            band_edges: make_band_edges(FRAME_SIZE, FINGERPRINT_RATE),
            scratch,
            spectrum_out,
            magnitudes: vec![0.0; FRAME_SIZE / 2],
        }
    }

    pub fn window(&self) -> &[f32] {
        &self.window
    }
    pub fn band_edges(&self) -> &[usize] {
        &self.band_edges
    }

    /// Fingerprint plus the silence trimmed from each end.
    pub fn analyze(&mut self, samples: &[f32], sample_rate: f64) -> AudioAnalysis {
        let range = content_range(samples);
        let leading = range.0 as f64 / sample_rate;
        let trailing = (samples.len() - range.1) as f64 / sample_rate;
        AudioAnalysis {
            fingerprint: self.fingerprint(samples),
            leading_silence: leading,
            trailing_silence: trailing,
        }
    }

    pub fn fingerprint(&mut self, samples: &[f32]) -> Fingerprint {
        let (start, end) = content_range(samples);
        let trimmed = &samples[start..end];
        if trimmed.len() < FRAME_SIZE + HOP_SIZE {
            return Fingerprint::empty();
        }

        let frame_count = (trimmed.len() - FRAME_SIZE) / HOP_SIZE + 1;
        let mut band_energies = [0.0f32; BAND_COUNT];
        let mut previous: Option<[f32; BAND_COUNT]> = None;
        let mut values: Vec<u32> = Vec::with_capacity(frame_count);
        let mut frame_flux: Vec<f32> = Vec::with_capacity(frame_count);
        let mut band_totals = [0.0f64; BAND_COUNT];
        let mut shape_frames = 0usize;
        let mut windowed = vec![0.0f32; FRAME_SIZE];

        for frame in 0..frame_count {
            let offset = frame * HOP_SIZE;
            for i in 0..FRAME_SIZE {
                windowed[i] = trimmed[offset + i] * self.window[i];
            }
            self.spectrum(&windowed, &mut band_energies);

            for m in 0..BAND_COUNT {
                band_totals[m] += band_energies[m] as f64;
            }
            shape_frames += 1;

            if let Some(prev) = previous {
                let mut bits: u32 = 0;
                let mut change: f32 = 0.0;
                for m in 0..32 {
                    let current = band_energies[m] - band_energies[m + 1];
                    let earlier = prev[m] - prev[m + 1];
                    if current - earlier > 0.0 {
                        bits |= 1 << (m as u32);
                    }
                }
                for m in 0..BAND_COUNT {
                    change += (band_energies[m] - prev[m]).abs();
                }
                values.push(bits);
                frame_flux.push(change / BAND_COUNT as f32);
            }
            previous = Some(band_energies);
        }

        Fingerprint {
            values,
            shape_profile: shape(&band_totals, shape_frames),
            flux: median(&frame_flux) as f64,
        }
    }

    /// Real FFT of one windowed frame, folded into log band energies.
    pub fn spectrum(&mut self, windowed: &[f32], bands: &mut [f32; BAND_COUNT]) {
        let mut input = windowed.to_vec();
        self.fft
            .process_with_scratch(&mut input, &mut self.spectrum_out, &mut self.scratch)
            .expect("fft");

        // Swift applies 0.25 here because vDSP_fft_zrip returns 2x the
        // mathematical DFT, so squaring gives 4x. realfft already produces the
        // unscaled mathematical DFT, so squaring is already correct and the
        // 0.25 must NOT be carried across.
        for k in 0..FRAME_SIZE / 2 {
            let c = self.spectrum_out[k];
            self.magnitudes[k] = c.re * c.re + c.im * c.im;
        }
        // vDSP packs DC and Nyquist together into bin 0, making magnitudes[0]
        // meaningless there. Harmless in both implementations: the lowest band
        // edge is clamped to 1 and 300 Hz lands at bin 111, so bin 0 is never
        // summed. Do not "fix" this.

        for band in 0..BAND_COUNT {
            let lower = self.band_edges[band];
            let upper = self.band_edges[band + 1].max(lower + 1);
            let mut sum = 0.0f32;
            for k in lower..upper {
                sum += self.magnitudes[k];
            }
            // Log energy keeps quiet passages as informative as loud ones.
            // The epsilon means this is NOT scale-invariant near the noise
            // floor, which is why the spectrum scale above has to be right.
            bands[band] = (sum + 1e-9).log10();
        }
    }
}

/// The Hann window vDSP produces for `vDSP_HANN_NORM`.
///
/// From the SDK header: `W = .816496580927726` (sqrt(2/3)) and
/// `C[n] = W * (1 - cos(2*pi*n/N))` — periodic (`/N`, not `/(N-1)`), giving a
/// peak of 1.633 rather than 1.0. A textbook `0.5 * (1 - cos(...))` would be
/// 0.612x in amplitude and 0.375x in power, which shifts every band energy.
///
/// One deliberate divergence: vDSP evaluates `1 - cos(x)` in f32, which loses
/// most of its significant digits to cancellation for small `x` — at n = 1 it
/// yields 1.192e-6 where the true value is 1.177e-6, a 1.3% error. This uses
/// the algebraically identical `2 sin²(x/2)`, which has no cancellation at all.
///
/// That is not merely more accurate, it is the only portable choice. Matching
/// vDSP bit for bit would mean reproducing its f32 cancellation, which
/// amplifies last-ulp differences between platform `cosf` implementations by
/// four orders of magnitude — Windows and Linux could then disagree with each
/// other about the window, and therefore about fingerprints. This form gives
/// every platform the same answer.
///
/// The divergence is immaterial: peak amplitude is bit-identical to vDSP, the
/// largest absolute difference anywhere is 2.4e-7 against a peak of 1.633
/// (1.5e-7 relative), and the only coefficients differing by more than 0.1%
/// are twelve whose values are below 7.8e-5 and so contribute nothing. Tier 0
/// asserts the band energies that result, and Tier 1 asserts the fingerprints.
pub fn make_window() -> Vec<f32> {
    const W: f64 = 0.816_496_580_927_726;
    let n = FRAME_SIZE as f64;
    (0..FRAME_SIZE)
        .map(|i| {
            let half = std::f64::consts::PI * i as f64 / n; // x/2 where x = 2*pi*i/N
            let sin_half = half.sin();
            (W * 2.0 * sin_half * sin_half) as f32
        })
        .collect()
}

/// Logarithmically spaced FFT bin boundaries covering 300 Hz - 3 kHz.
pub fn make_band_edges(frame_size: usize, sample_rate: f64) -> Vec<usize> {
    let bins_per_hz = frame_size as f64 / sample_rate;
    let ratio = (HIGH_FREQUENCY / LOW_FREQUENCY).ln();
    let mut edges: Vec<usize> = Vec::with_capacity(BAND_COUNT + 1);
    for i in 0..=BAND_COUNT {
        let frequency = LOW_FREQUENCY * (ratio * i as f64 / BAND_COUNT as f64).exp();
        // Swift's .rounded() is half-away-from-zero, matching f64::round().
        let mut bin = (frequency * bins_per_hz).round() as usize;
        bin = bin.clamp(1, frame_size / 2 - 1);
        if let Some(&last) = edges.last() {
            if bin <= last {
                bin = last + 1;
            }
        }
        edges.push(bin.min(frame_size / 2 - 1));
    }
    edges
}

/// The span of `samples` that actually carries audio.
pub fn content_range(samples: &[f32]) -> (usize, usize) {
    if samples.is_empty() {
        return (0, 0);
    }
    let peak = samples.iter().fold(0.0f32, |acc, v| acc.max(v.abs()));
    if peak <= 1e-5 {
        return (0, samples.len());
    }
    let threshold = peak * 0.005;

    let mut start = 0;
    while start < samples.len() && samples[start].abs() < threshold {
        start += 1;
    }
    let mut end = samples.len() - 1;
    while end > start && samples[end].abs() < threshold {
        end -= 1;
    }
    if end <= start {
        return (0, samples.len());
    }
    (start, end + 1)
}

/// Spectral-shape template: mean band energy relative to the loudest band,
/// clamped to an 80 dB window and quantised.
pub fn shape(totals: &[f64], frames: usize) -> Vec<u8> {
    if frames == 0 || totals.is_empty() {
        return Vec::new();
    }
    let means: Vec<f64> = totals.iter().map(|t| t / frames as f64).collect();
    let peak = means.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let window = 8.0;
    means
        .iter()
        .map(|mean| {
            let relative = (mean - peak).max(-window).min(0.0);
            (((relative + window) / window * 255.0).round()) as u8
        })
        .collect()
}

/// Weighted distance between two shape templates, 0 (identical) to 1.
pub fn shape_distance(a: &[u8], b: &[u8]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 1.0;
    }
    let mut weighted_difference = 0.0;
    let mut total_weight = 0.0;
    for i in 0..a.len() {
        let (va, vb) = (a[i] as f64, b[i] as f64);
        let weight = va.max(vb) / 255.0;
        weighted_difference += weight * (va - vb).abs();
        total_weight += weight;
    }
    if total_weight <= 0.0 {
        return 0.0;
    }
    weighted_difference / total_weight / 255.0
}

/// Upper median, matching Swift's `sorted[count / 2]`.
pub fn median(values: &[f32]) -> f32 {
    if values.is_empty() {
        return f32::MAX;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    sorted[sorted.len() / 2]
}
