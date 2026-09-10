//! MP3 encoding through LAME. Linked dynamically.
//!
//! The caller narrows symphonia full-scale i32 to i16 BEFORE calling
//! [`encode_mp3`]. Narrowing is `(v >> 16) as i16`, the exact inverse of
//! symphonia's widening.

#![allow(non_snake_case, dead_code)]

use std::ffi::c_void;

pub type LameHandle = *mut c_void;

extern "C" {
    /// Allocate encoder state. Returns NULL on failure.
    fn lame_init() -> LameHandle;
    /// Free all resources. Call exactly once per handle.
    fn lame_close(handle: LameHandle) -> i32;
    /// Set input sample rate in Hz. Returns 0 on success.
    ///
    /// This vcpkg LAME spells it `lame_set_in_samplerate`; older releases
    /// called it `lame_set_inSamplerate`.
    fn lame_set_in_samplerate(handle: LameHandle, rate: i32) -> i32;
    /// Set number of channels (1 or 2). Returns 0 on success.
    fn lame_set_num_channels(handle: LameHandle, channels: i32) -> i32;
    /// Set output bitrate in kbps. Returns 0 on success.
    fn lame_set_brate(handle: LameHandle, kbps: i32) -> i32;
    /// Set encoding quality (0=highest, 9=fastest). Returns 0 on success.
    fn lame_set_quality(handle: LameHandle, quality: i32) -> i32;
    /// Must be called after all set_* calls. Returns 0 on success.
    fn lame_init_params(handle: LameHandle) -> i32;
    /// Encode one chunk of interleaved PCM. Returns bytes written, or -1 if buffer too small.
    fn lame_encode_buffer_interleaved(
        handle: LameHandle,
        pcm: *const i16,
        num_samples: i32,
        output: *mut u8,
        output_size: u32,
    ) -> i32;
    /// Flush remaining encoded data. Returns bytes written.
    fn lame_encode_flush(handle: LameHandle, output: *mut u8, output_size: u32) -> i32;
}

