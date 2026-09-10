//! Diagnostic: speech probabilities over a file.
//!
//! `cargo run --release -p oc-ai --example try_vad -- <vad.onnx> <audio>`

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile -- which took
// the whole crate's `--all-targets` build down with it on Linux. The example is
// a manual check, so the honest fallback is one that says why it cannot run.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let model = std::fs::read(a.next().ok_or("need <vad.onnx>")?)?;
    let audio = std::fs::read(a.next().ok_or("need <audio>")?)?;
    let limits = openconvert_worker::RunLimits {
        decode_pixels: 1,
        memory_bytes: 1 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };
    let p = oc_ai::vad_probe(&audio, &model, &limits)?;
    let max = p.iter().cloned().fold(f32::MIN, f32::max);
    let over = p.iter().filter(|&&v| v >= 0.5).count();
    println!(
        "chunks={} max={max:.4} over_0.5={over} ({:.0}%)",
        p.len(),
        100.0 * over as f32 / p.len() as f32
    );
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() {
    eprintln!(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
    );
}
