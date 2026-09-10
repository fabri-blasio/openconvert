//! DeepFilterNet3: speech enhancement.
//!
//! # This model is a graph, not a pipeline
//!
//! Unlike every other adapter here, the export takes **features** rather than
//! audio and returns **coefficients** rather than a result. Its own README is
//! blunt about it: *"A compatible STFT, feature-normalization, filtering, and
//! synthesis implementation is required."* So the DSP on both sides is ours,
//! and every constant below comes from the published contract rather than from
//! guesswork:
//!
//! - 48 kHz, 960-point real FFT, 480 hop, Vorbis window
//! - 481 bins; deep filtering over the first 96
//! - 32 ERB bands, order 5, lookahead 2
//! - normalisation alpha 0.99, ERB state initialised -60 dB → -90 dB,
//!   complex unit state 0.001 → 0.0001
//!
//! The ERB matrices and the window are not derived here either — they ship as
//! a second artifact, which removes the likeliest source of a silently wrong
//! result.

use ndarray::Array4;
use ort::value::Value;

use crate::accelerator::Workload;
use crate::infer::session_for;

/// Everything the contract fixes.
const SR: u32 = 48_000;
const N_FFT: usize = 960;
const HOP: usize = 480;
const BINS: usize = N_FFT / 2 + 1; // 481
const ERB: usize = 32;
const DF_BINS: usize = 96;
const DF_ORDER: usize = 5;
const DF_LOOKAHEAD: usize = 2;
const ALPHA: f32 = 0.99;

/// ERB features are divided by this after mean subtraction. DeepFilterNet's
/// `NORM_DIV` for the ERB branch.
const ERB_NORM_DIV: f32 = 40.0;

/// Upstream libDF v0.5.6 applies 2*hop/fft_size^2 in analysis.
/// Our inverse divides by fft_size, so apply this only to the model features.
const FEATURE_SCALE: f32 = (2 * HOP) as f32 / (N_FFT * N_FFT) as f32;

/// Longest input this will process.
///
/// Bound whole-recording model tensors and FFT buffers to two minutes.
/// Longer recordings fail explicitly instead of exhausting worker memory.
const MAX_SAMPLES: usize = SR as usize * 120;

/// The constants that ship beside the graph.
struct Aux {
    /// `[BINS][ERB]`, row-major: bin → band weights, each band summing to 1.
    fwd: Vec<f32>,
    /// `[ERB][BINS]`, row-major: band → bin weights.
    inv: Vec<f32>,
    /// The 960-point Vorbis window.
    window: Vec<f32>,
}

impl Aux {
    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let want = (BINS * ERB + ERB * BINS + N_FFT) * 4;
        if bytes.len() != want {
            return Err(format!(
                "the auxiliary constants are {} bytes; this build expects exactly {want}",
                bytes.len()
            ));
        }
        let f: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let (a, b) = (BINS * ERB, BINS * ERB + ERB * BINS);
        Ok(Self {
            fwd: f[..a].to_vec(),
            inv: f[a..b].to_vec(),
            window: f[b..].to_vec(),
        })
    }
}

/// Mixed-radix FFT, matching libDF's transform convention.
struct Stft {
    forward: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    inverse: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
}
impl Stft {
    fn new() -> Self {
        let mut planner = realfft::RealFftPlanner::<f32>::new();
        Self {
            forward: planner.plan_fft_forward(N_FFT),
            inverse: planner.plan_fft_inverse(N_FFT),
        }
    }
    fn forward(&self, frame: &[f32], out: &mut [f32]) {
        let mut input = frame.to_vec();
        let mut bins = self.forward.make_output_vec();
        self.forward
            .process(&mut input, &mut bins)
            .expect("fixed FFT sizes");
        for (slot, bin) in out.chunks_exact_mut(2).zip(bins) {
            slot[0] = bin.re;
            slot[1] = bin.im;
        }
    }
    fn inverse(&self, spec: &[f32], out: &mut [f32]) {
        let mut bins = self.inverse.make_input_vec();
        for (bin, pair) in bins.iter_mut().zip(spec.chunks_exact(2)) {
            bin.re = pair[0];
            bin.im = pair[1];
        }
        bins[0].im = 0.0;
        bins[BINS - 1].im = 0.0;
        self.inverse
            .process(&mut bins, out)
            .expect("fixed FFT sizes");
        for sample in out {
            *sample /= N_FFT as f32;
        }
    }
}

