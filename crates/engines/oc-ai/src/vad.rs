//! Silero VAD: which parts of a recording contain speech.
//!
//! # What this is for
//!
//! Whisper processes a fixed 30-second window whether or not anyone is
//! speaking, and fed silence it does not return nothing — it returns
//! *something*, confidently. That is a documented failure mode, and the
//! `MAX_TOKENS` bound in [`crate::whisper`] stops a runaway loop without
//! stopping a single fabricated sentence.
//!
//! So every window is checked before it is transcribed. A window with no
//! speech is skipped entirely: no invented text, and no model run either.
//!
//! # The context window is the whole trick
//!
//! A first attempt fed the model 512 samples per call — the chunk size every
//! description of v5 quotes — and it scored **below 0.004 on unmistakable
//! speech**, so every window was skipped and transcription returned empty for
//! everything. It was removed rather than shipped.
//!
//! The chunk size was right and the INPUT size was not. Silero v5 prepends 64
//! samples carried over from the previous call, and that concatenation happens
//! **outside** the graph: the tensor it wants is 576 long, not 512. The LSTM
//! state is separate and does not carry it. Feeding 512 is accepted without
//! complaint, because the input dimension is dynamic, and then every frame
//! looks like a discontinuity.

use ndarray::{Array2, Array3};
use ort::session::Session;
use ort::value::Value;

/// New samples per call. Fixed by the v5 model at 16 kHz.
const CHUNK: usize = 512;

/// Samples carried over from the previous call.
///
/// 64 at 16 kHz. This is not a lookback window anyone chose — it is what the
/// reference wrapper concatenates, and the graph was traced with it present.
const CONTEXT: usize = 64;

/// What the model actually receives.
const INPUT: usize = CONTEXT + CHUNK;

/// Above this, a chunk counts as speech.
///
/// Silero's own examples use 0.5. The cost of a false positive is one
/// unnecessary Whisper run; the cost of a false negative is losing real words.
/// When in doubt this errs toward transcribing.
const SPEECH_THRESHOLD: f32 = 0.5;

/// Runs the VAD over a stream, carrying both pieces of its memory.
///
/// The state is the LSTM's; the context is the last 64 samples. Resetting
/// either between chunks of one recording makes every chunk look like the
/// start of a new one.
pub struct Vad {
    session: Session,
    state: Array3<f32>,
    context: Vec<f32>,
}

impl Vad {
    /// Load the model.
    ///
    /// # Errors
    ///
    /// Weights the runtime will not load.
    pub fn new(model: &[u8]) -> Result<Self, String> {
        Ok(Self {
            session: crate::infer::session_from(model)?,
            // [2, batch, 128], zeroed: the documented initial state.
            state: Array3::zeros((2, 1, 128)),
            context: vec![0.0; CONTEXT],
        })
    }

    /// Whether any chunk of `samples` contains speech.
    ///
    /// # Errors
    ///
    /// A run failure, or an output that is not the probability this expects.
    pub fn has_speech(&mut self, samples: &[f32]) -> Result<bool, String> {
        Ok(self
            .probabilities(samples)?
            .iter()
            .any(|&p| p >= SPEECH_THRESHOLD))
    }

    /// Every chunk's speech probability, in order.
    ///
    /// # Errors
    ///
    /// A run failure, or an output that is not the probability this expects.
    pub fn probabilities(&mut self, samples: &[f32]) -> Result<Vec<f32>, String> {
        let mut out = Vec::with_capacity(samples.len() / CHUNK + 1);
        for chunk in samples.chunks(CHUNK) {
            // Context first, then the new samples. A final partial chunk is
            // zero-padded rather than dropped: a recording can end mid-word.
            let mut buf = vec![0.0_f32; INPUT];
            buf[..CONTEXT].copy_from_slice(&self.context);
            buf[CONTEXT..CONTEXT + chunk.len()].copy_from_slice(chunk);

            let input = Array2::from_shape_vec((1, INPUT), buf.clone())
                .map_err(|e| format!("could not build the VAD input: {e}"))?;
            // RANK ZERO. The graph declares `sr` with shape `[]`, and a `[1]`
            // tensor is accepted and then read as something else.
            let sr = ndarray::arr0(16_000_i64);

            let outputs = self
                .session
                .run(ort::inputs![
                    "input" => Value::from_array(input)
                        .map_err(|e| format!("could not build the VAD input: {e}"))?,
                    "state" => Value::from_array(self.state.clone())
                        .map_err(|e| format!("could not build the VAD state: {e}"))?,
                    "sr" => Value::from_array(sr)
                        .map_err(|e| format!("could not build the VAD rate: {e}"))?,
                ])
                .map_err(|e| format!("the voice-activity model failed: {e}"))?;

            let prob = {
                let o = outputs
                    .get("output")
                    .ok_or("the VAD returned no probability")?;
                let (_, data) = o
                    .try_extract_tensor::<f32>()
                    .map_err(|e| format!("the VAD's output was not a float: {e}"))?;
                *data
                    .first()
                    .ok_or("the VAD returned an empty probability")?
            };

            if let Some(next) = outputs.get("stateN") {
                if let Ok((shape, data)) = next.try_extract_tensor::<f32>() {
                    let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
                    if dims.len() == 3 {
                        if let Ok(a) =
                            Array3::from_shape_vec((dims[0], dims[1], dims[2]), data.to_vec())
                        {
                            self.state = a;
                        }
                    }
                }
            }
            // The tail of what was just fed becomes the next call's context.
            self.context.copy_from_slice(&buf[INPUT - CONTEXT..]);
            out.push(prob);
        }
        Ok(out)
    }
}
