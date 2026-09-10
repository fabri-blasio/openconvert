//! Manual check: background removal through the **quality** adapter.
//!
//! `cargo run --release -p oc-ai --example try_bg_quality -- <modnet.onnx> <in> <out.png>`
//!
//! Separate from `try_bg` because it drives a genuinely different code path,
//! and that distinction was missed once already: `try_bg` calls
//! `remove_background`, which is the u2netp adapter — 320 px input, `normalise`,
//! saliency post-processing with a min-max stretch. `remove_background_quality`
//! is 512 px, `normalise_signed`, and treats the output as an alpha channel
//! directly. Running the first with the second's weights loads the file and
//! proves nothing about the second.
//!
//! The app defaults to this path whenever MODNet is installed, so it is the
//! one most users get.

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile -- which took
// the whole crate's `--all-targets` build down with it on Linux. The example is
// a manual check, so the honest fallback is one that says why it cannot run.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let model = std::fs::read(a.next().ok_or("need <modnet.onnx>")?)?;
    let input = std::fs::read(a.next().ok_or("need <in.png>")?)?;
    let out_path = a.next().ok_or("need <out.png>")?;

    let limits = openconvert_worker::RunLimits {
        decode_pixels: 50_000_000,
        memory_bytes: 1 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };

    let t = std::time::Instant::now();
    let done = oc_ai::remove_background_quality(&input, &model, "png", &limits)?;
    std::fs::write(&out_path, &done.bytes)?;
    println!("ok: {} bytes in {:?}", done.bytes.len(), t.elapsed());
    for line in &done.removed {
        println!("  {line}");
    }

    // A matte that is entirely opaque or entirely transparent is the failure
    // this example exists to catch: it writes a file, so the caller reports
    // success, and the picture is either unchanged or gone. Counting the alpha
    // channel is the only way to tell that from a real cut-out.
    let img = image::load_from_memory(&done.bytes)?.to_rgba8();
    let total = img.pixels().len();
    let opaque = img.pixels().filter(|p| p.0[3] > 250).count();
    let clear = img.pixels().filter(|p| p.0[3] < 5).count();
    println!(
        "  alpha: {:.1}% opaque, {:.1}% transparent, {:.1}% partial",
        100.0 * opaque as f64 / total as f64,
        100.0 * clear as f64 / total as f64,
        100.0 * (total - opaque - clear) as f64 / total as f64,
    );
    if opaque == total {
        println!("  WARNING: nothing was cut out — the matte is fully opaque");
    }
    if clear == total {
        println!("  WARNING: everything was cut out — the matte is fully transparent");
    }
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
            .into(),
    )
}
