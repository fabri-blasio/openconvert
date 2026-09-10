//! Fixture generator: a real WAV (sine tone) and the matching FLAC. The MKA
//! wrapper lives in openconvert-run's `make_media_fixtures` and consumes THIS
//! one's FLAC output, so run this one first.
//!
//! The two generators carry DISTINCT target names on purpose. Cargo names an
//! example binary after its source file across the whole workspace, so two
//! crates both shipping `examples/make_fixtures.rs` link to one path and
//! collide -- LNK1104 on Windows -- whenever the build parallelises.
//!
//! `cargo run -p oc-audio --example make_audio_fixtures -- <out-dir>`

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/fixtures"));
    std::fs::create_dir_all(&out)?;
    make_wav(&out.join("tone.wav"))?;
    make_flac(&out.join("tone.flac"))?;
    println!("fixtures written to {}", out.display());
    Ok(())
}

/// One second of 440 Hz sine at 48 kHz stereo — long enough that MP3 frames
/// and Ogg pages are exercised in quantity.
fn make_wav(path: &std::path::Path) -> hound::Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec)?;
    for i in 0..48_000i32 {
        let v = ((i as f32) * 440.0 * core::f32::consts::TAU / 48_000.0).sin();
        let s = (v * 20_000.0) as i16;
        w.write_sample(s)?;
        w.write_sample(s)?;
    }
    w.finalize()?;
    Ok(())
}

/// The same tone as FLAC through the encoder oc-audio itself uses.
fn make_flac(path: &std::path::Path) -> Result<(), String> {
    use flacenc::error::Verify as _;
    use flacenc::source::MemSource;

    const RATE: usize = 48_000;
    const CHANNELS: usize = 2;
    let mut samples = Vec::with_capacity(RATE * CHANNELS);
    for i in 0..RATE {
        let v = (i as f32) * 440.0 * core::f32::consts::TAU / RATE as f32;
        let s = (v.sin() * 20_000.0) as i32;
        samples.push(s);
        samples.push(s);
    }

    let verified = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("flac config: {e}"))?;
    const BLOCK_SIZE: usize = 4096;
    let src = MemSource::from_samples(&samples, CHANNELS, 16, RATE);
    let mut stream = flacenc::encode_with_fixed_block_size(&verified, src, BLOCK_SIZE)
        .map_err(|e| format!("flac encode: {e}"))?;

    // Same STREAMINFO canonicalisation the worker performs, and for the same
    // reason -- see oc-audio's `main.rs`. flacenc lowers `min_block_size` to
    // the final partial frame's length, leaving min != max beside
    // FIXED-strategy frames; symphonia reads min == max as the statement
    // "fixed blocking" and refuses every frame when they differ. A fixture
    // WITHOUT this is not a harder test case, it is a file no mainstream
    // decoder accepts -- so it would prove nothing about the routes it feeds.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
        .map_err(|e| format!("flac block sizes: {e}"))?;
    use flacenc::component::BitRepr;
    let mut sink = flacenc::bitsink::ByteSink::with_capacity(stream.count_bits());
    stream
        .write(&mut sink)
        .map_err(|e| format!("flac serialise: {e}"))?;
    std::fs::write(path, sink.as_slice()).map_err(|e| e.to_string())?;
    Ok(())
}
