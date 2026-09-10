//! Whisper's front end: audio in, log-mel spectrogram out.
//!
//! Every constant here is fixed by the model, not chosen. Whisper was trained
//! on 30-second windows of 16 kHz mono audio turned into 80 mel bands over
//! 3000 frames, and it will accept a differently-shaped tensor without
//! complaint while producing nonsense — so these are the numbers, and the
//! reason each is what it is sits beside it.

/// Whisper's sample rate. Not a preference: the positional embeddings assume
/// one frame is 10 ms at this rate.
pub const SAMPLE_RATE: u32 = 16_000;
/// 30 seconds. The encoder's input length is fixed, so shorter audio is padded
/// with silence and longer audio is processed one window at a time.
pub const WINDOW_SAMPLES: usize = 30 * SAMPLE_RATE as usize;
/// FFT size — 25 ms.
const N_FFT: usize = 400;
/// Hop — 10 ms, which is what makes 3000 frames out of 30 seconds.
const HOP: usize = 160;
/// Mel bands.
pub const N_MELS: usize = 80;
/// Frames the encoder expects.
pub const N_FRAMES: usize = WINDOW_SAMPLES / HOP;
/// Real spectrum bins from an `N_FFT` transform.
const N_BINS: usize = N_FFT / 2 + 1;

/// The log-mel spectrogram of one 30-second window, row-major `[N_MELS][N_FRAMES]`.
///
/// # Why the DFT here is direct rather than a fast transform
///
/// `N_FFT` is 400 — not a power of two — so radix-2 does not apply, and
/// zero-padding to 512 would move every frequency bin and invalidate the mel
/// filterbank built for 201 of them. A mixed-radix transform would be correct
/// and is a great deal of code to get subtly right.
///
/// So this evaluates the 201 needed bins directly against a precomputed
/// twiddle table. It is O(N_FFT · N_BINS) per frame instead of O(N log N), and
/// for the fixed 3000 frames of one window that is a bounded, one-off cost
/// measured in the low seconds — against a model run that costs more. The
/// table is built once per call and shared by every frame.
#[must_use]
pub fn log_mel(samples: &[f32]) -> Vec<f32> {
    let filters = mel_filterbank();
    let window = hann(N_FFT);

    // `center = true`: the signal is reflect-padded by half a window so frame
    // `t` is centred on sample `t * HOP`. Whisper's frame count depends on it.
    let padded = reflect_pad(samples, N_FFT / 2);

    // cos/sin for every (bin, sample) pair, built once.
    let mut cos_t = vec![0.0_f32; N_BINS * N_FFT];
    let mut sin_t = vec![0.0_f32; N_BINS * N_FFT];
    for k in 0..N_BINS {
        for n in 0..N_FFT {
            let a = -2.0 * std::f64::consts::PI * (k as f64) * (n as f64) / (N_FFT as f64);
            cos_t[k * N_FFT + n] = a.cos() as f32;
            sin_t[k * N_FFT + n] = a.sin() as f32;
        }
    }

    let mut mel = vec![0.0_f32; N_MELS * N_FRAMES];
    let mut power = vec![0.0_f32; N_BINS];
    let mut frame = vec![0.0_f32; N_FFT];

    for t in 0..N_FRAMES {
        let start = t * HOP;
        for n in 0..N_FFT {
            frame[n] = padded.get(start + n).copied().unwrap_or(0.0) * window[n];
        }
        for (k, pw) in power.iter_mut().enumerate() {
            let (mut re, mut im) = (0.0_f32, 0.0_f32);
            let base = k * N_FFT;
            for n in 0..N_FFT {
                re += frame[n] * cos_t[base + n];
                im += frame[n] * sin_t[base + n];
            }
            // Magnitude SQUARED. Whisper's reference takes `abs()**2`, and
            // using magnitude instead halves everything in the log domain.
            *pw = re * re + im * im;
        }
        for (m, row) in filters.chunks_exact(N_BINS).enumerate() {
            let mut acc = 0.0_f32;
            for k in 0..N_BINS {
                acc += row[k] * power[k];
            }
            mel[m * N_FRAMES + t] = acc;
        }
    }

    // log10, floored, then clamped to 8 decades below the window's own peak
    // and rescaled to roughly [-1, 1]. The dynamic-range clamp is per-window
    // and is what keeps a quiet recording from being scaled up into noise.
    let mut peak = f32::MIN;
    for v in &mut mel {
        *v = v.max(1e-10).log10();
        peak = peak.max(*v);
    }
    let floor = peak - 8.0;
    for v in &mut mel {
        *v = (v.max(floor) + 4.0) / 4.0;
    }
    mel
}

