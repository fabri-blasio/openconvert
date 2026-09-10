//! Manual check: the streaming decode against the whole-file one, on real audio.
//!
//! `cargo run --release -p oc-ai --example try_decode -- <audio>`
//!
//! Needs no model weights, which is the point: it isolates the decode change
//! from everything downstream of it. Reports both results and the peak
//! intermediate each path holds, because the memory is the reason the change
//! exists — an hour of 48 kHz stereo is 1.38 GB of interleaved `f32` before a
//! single sample reaches the model.

// The adapters this example drives are `#[cfg(all(windows, has_ort))]`, so on
// any other build they do not exist and this file did not compile.
#[cfg(all(windows, has_ort))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("need <audio>")?;
    let bytes = std::fs::read(&path)?;
    let limits = openconvert_worker::RunLimits {
        decode_pixels: 0,
        memory_bytes: 4 << 30,
        archive_depth: 0,
        archive_entries: 0,
        archive_total_bytes: 0,
        use_gpu: true,
    };

    let t = std::time::Instant::now();
    let (raw, channels, rate) = oc_ai::decode_audio_for_test(&bytes, &limits)?;
    let batch_decode = t.elapsed();
    let interleaved_bytes = raw.len() * 4;

    let t2 = std::time::Instant::now();
    let batch = oc_ai::to_mono_16k_for_test(&raw, channels, rate);
    let batch_resample = t2.elapsed();

    let t3 = std::time::Instant::now();
    let streamed = oc_ai::decode_audio_mono_for_test(&bytes, &limits, 16_000)?;
    let stream_total = t3.elapsed();

    println!(
        "source: {channels} ch @ {rate} Hz, {:.1} s",
        raw.len() as f64 / f64::from(rate) / channels as f64
    );
    println!();
    println!("  whole-file : decode {batch_decode:?} + resample {batch_resample:?}");
    println!(
        "               peak intermediate {:.0} MB (interleaved) + {:.0} MB (mono)",
        interleaved_bytes as f64 / 1e6,
        batch.len() as f64 * 4.0 / 1e6
    );
    println!("  streaming  : {stream_total:?}");
    println!(
        "               peak intermediate {:.0} MB (mono only)",
        streamed.len() as f64 * 4.0 / 1e6
    );
    println!();

    // The signals must agree, or the transcript would change with the decode.
    if streamed.len().abs_diff(batch.len()) > 2 {
        println!(
            "MISMATCH: streamed {} samples, batch {}",
            streamed.len(),
            batch.len()
        );
        return Ok(());
    }
    let n = streamed.len().min(batch.len());
    let worst = (0..n)
        .map(|i| (streamed[i] - batch[i]).abs())
        .fold(0.0_f32, f32::max);
    let mean = (0..n)
        .map(|i| f64::from((streamed[i] - batch[i]).abs()))
        .sum::<f64>()
        / n as f64;
    println!("  samples: {n} compared, worst |difference| {worst:.6}, mean {mean:.8}");
    println!(
        "  {}",
        if worst < 0.02 {
            "SAME SIGNAL"
        } else {
            "DIFFERENT — investigate before shipping"
        }
    );
    Ok(())
}

#[cfg(not(all(windows, has_ort)))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "this example needs a Windows build with the ONNX Runtime present. oc-ai's adapters are gated on `all(windows, has_ort)`."
            .into(),
    )
}