/// Encode full-scale i16 interleaved PCM to MP3 at the given bitrate.
pub fn encode_mp3(
    samples_i16: &[i16],
    channels: usize,
    sample_rate: u32,
    bitrate_kbps: u32,
) -> Result<Vec<u8>, String> {
    if channels != 1 && channels != 2 {
        return Err(format!("MP3 supports 1 or 2 channels, got {channels}"));
    }

    // ONE LAME AT A TIME, PROCESS-WIDE.
    //
    // Measured, not assumed: `cargo test -p oc-audio --test native_encoders`
    // crashes with STATUS_ACCESS_VIOLATION roughly one run in six when the
    // harness runs its tests in parallel, and NOT ONCE in six runs with
    // `--test-threads=1`. Narrowing it further, the LAME tests alone reproduce
    // it and the Opus tests alone do not.
    //
    // Each call already owns its own handle -- `lame_init` here, `lame_close`
    // in the guard below -- so the encoder instance is not what is shared. What
    // is shared is libmp3lame's own process-global state, which its lifecycle
    // functions touch and which is not documented as thread-safe.
    //
    // The lock covers the WHOLE call rather than just init and close. Guarding
    // only the lifecycle would be a guess about which global is involved, and a
    // guess that is wrong leaves an intermittent memory-safety fault in shipped
    // code. The cost of the wider lock is nothing this product pays: a batch
    // converts files one after another, so two encodes never overlap in
    // practice -- the crash was reachable from the TEST harness, which is the
    // only thing here that runs them at once.
    //
    // Poisoning is ignored deliberately: a previous panic inside LAME says
    // nothing about whether the next encode can run, and refusing every
    // subsequent MP3 for the life of the process would turn one failure into
    // all of them.
    static LAME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serialised = LAME.lock().unwrap_or_else(|e| e.into_inner());

    // SAFETY: allocates encoder; freed by Guard below.
    let handle = unsafe { lame_init() };
    if handle.is_null() {
        return Err("lame_init failed: out of memory".into());
    }
    struct Guard(LameHandle);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                lame_close(self.0);
            }
        }
    }
    let _guard = Guard(handle);

    // SAFETY: live handle from init above.
    unsafe {
        lame_set_in_samplerate(handle, sample_rate as i32);
        lame_set_num_channels(handle, channels as i32);
        lame_set_brate(handle, bitrate_kbps as i32);
        lame_set_quality(handle, 2);
        if lame_init_params(handle) != 0 {
            return Err("LAME parameter initialisation failed".into());
        }
    }

    let chunk_samples = 1152usize; // MPEG-1 Layer 3 frame size per channel
    let frame_size = chunk_samples * channels;

    // THE OUTPUT BUFFER IS SIZED PER CHANNEL, AND WITH HEADROOM.
    //
    // LAME's contract for `lame_encode_buffer_interleaved` is
    // `1.25 * samples-per-channel + 7200`, and `lame_encode_flush` may use the
    // whole 7200 by itself. `samples-per-channel` is `chunk_samples` -- the
    // `n` passed to the call -- and NOT `frame_size`, which counts samples
    // across every channel.
    //
    // The old expression used `frame_size`, which made the size depend on the
    // channel count in a way the contract does not:
    //
    //   stereo: 2304 * 1.25 + 7200 = 10080, against 8640 required -- 1440 spare
    //   mono:   1152 * 1.25 + 7200 =  8640, against 8640 required -- NONE
    //
    // So the mono path ran permanently at the exact documented minimum, with
    // no margin for anything -- and mono is the arm that has been crashing
    // with STATUS_ACCESS_VIOLATION under whole-workspace test load
    // (`lame::tests::mono_48000hz_produces_output`, the only one of the
    // fourteen that fails to print a result). Being at the boundary is not
    // proven to be the cause; it passes twelve times out of twelve in
    // isolation. It is a latent defect on its own terms either way: a buffer
    // with zero slack against a third-party contract is one revision of that
    // library away from being too small.
    //
    // 1 KiB of slack costs nothing -- this is one allocation per encode, not
    // per frame.
    let mut out_buf = vec![0u8; chunk_samples * 5 / 4 + 7200 + 1024];
    let mut mp3 = Vec::with_capacity(samples_i16.len());

    for chunk in samples_i16.chunks(frame_size) {
        let n = (chunk.len() / channels) as i32;
        // SAFETY: chunk has n*channels valid i16 values; out_buf is large enough.
        let written = unsafe {
            lame_encode_buffer_interleaved(
                handle,
                chunk.as_ptr(),
                n,
                out_buf.as_mut_ptr(),
                out_buf.len() as u32,
            )
        };
        if written < 0 {
            return Err(format!("LAME encoding failed: error {written}"));
        }
        if written > 0 {
            mp3.extend_from_slice(&out_buf[..written as usize]);
        }
    }

    // SAFETY: live handle.
    let written = unsafe { lame_encode_flush(handle, out_buf.as_mut_ptr(), out_buf.len() as u32) };
    if written < 0 {
        // A failed flush means the tail of the stream was never encoded;
        // reporting success would hand out a truncated file under a clean
        // receipt — exactly the dishonesty this product exists to end.
        return Err(format!("LAME flush failed: error {written}"));
    }
    if written > 0 {
        mp3.extend_from_slice(&out_buf[..written as usize]);
    }

    Ok(mp3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_44100hz_produces_valid_mpeg_sync() {
        let samples: Vec<i16> = vec![0i16; 44100 * 2]; // 1 second stereo silence
        let mp3 = encode_mp3(&samples, 2, 44100, 192).unwrap();
        assert!(
            mp3[0] & 0xE0 == 0xE0,
            "missing MPEG sync: first byte {:02x}",
            mp3[0]
        );
        assert!(mp3.len() > 100);
    }

    #[test]
    fn mono_48000hz_produces_output() {
        let samples: Vec<i16> = vec![100i16; 48000];
        let mp3 = encode_mp3(&samples, 1, 48000, 128).unwrap();
        assert!(!mp3.is_empty());
    }

    #[test]
    fn empty_input_does_not_crash() {
        let result = encode_mp3(&[], 2, 44100, 192);
        assert!(result.is_ok()); // header/flush only
    }

    #[test]
    fn three_channels_refused() {
        assert!(encode_mp3(&[], 3, 44100, 192).is_err());
    }
}
