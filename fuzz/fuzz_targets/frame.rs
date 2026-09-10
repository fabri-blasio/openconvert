//! The protocol framer, over arbitrary bytes.
//!
//! The point is not that it must not panic -- it is that it must not
//! ALLOCATE. A length prefix is a correct decode however large it is, so the
//! bug fuzzing normally hunts for does not exist here. What exists is a
//! reader that believes a number.
//!
//! So this target asserts the bound rather than merely surviving it: anything
//! that comes back is at most MAX_FRAME_BYTES, and the process is expected to
//! stay within libFuzzer's own memory ceiling, which is what would catch a
//! regression that started allocating first and checking second.
#![no_main]
use libfuzzer_sys::fuzz_target;
use std::io::Cursor;
use openconvert_sandbox::protocol::{read_frame, read_frames, MAX_FRAME_BYTES};

fuzz_target!(|data: &[u8]| {
    if let Ok(frame) = read_frame(&mut Cursor::new(data)) {
        assert!(
            frame.len() as u32 <= MAX_FRAME_BYTES,
            "a frame of {} bytes came back past the {MAX_FRAME_BYTES} limit",
            frame.len()
        );
        assert!(!frame.is_empty(), "a zero-length frame was accepted");
    }

    // And the sequence reader, which has its own bound. Never terminal, so a
    // missing count check shows up as a hang rather than a wrong answer -- and
    // libFuzzer's timeout is what reports it.
    let _ = read_frames(&mut Cursor::new(data), |_| false);
});