/// Periodic Hann, which is what `torch.hann_window` gives by default.
///
/// The symmetric variant divides by `n - 1` and is the right window for
/// filter design and the wrong one for an STFT.
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let x = std::f64::consts::PI * 2.0 * (i as f64) / (n as f64);
            (0.5 - 0.5 * x.cos()) as f32
        })
        .collect()
}

/// Mirror the signal at both ends, excluding the edge sample itself.
fn reflect_pad(x: &[f32], pad: usize) -> Vec<f32> {
    if x.is_empty() {
        return vec![0.0; pad * 2];
    }
    let mut out = Vec::with_capacity(x.len() + pad * 2);
    for i in (1..=pad).rev() {
        out.push(x[i.min(x.len() - 1)]);
    }
    out.extend_from_slice(x);
    for i in 1..=pad {
        let idx = x.len().saturating_sub(1 + i);
        out.push(x[idx]);
    }
    out
}

/// Slaney-style mel filterbank, `[N_MELS][N_BINS]` row-major.
///
/// Whisper ships this as a precomputed array produced by
/// `librosa.filters.mel(sr=16000, n_fft=400, n_mels=80)`, whose defaults are
/// the SLANEY scale and Slaney area normalisation — not HTK. The two mel
/// scales differ by a few percent, which is enough to shift every band and
/// degrade transcription without ever failing.
fn mel_filterbank() -> Vec<f32> {
    // Slaney: linear below 1 kHz, logarithmic above.
    const F_SP: f64 = 200.0 / 3.0;
    const MIN_LOG_HZ: f64 = 1000.0;
    const MIN_LOG_MEL: f64 = MIN_LOG_HZ / F_SP;
    let logstep = (6.4_f64).ln() / 27.0;

    let hz_to_mel = |hz: f64| {
        if hz < MIN_LOG_HZ {
            hz / F_SP
        } else {
            MIN_LOG_MEL + (hz / MIN_LOG_HZ).ln() / logstep
        }
    };
    let mel_to_hz = |mel: f64| {
        if mel < MIN_LOG_MEL {
            mel * F_SP
        } else {
            MIN_LOG_HZ * ((mel - MIN_LOG_MEL) * logstep).exp()
        }
    };

    let f_max = f64::from(SAMPLE_RATE) / 2.0;
    let (m_min, m_max) = (hz_to_mel(0.0), hz_to_mel(f_max));
    // N_MELS + 2 edges: each filter spans three consecutive ones.
    let edges: Vec<f64> = (0..N_MELS + 2)
        .map(|i| mel_to_hz(m_min + (m_max - m_min) * (i as f64) / ((N_MELS + 1) as f64)))
        .collect();
    let bin_hz: Vec<f64> = (0..N_BINS)
        .map(|k| f64::from(SAMPLE_RATE) * (k as f64) / (N_FFT as f64))
        .collect();

    let mut fb = vec![0.0_f32; N_MELS * N_BINS];
    for m in 0..N_MELS {
        let (lo, ctr, hi) = (edges[m], edges[m + 1], edges[m + 2]);
        // Slaney normalisation: equal AREA per filter, not equal peak. Without
        // it the high bands dominate, because they are wider.
        let enorm = 2.0 / (hi - lo);
        for k in 0..N_BINS {
            let f = bin_hz[k];
            let w = if f >= lo && f <= ctr && ctr > lo {
                (f - lo) / (ctr - lo)
            } else if f > ctr && f <= hi && hi > ctr {
                (hi - f) / (hi - ctr)
            } else {
                0.0
            };
            fb[m * N_BINS + k] = (w * enorm) as f32;
        }
    }
    fb
}

/// Resample to [`SAMPLE_RATE`] and mix down to mono.
#[must_use]
pub fn to_mono_16k(interleaved: &[f32], channels: usize, rate: u32) -> Vec<f32> {
    resample_mono(interleaved, channels, rate, SAMPLE_RATE)
}