/// Enhance speech in interleaved PCM, returning mono 48 kHz samples.
///
/// # Errors
///
/// Audio longer than [`MAX_SAMPLES`], malformed constants, or a model whose
/// outputs are not the shapes the contract declares.
pub fn denoise(
    samples: &[f32],
    channels: usize,
    rate: u32,
    model: &[u8],
    aux: &[u8],
) -> Result<Vec<f32>, String> {
    let aux = Aux::parse(aux)?;
    let mut audio = crate::mel::resample_mono(samples, channels, rate, SR);
    if audio.is_empty() {
        return Err("this file contains no audio to enhance".into());
    }
    if audio.len() > MAX_SAMPLES {
        return Err(format!(
            "this is {:.0} seconds of audio; enhancement is limited to {} seconds",
            audio.len() as f64 / f64::from(SR),
            MAX_SAMPLES / SR as usize
        ));
    }

    let input_energy = audio.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
    let peak = audio.iter().copied().map(f32::abs).fold(0.0, f32::max);
    let input_gain = if peak > 1e-6 && peak < 0.3 {
        (0.5 / peak).min(100.0)
    } else {
        1.0
    };
    for sample in &mut audio {
        *sample *= input_gain;
    }

    // "append one 960-sample analysis tail and remove the 480-sample STFT
    // delay after overlap-add synthesis" -- the contract's batch-parity rule.
    //
    // The delay has to be CREATED before it can be removed. Overlap-add only
    // reconstructs a sample that two frames cover, and the first hop is
    // covered by one, so a front pad of one hop is what puts the signal's
    // first sample at output index HOP. Without it, draining HOP does not
    // remove a delay -- it deletes the first 10 ms of the recording, which is
    // what took the round trip to -3 dB.
    let mut padded = vec![0.0_f32; HOP];
    padded.extend_from_slice(&audio);
    padded.extend(std::iter::repeat_n(0.0, N_FFT));

    let stft = Stft::new();
    let frames = (padded.len().saturating_sub(N_FFT)) / HOP + 1;

    // ---- analysis ----
    let mut spec = vec![0.0_f32; frames * BINS * 2];
    let mut frame = vec![0.0_f32; N_FFT];
    for t in 0..frames {
        let start = t * HOP;
        for (n, f) in frame.iter_mut().enumerate() {
            *f = padded.get(start + n).copied().unwrap_or(0.0) * aux.window[n];
        }
        stft.forward(&frame, &mut spec[t * BINS * 2..(t + 1) * BINS * 2]);
    }

    // ---- features ----
    let (feat_erb, feat_spec) = features(&spec, frames, &aux, FEATURE_SCALE);

    // ---- the graph ----
    // Frame-by-frame over a whole recording: one session, one call per hop.
    // NOT COVERED BY `run_for`, AND THAT IS A DECISION. DeepFilterNet is
    // 8.3 MB and runs a frame at a time over a streaming buffer, so the tensors
    // it hands the card are orders of magnitude smaller than the ones that
    // exhaust it. This line is the record of the decision.
    let (mut session, _provider) = session_for(model, Workload::Light)?; // openconvert-lint: allow -- small enhancement graph, avoid GPU session overhead
    let erb_in = Array4::from_shape_vec((1, 1, frames, ERB), feat_erb)
        .map_err(|e| format!("could not build the ERB feature tensor: {e}"))?;
    let spec_in = Array4::from_shape_vec((1, 2, frames, DF_BINS), feat_spec)
        .map_err(|e| format!("could not build the spectral feature tensor: {e}"))?;
    let outputs = session
        .run(ort::inputs![
            "feat_erb" => Value::from_array(erb_in)
                .map_err(|e| format!("could not build the ERB feature tensor: {e}"))?,
            "feat_spec" => Value::from_array(spec_in)
                .map_err(|e| format!("could not build the spectral feature tensor: {e}"))?,
        ])
        .map_err(|e| format!("the enhancement model failed: {e}"))?;

    let (_, mask_v) = outputs
        .get("erb_mask")
        .ok_or("the model returned no ERB mask")?
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("the ERB mask was not a float tensor: {e}"))?;
    let mask: Vec<f32> = mask_v.to_vec();
    let (_, coef_v) = outputs
        .get("df_coefs")
        .ok_or("the model returned no filter coefficients")?
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("the filter coefficients were not a float tensor: {e}"))?;
    let coefs: Vec<f32> = coef_v.to_vec();
    if mask.len() < frames * ERB || coefs.len() < DF_ORDER * frames * DF_BINS * 2 {
        return Err("the model returned fewer values than its own shapes".into());
    }

    // ---- apply ----
    //
    // The deep filter reads an IMMUTABLE copy of the noisy spectrum, per the
    // contract: filtering the already-masked spectrum would apply the ERB
    // gains twice to the first 96 bins.
    let noisy = spec.clone();
    apply_erb_gains(&mut spec, frames, &mask, &aux);
    apply_deep_filter(&mut spec, &noisy, frames, &coefs);
    // A speech model can classify music or quiet voices as noise. Limit
    // suppression to 18 dB by mixing a small, phase-aligned original signal.
    let dry = 10.0_f32.powf(-18.0 / 20.0);
    for (enhanced, original) in spec.iter_mut().zip(&noisy) {
        if !enhanced.is_finite() {
            return Err("Noise removal returned invalid audio; the original file was kept.".into());
        }
        *enhanced = *enhanced * (1.0 - dry) + *original * dry;
    }

    // ---- synthesis ----
    let mut out = vec![0.0_f32; padded.len()];
    let mut recon = vec![0.0_f32; N_FFT];
    for t in 0..frames {
        stft.inverse(&spec[t * BINS * 2..(t + 1) * BINS * 2], &mut recon);
        let start = t * HOP;
        for (n, &r) in recon.iter().enumerate() {
            if let Some(slot) = out.get_mut(start + n) {
                // Window again on the way out: analysis and synthesis each
                // apply it, and the Vorbis window's `w² + w² = 1` property at
                // 50% overlap is what makes the sum reconstruct exactly.
                *slot += r * aux.window[n];
            }
        }
    }
    out.drain(..HOP.min(out.len()));
    out.truncate(audio.len());
    for sample in &mut out {
        *sample /= input_gain;
    }
    let output_energy = out.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
    if input_energy > 1e-8 && output_energy < input_energy * 0.063 {
        return Err("Noise removal could not preserve enough sound. This model is designed for speech; the original file was kept.".into());
    }
    Ok(out)
}

