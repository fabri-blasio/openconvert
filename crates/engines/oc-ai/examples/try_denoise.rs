//! Manual check: enhance an audio file and write the result.
//!
//! `cargo run --release -p oc-ai --example try_denoise -- <model> <aux> <in> <out.wav>`

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile -- which took
// the whole crate's `--all-targets` build down with it on Linux. The example is
// a manual check, so the honest fallback is one that says why it cannot run.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let model = std::fs::read(a.next().ok_or("need <model.onnx>")?)?;
    let aux = std::fs::read(a.next().ok_or("need <aux.bin>")?)?;
    let input = std::fs::read(a.next().ok_or("need <in>")?)?;
    let out_path = a.next().ok_or("need <out.wav>")?;
    let limits = openconvert_worker::RunLimits {
        decode_pixels: 1,
        memory_bytes: 1 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };
    let t = std::time::Instant::now();
    let done = oc_ai::denoise_audio(&input, &[model, aux], &limits)?;
    println!("ok: {} bytes in {:?}", done.bytes.len(), t.elapsed());
    std::fs::write(out_path, &done.bytes)?;
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() {
    eprintln!(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
    );
}
