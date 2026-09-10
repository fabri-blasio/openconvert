//! Manual check: read text out of an image with real weights.
//!
//! `cargo run -p oc-ai --example try_ocr -- <det> <rec> <dict> <image>`

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile -- which took
// the whole crate's `--all-targets` build down with it on Linux. The example is
// a manual check, so the honest fallback is one that says why it cannot run.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let det = std::fs::read(a.next().ok_or("need <det.onnx>")?)?;
    let rec = std::fs::read(a.next().ok_or("need <rec.onnx>")?)?;
    let dict = std::fs::read(a.next().ok_or("need <dict.txt>")?)?;
    let img = std::fs::read(a.next().ok_or("need <image>")?)?;

    let limits = openconvert_worker::RunLimits {
        decode_pixels: 50_000_000,
        memory_bytes: 1 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };
    let t = std::time::Instant::now();
    let out = oc_ai::ocr_to_text(&img, &[det, rec, dict], &limits)?;
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