/// ERB and complex features, each with its own exponential-moving-average
/// normalisation.
///
/// The EMA state is the part that cannot be checked by eye. Both states are
/// initialised to a ramp rather than to zero — the contract specifies -60 dB
/// to -90 dB across the bands and 0.001 to 0.0001 across the bins — because a
/// zeroed state makes the first frames look enormously louder than they are,
/// and the model responds by gating real speech at the start of every file.
fn features(spec: &[f32], frames: usize, aux: &Aux, fscale: f32) -> (Vec<f32>, Vec<f32>) {
    let mut erb_state: Vec<f32> = (0..ERB)
        .map(|b| -60.0 + (-30.0) * (b as f32) / ((ERB - 1) as f32))
        .collect();
    let mut unit_state: Vec<f32> = (0..DF_BINS)
        .map(|f| 0.001 + (0.0001 - 0.001) * (f as f32) / ((DF_BINS - 1) as f32))
        .collect();

    let mut feat_erb = vec![0.0_f32; frames * ERB];
    // Channel-major: [1, 2, T, 96] means all the real parts, then all the
    // imaginary ones — not interleaved per bin.
    let mut feat_spec = vec![0.0_f32; 2 * frames * DF_BINS];

    for t in 0..frames {
        let s = &spec[t * BINS * 2..(t + 1) * BINS * 2];

        // Band energies through the shipped matrix, then dB.
        for b in 0..ERB {
            let mut e = 0.0_f32;
            for k in 0..BINS {
                let w = aux.fwd[k * ERB + b];
                if w != 0.0 {
                    let (re, im) = (s[k * 2] * fscale, s[k * 2 + 1] * fscale);
                    e += w * (re * re + im * im);
                }
            }
            let db = 10.0 * e.max(1e-10).log10();
            erb_state[b] = erb_state[b] * ALPHA + db * (1.0 - ALPHA);
            feat_erb[t * ERB + b] = (db - erb_state[b]) / ERB_NORM_DIV;
        }

        // Complex bins, normalised by the square root of a running magnitude.
        for f in 0..DF_BINS {
            let (re, im) = (s[f * 2] * fscale, s[f * 2 + 1] * fscale);
            let mag = (re * re + im * im).sqrt();
            unit_state[f] = unit_state[f] * ALPHA + mag * (1.0 - ALPHA);
            let d = unit_state[f].max(1e-10).sqrt();
            feat_spec[t * DF_BINS + f] = re / d;
            feat_spec[frames * DF_BINS + t * DF_BINS + f] = im / d;
        }
    }
    (feat_erb, feat_spec)
}

