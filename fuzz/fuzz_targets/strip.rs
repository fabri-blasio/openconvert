//! Metadata stripping over attacker-supplied bytes.
//!
//! The strip walks a segment chain whose lengths come from the file. A wrong
//! bound is an out-of-range slice; a length of zero is an infinite loop. Both
//! are guarded, and this is what keeps them guarded.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let mut out = Vec::new();
    let _ = openconvert_run::strip::strip_jpeg(&mut &data[..], &mut out);
});
