//! Manual check: transcribe an audio file with real weights.
//!
//! `cargo run --release -p oc-ai --example try_asr -- <enc> <dec> <tok> <vad> <audio>`

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile -- which took
// the whole crate's `--all-targets` build down with it on Linux. The example is
// a manual check, so the honest fallback is one that says why it cannot run.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let enc = std::fs::read(a.next().ok_or("need <encoder.onnx>")?)?;
    let dec = std::fs::read(a.next().ok_or("need <decoder.onnx>")?)?;
    let tok = std::fs::read(a.next().ok_or("need <tokenizer.json>")?)?;
    // THE FOURTH ARTIFACT. `transcribe_to_text` destructures four models and
    // this example passed three, so it failed at RUN time with a message about
    // artifact counts -- a stale harness reporting a fault in the thing it was
    // supposed to be testing. The silence detector was added to the adapter and
    // never added here.
    let vad = std::fs::read(a.next().ok_or("need <silero-vad.onnx>")?)?;
    let audio = std::fs::read(a.next().ok_or("need <audio>")?)?;

    let limits = openconvert_worker::RunLimits {
        decode_pixels: 50_000_000,
        memory_bytes: 1 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };
    let t = std::time::Instant::now();
    let out = oc_ai::transcribe_to_text(&audio, &[enc, dec, tok, vad], &limits)?;
    println!("--- {:?} ---", t.elapsed());
    println!("{}", String::from_utf8_lossy(&out.bytes));
    println!("--- end ---");
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() {
    eprintln!(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
    );
}
