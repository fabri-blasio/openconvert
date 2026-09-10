//! Scoring over arbitrary signals.
//!
//! Prediction must be total and deterministic: no panic, and no NaN. A NaN
//! score sorts unpredictably, so the suggestion shown to the user would depend
//! on the sort implementation rather than on the evidence.
//!
//! # The history fields are floats, and that is the point of fuzzing them
//!
//! They are decayed weights rather than counts, so the shell can hand this any
//! `f32` at all — including a NaN produced by a corrupted journal, an infinity,
//! or a negative weight. `personal_weight` is written to return 0 for every one
//! of those, and this is what holds it to that: a NaN reaching the blend would
//! produce a NaN score, which is exactly the failure the assertions below name.
#![no_main]
use libfuzzer_sys::fuzz_target;
use openconvert_core::format::TABLE;
use openconvert_core::predict::{score, FolderCategory, Signals};
use openconvert_core::target::Target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 24 {
        return;
    }
    let pick = |i: usize| TABLE[data[i] as usize % TABLE.len()].id;
    // Straight from the input bytes, so every bit pattern is reachable —
    // including the signalling NaNs and the subnormals a `0.0..=1.0` generator
    // would never produce.
    let float = |i: usize| {
        f32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
    };

    let s = Signals {
        input: pick(0),
        history_count: float(2),
        history_to_target: float(6),
        folder_history_count: float(10),
        folder_history_to_target: float(14),
        folder_pattern: u32::from(data[18]),
        sibling_same_stem: data[19] & 1 == 1,
        batch_size: u32::from(data[20]),
        folder: match data[21] % 4 {
            0 => FolderCategory::Screenshots,
            1 => FolderCategory::Scans,
            2 => FolderCategory::Downloads,
            _ => FolderCategory::Unknown,
        },
    };
    let v = score(&s, Target::Format(pick(1)));
    assert!(v.is_finite(), "score returned {v}, which does not sort");
    assert!((0.0..=1.0).contains(&v), "score {v} is outside 0..=1");
});