/// Downmix to mono and resample to `target`.
///
/// Split out from [`to_mono_16k`] when a second model wanted 48 kHz: the
/// arithmetic is the same and two copies of it would drift.
///
/// Linear interpolation, stated plainly rather than dressed up: a
/// windowed-sinc resampler would be more faithful, and for speech into models
/// trained on the destination rate this is adequate. Pretending otherwise
/// would be the dishonest part.
#[must_use]
pub fn resample_mono(interleaved: &[f32], channels: usize, rate: u32, target: u32) -> Vec<f32> {
    if channels == 0 || interleaved.is_empty() {
        return Vec::new();
    }
    let frames = interleaved.len() / channels;
    let mono: Vec<f32> = (0..frames)
        .map(|i| {
            let sum: f32 = (0..channels).map(|c| interleaved[i * channels + c]).sum();
            sum / channels as f32
        })
        .collect();
    if rate == target || mono.is_empty() {
        return mono;
    }
    let ratio = f64::from(target) / f64::from(rate);
    let out_len = ((mono.len() as f64) * ratio).round() as usize;
    (0..out_len)
        .map(|i| {
            let src = (i as f64) / ratio;
            let i0 = src.floor() as usize;
            let frac = (src - src.floor()) as f32;
            let a = mono.get(i0).copied().unwrap_or(0.0);
            let b = mono.get(i0 + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

/// Downmix and resample a stream, one decoded packet at a time.
///
/// **The whole-file version is what made long recordings fail.**
/// `decode_audio` collected every sample as interleaved `f32` before anything
/// was resampled: an hour of 48 kHz stereo is 1.38 GB of intermediate before
/// the 230 MB the model actually needs, and the memory limit refused it.
///
/// Feeding packets through as they arrive means only the 16 kHz mono result is
/// ever held. Same arithmetic as [`resample_mono`] — linear interpolation, and
/// no claim to be more than that — but the source position carries across
/// packet boundaries, which is the part a naive per-packet call gets wrong:
/// resampling each packet independently rounds its length separately and
/// inserts a discontinuity at every seam.
pub struct StreamResampler {
    target: u32,
    /// Source rate, learned from the first packet.
    rate: u32,
    /// The last source sample of the previous packet, for interpolating across
    /// the seam.
    carry: Option<f32>,
    /// How many source samples have been consumed, as a whole number.
    consumed: u64,
    /// Position in the source, in samples, of the next output sample.
    next_src: f64,
    out: Vec<f32>,
}

impl StreamResampler {
    /// A resampler that produces `target` Hz mono.
    #[must_use]
    pub fn new(target: u32) -> Self {
        Self {
            target,
            rate: 0,
            carry: None,
            consumed: 0,
            next_src: 0.0,
            out: Vec::new(),
        }
    }

    /// Take one packet of interleaved samples.
    pub fn push(&mut self, interleaved: &[f32], channels: usize, rate: u32) {
        if channels == 0 || interleaved.is_empty() {
            return;
        }
        self.rate = rate;

        let frames = interleaved.len() / channels;
        let mono = |i: usize| -> f32 {
            let sum: f32 = (0..channels).map(|c| interleaved[i * channels + c]).sum();
            sum / channels as f32
        };

        if rate == self.target {
            self.out.extend((0..frames).map(mono));
            self.consumed += frames as u64;
            return;
        }

        let ratio = f64::from(self.target) / f64::from(rate);
        let base = self.consumed;
        // `carry` is the sample at index `base - 1`, so a source position
        // landing between packets still has both neighbours.
        let at = |i: i64| -> f32 {
            if i < 0 {
                return 0.0;
            }
            let i = i as u64;
            if i + 1 == base {
                return self.carry.unwrap_or(0.0);
            }
            let local = i.saturating_sub(base);
            if local < frames as u64 {
                mono(local as usize)
            } else {
                0.0
            }
        };

        // Emit every output sample whose source position has fully arrived.
        // The last source sample of this packet is held back, because the
        // sample after it is needed to interpolate and has not been decoded.
        let available = (base + frames as u64).saturating_sub(1) as f64;
        while self.next_src <= available {
            let i0 = self.next_src.floor();
            let frac = (self.next_src - i0) as f32;
            #[allow(clippy::cast_possible_truncation)]
            let i0 = i0 as i64;
            let a = at(i0);
            let b = at(i0 + 1);
            self.out.push(a + (b - a) * frac);
            self.next_src += 1.0 / ratio;
        }

        self.carry = (frames > 0).then(|| mono(frames - 1));
        self.consumed += frames as u64;
    }

    /// The resampled mono signal.
    #[must_use]
    pub fn finish(self) -> Vec<f32> {
        self.out
    }

    /// How many output samples are held so far, for a memory bound.
    #[must_use]
    pub fn len(&self) -> usize {
        self.out.len()
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;

    /// A repeatable signal with structure, so a seam artefact shows up as a
    /// difference rather than hiding in noise.
    fn tone(frames: usize, channels: usize) -> Vec<f32> {
        (0..frames * channels)
            .map(|i| {
                let f = (i / channels) as f32;
                (f * 0.031).sin() * 0.7 + (f * 0.0007).cos() * 0.3
            })
            .collect()
    }

    /// **Streaming must equal the whole-file result.**
    ///
    /// This is the property the change rests on. `decode_audio` collected every
    /// sample as interleaved `f32` before resampling — 1.38 GB for an hour of
    /// 48 kHz stereo, against a memory limit that refused it. Feeding packets
    /// through as they arrive holds only the 16 kHz mono output, but it is only
    /// a safe substitution if it produces the same signal.
    ///
    /// The tolerance is for the seams: resampling packet-by-packet interpolates
    /// across boundaries using a carried sample, and the last output of each
    /// packet can land a fraction differently from the batch pass. Anything
    /// larger than this would be an audible discontinuity.
    #[test]
    fn streaming_matches_the_whole_file_resampler() {
        for (rate, channels, packet) in [
            (48_000_u32, 2_usize, 1024_usize),
            (44_100, 2, 512),
            (22_050, 1, 700),
            // The no-resample path: rate already equal to the target.
            (16_000, 2, 333),
        ] {
            let frames = 40_000;
            let all = tone(frames, channels);

            let batch = resample_mono(&all, channels, rate, SAMPLE_RATE);

            let mut stream = StreamResampler::new(SAMPLE_RATE);
            for chunk in all.chunks(packet * channels) {
                stream.push(chunk, channels, rate);
            }
            let streamed = stream.finish();

            assert!(
                streamed.len().abs_diff(batch.len()) <= 2,
                "{rate} Hz x{channels}: streamed {} samples, batch {}",
                streamed.len(),
                batch.len()
            );

            let n = streamed.len().min(batch.len());
            let worst = (0..n)
                .map(|i| (streamed[i] - batch[i]).abs())
                .fold(0.0_f32, f32::max);
            assert!(
                worst < 0.02,
                "{rate} Hz x{channels}: worst sample differs by {worst}, which would be audible"
            );
        }
    }

    /// The packet size must not change the answer.
    ///
    /// A real decoder hands over whatever the codec's frame size happens to be,
    /// and it varies within one file. If the result depended on that, the
    /// transcript would depend on the encoder.
    #[test]
    fn the_packet_size_does_not_change_the_result() {
        let channels = 2;
        let all = tone(30_000, channels);
        let mut lengths = Vec::new();
        for packet in [128_usize, 1024, 4096, 9973] {
            let mut stream = StreamResampler::new(SAMPLE_RATE);
            for chunk in all.chunks(packet * channels) {
                stream.push(chunk, channels, 48_000);
            }
            lengths.push(stream.finish().len());
        }
        let first = lengths[0];
        assert!(
            lengths.iter().all(|n| n.abs_diff(first) <= 1),
            "packet size changed the output length: {lengths:?}"
        );
    }

    /// Nothing in, nothing out — and no panic on a zero-channel packet.
    #[test]
    fn empty_and_degenerate_input_is_safe() {
        let mut s = StreamResampler::new(SAMPLE_RATE);
        s.push(&[], 2, 48_000);
        s.push(&[0.1, 0.2], 0, 48_000);
        assert_eq!(s.len(), 0);
        assert_eq!(s.finish().len(), 0);
    }
}
