//! `oc-ai` as a library, so the adapters can be tested and driven directly.
//!
//! The binary is a thin protocol shell over this; everything that decides
//! anything lives here, which is what makes the pre- and post-processing
//! testable without spawning a confined process.
//!
//! See `main.rs` for why the weights arrive on the pipe rather than from disk.

#[cfg(all(windows, has_ort))]
pub mod accelerator;
#[cfg(all(windows, has_ort))]
mod denoise;
#[cfg(all(windows, has_ort))]
mod infer;
#[cfg(all(windows, has_ort))]
mod mel;
#[cfg(all(windows, has_ort))]
mod ocr;
#[cfg(all(windows, has_ort))]
mod upscale;
#[cfg(all(windows, has_ort))]
pub mod vad;
#[cfg(all(windows, has_ort))]
mod whisper;

/// Test-only re-exports, for the decode comparison example.
///
/// The decode path is `pub(crate)` because nothing outside this crate has any
/// business calling it — but the change that made it streaming has to be
/// checkable against the whole-file path on real audio, and that check should
/// not need model weights to run.
#[cfg(all(windows, has_ort))]
pub use infer::{
    decode_audio as decode_audio_for_test, decode_audio_mono as decode_audio_mono_for_test,
};
#[cfg(all(windows, has_ort))]
pub use mel::to_mono_16k as to_mono_16k_for_test;

#[cfg(all(windows, has_ort))]
pub use infer::{
    denoise_audio, ocr_to_layout, ocr_to_text, probe_matting_model, remove_background,
    remove_background_best, remove_background_birefnet, remove_background_quality, runtime_version,
    transcribe_to_format_progress, transcribe_to_text, transcribe_to_text_progress, upscale_image,
    vad_probe, MattingProbe,
};

/// The adapter names this build can run, for the refusal message and for the
/// host-side registry to agree with.
pub const ADAPTERS: &[&str] = &[
    "remove-background",
    "remove-background-quality",
    "ocr",
    "transcribe",
    "upscale",
    "denoise",
];