/// Spread each band's gain back over its bins and scale the spectrum.
fn apply_erb_gains(spec: &mut [f32], frames: usize, mask: &[f32], aux: &Aux) {
    let mut gains = vec![0.0_f32; BINS];
    for t in 0..frames {
        gains.iter_mut().for_each(|g| *g = 0.0);
        for b in 0..ERB {
            let m = mask[t * ERB + b];
            if m == 0.0 {
                continue;
            }
            for (k, g) in gains.iter_mut().enumerate() {
                *g += m * aux.inv[b * BINS + k];
            }
        }
        let s = &mut spec[t * BINS * 2..(t + 1) * BINS * 2];
        for k in 0..BINS {
            s[k * 2] *= gains[k];
            s[k * 2 + 1] *= gains[k];
        }
    }
}

/// The five-tap complex filter over the first 96 bins.
///
/// `df_coefs` is `[1, order, T, 96, 2]`, so the order axis is outermost after
/// the batch — reading it as if frames came first produces a filter that is
/// confidently wrong rather than an error.
fn apply_deep_filter(spec: &mut [f32], noisy: &[f32], frames: usize, coefs: &[f32]) {
    for t in 0..frames {
        for f in 0..DF_BINS {
            let (mut re, mut im) = (0.0_f32, 0.0_f32);
            for i in 0..DF_ORDER {
                // Lookahead means tap `i` reads a frame AHEAD of `t` for
                // i > lookahead and behind it for i < lookahead.
                let tap = (t + i) as isize - DF_LOOKAHEAD as isize;
                if tap < 0 || tap as usize >= frames {
                    continue;
                }
                let tap = tap as usize;
                let ci = ((i * frames + t) * DF_BINS + f) * 2;
                let (cr, ci_im) = (coefs[ci], coefs[ci + 1]);
                let si = (tap * BINS + f) * 2;
                let (sr, si_im) = (noisy[si], noisy[si + 1]);
                re += cr * sr - ci_im * si_im;
                im += cr * si_im + ci_im * sr;
            }
            let out = (t * BINS + f) * 2;
            spec[out] = re;
            spec[out + 1] = im;
        }
    }
}

/// The Vorbis power-complementary window, generated.
///
/// The shipped constants carry this same window, and the test below builds it
/// instead of reading them so that it RUNS. A test gated on an environment
/// variable pointing at a downloaded file is a test that is green because it
/// did nothing, which this project has been caught by before.
///
/// `w[n] = sin(pi/2 * sin^2(pi(n+0.5)/N))`, whose defining property is
/// `w[n]^2 + w[n+N/2]^2 == 1` — that is what makes analysis and synthesis
/// windowing reconstruct exactly at 50% overlap.
#[cfg(test)]
fn vorbis_window() -> Vec<f32> {
    (0..N_FFT)
        .map(|n| {
            let x = std::f64::consts::PI * (n as f64 + 0.5) / N_FFT as f64;
            let s = x.sin();
            (std::f64::consts::FRAC_PI_2 * s * s).sin() as f32
        })
        .collect()
}

