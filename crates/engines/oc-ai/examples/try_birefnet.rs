//! Manual check: does a BiRefNet ONNX export load, run, and produce a matte?
//!
//! `cargo run --release -p oc-ai --example try_birefnet -- <model.onnx> <in> <out.png>`
//!
//! # Why this exists before the registry row does
//!
//! `site/scripts/gates.mjs` carries a note that the features page once listed
//! "BiRefNet-lite, which has no ONNX export that loads". That claim was made
//! about *some* export at *some* point, and the background-removal plan flags
//! the same risk from the other direction: the ONNX conversion replaces
//! deformable convolution — which has no ONNX operator — with a `grid_sample`
//! equivalent, so the exported graph is a rewrite rather than a translation.
//!
//! A model row is a promise that the artifact works. Pinning one on the
//! strength of a licence tag and a file size would be pinning a promise nobody
//! checked, so this runs the thing first: load it, read what it declares, push
//! a real image through, and report whether what comes back is a matte or
//! noise.

#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = std::env::args().skip(1);
    let model = std::fs::read(a.next().ok_or("need <model.onnx>")?)?;
    let input = std::fs::read(a.next().ok_or("need <in.png>")?)?;
    let out_path = a.next().ok_or("need <out.png>")?;

    let limits = openconvert_worker::RunLimits {
        decode_pixels: 50_000_000,
        memory_bytes: std::env::var("MEM_MB")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(4 << 30, |mb| mb << 20),
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };

    let t = std::time::Instant::now();
    let report = oc_ai::probe_matting_model(&model, &input, &limits)?;
    println!("loaded and ran in {:?}", t.elapsed());
    println!("  input  : {}", report.input_shape);
    println!("  output : {}", report.output_shape);
    println!(
        "  matte  : {:.1}% opaque, {:.1}% transparent, {:.1}% partial",
        report.opaque * 100.0,
        report.transparent * 100.0,
        report.partial * 100.0
    );
    std::fs::write(&out_path, &report.cut_out)?;
    println!("  wrote  : {out_path}");

    // A matte that is entirely one value is a model that ran and decided
    // nothing — which loads, produces output, and is useless.
    if report.opaque > 0.999 || report.transparent > 0.999 {
        println!("\nDEGENERATE: the matte is one flat value. This export runs and says nothing.");
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