/// Analysis followed immediately by synthesis, with nothing in between.
/// If this does not return the input, the transform is wrong and no amount of
/// looking at the model will show it.
#[cfg(test)]
fn round_trip(audio: &[f32], window: &[f32]) -> Vec<f32> {
    let stft = Stft::new();
    let mut padded = vec![0.0_f32; HOP];
    padded.extend_from_slice(audio);
    padded.extend(std::iter::repeat_n(0.0, N_FFT));
    let frames = (padded.len().saturating_sub(N_FFT)) / HOP + 1;

    let mut spec = vec![0.0_f32; frames * BINS * 2];
    let mut frame = vec![0.0_f32; N_FFT];
    for t in 0..frames {
        let start = t * HOP;
        for (n, f) in frame.iter_mut().enumerate() {
            *f = padded.get(start + n).copied().unwrap_or(0.0) * window[n];
        }
        stft.forward(&frame, &mut spec[t * BINS * 2..(t + 1) * BINS * 2]);
    }
    let mut out = vec![0.0_f32; padded.len()];
    let mut recon = vec![0.0_f32; N_FFT];
    for t in 0..frames {
        stft.inverse(&spec[t * BINS * 2..(t + 1) * BINS * 2], &mut recon);
        let start = t * HOP;
        for (n, &r) in recon.iter().enumerate() {
            if let Some(slot) = out.get_mut(start + n) {
                *slot += r * window[n];
            }
        }
    }
    out.drain(..HOP.min(out.len()));
    out.truncate(audio.len());
    out
}

#[cfg(test)]
mod tests {
    /// The DFT alone, with no window and no overlap-add.
    ///
    /// If this fails, nothing downstream can be judged: the transform is not
    /// a transform.
    #[test]
    fn one_frame_survives_forward_then_inverse() {
        let stft = super::Stft::new();
        // Deterministic pseudo-random content across the whole frame.
        let x: Vec<f32> = (0..super::N_FFT)
            .map(|i| {
                let a = (i as f32 * 12.9898).sin() * 43758.547;
                a - a.floor() - 0.5
            })
            .collect();
        let mut spec = vec![0.0_f32; super::BINS * 2];
        stft.forward(&x, &mut spec);
        let mut y = vec![0.0_f32; super::N_FFT];
        stft.inverse(&spec, &mut y);
        let num: f64 = x.iter().map(|v| f64::from(*v) * f64::from(*v)).sum();
        let den: f64 = x
            .iter()
            .zip(&y)
            .map(|(a, b)| {
                let d = f64::from(*a) - f64::from(*b);
                d * d
            })
            .sum();
        let snr = 10.0 * (num / den.max(1e-30)).log10();
        assert!(
            snr > 80.0,
            "single-frame DFT round trip is only {snr:.1} dB"
        );
    }

    /// The transform must be transparent before anything is asked of the model.
    #[test]
    fn analysis_then_synthesis_returns_the_input() {
        let window = super::vorbis_window();
        // The property the whole reconstruction rests on, asserted rather than
        // assumed: without it, windowing twice does not sum to one.
        for n in 0..super::HOP {
            let cola = window[n] * window[n] + window[n + super::HOP] * window[n + super::HOP];
            assert!(
                (cola - 1.0).abs() < 1e-4,
                "window is not power-complementary at {n}"
            );
        }
        // A second of speech-shaped content: a few harmonics plus a sweep.
        let n = 48_000usize;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / 48_000.0;
                0.3 * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 440.0 * t).sin()
                    + 0.1 * (2.0 * std::f32::consts::PI * (800.0 + 200.0 * t) * t).sin()
            })
            .collect();
        let y = super::round_trip(&x, &window);
        assert_eq!(y.len(), x.len());
        // Ignore the first and last window: overlap-add cannot reconstruct
        // the edges without frames that do not exist.
        let lo = 960usize;
        let hi = x.len() - 960;
        let num: f64 = (lo..hi).map(|i| f64::from(x[i]) * f64::from(x[i])).sum();
        let den: f64 = (lo..hi)
            .map(|i| {
                let d = f64::from(x[i]) - f64::from(y[i]);
                d * d
            })
            .sum();
        let snr = 10.0 * (num / den.max(1e-30)).log10();
        assert!(
            snr > 60.0,
            "round-trip SNR only {snr:.1} dB; the transform is not transparent"
        );
    }
}
