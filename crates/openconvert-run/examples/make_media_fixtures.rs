//! Fixture generator for the A10 manual conversion checks.
//!
//! `cargo run -p openconvert-run --example make_media_fixtures -- <out-dir>`
//!
//! Emits a minimal three-page PDF here and wraps oc-audio's FLAC as an MKA;
//! the WAV/FLAC halves come from `oc-audio --example make_audio_fixtures`, so
//! each fixture stays closest to the crate that owns its format and that one
//! runs first. The two examples are named differently because Cargo names
//! example binaries workspace-wide: a shared name links to a shared path.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/fixtures"));
    std::fs::create_dir_all(&out)?;
    make_three_page_pdf(&out.join("three-pages.pdf"))?;
    if out.join("tone.wav").exists() {
        make_mka(&out.join("tone.mka"), &out.join("tone.wav"))?;
    } else {
        eprintln!("note: target/fixtures/tone.wav not found; skipping tone.mka");
    }
    println!("fixtures written to {}", out.display());
    Ok(())
}

/// Build an audio-only Matroska carrying a real FLAC track.
///
/// # Frame boundaries come from the encoder, not from a scanner
///
/// Two earlier versions of this got it wrong in instructive ways.
///
/// The first re-boxed an already-encoded FLAC by hunting for the frame sync
/// pattern (`0xFF`, then `0xF8` under a mask). That pattern occurs inside
/// frame DATA too, so blocks were cut at false positives and symphonia
/// decoded five frames before "unexpected end of bitstream".
///
/// The second switched to raw PCM to avoid framing entirely. symphonia's PCM
/// decoder requires `max_frames_per_packet`, and Matroska has no element that
/// carries it -- so `A_PCM/INT/LIT` in MKV is undecodable there regardless of
/// how correct the file is.
///
/// So the encoder is asked directly: `Stream::frame(n)` hands back each frame
/// as a structure, and serialising them one at a time gives exact boundaries
/// by construction rather than by guess.
fn make_mka(
    path: &std::path::Path,
    wav_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use flacenc::bitsink::ByteSink;
    use flacenc::component::BitRepr;
    use flacenc::error::Verify as _;
    use flacenc::source::MemSource;
    use openconvert_core::codec::AudioCodec;
    use openconvert_run::matroska::{AvGraph, AvSample, AvTrack, TrackKind};

    const RATE: usize = 48_000;
    const CHANNELS: usize = 2;
    const BLOCK: usize = 4096;

    let mut reader = hound::WavReader::open(wav_path)?;
    let spec = reader.spec();
    if spec.channels as usize != CHANNELS || spec.sample_rate as usize != RATE {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "this fixture expects the 48 kHz stereo tone",
        )));
    }
    let pcm: Vec<i32> = reader
        .samples::<i16>()
        .map(|s| s.map(i32::from))
        .collect::<Result<_, _>>()?;

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| format!("flac config: {e}"))?;
    let source = MemSource::from_samples(&pcm, CHANNELS, 16, RATE);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, BLOCK)
        .map_err(|e| format!("flac encode: {e}"))?;

    // The header, taken from the whole-stream serialisation: `fLaC` plus every
    // metadata block. Matroska's CodecPrivate for A_FLAC is exactly this, the
    // marker included -- symphonia refuses with "missing flac stream marker"
    // without it. Walking metadata blocks is reliable in a way that scanning
    // for frame syncs is not: each block states its own length.
    let mut whole = ByteSink::with_capacity(stream.count_bits());
    stream
        .write(&mut whole)
        .map_err(|e| format!("flac serialise: {e}"))?;
    let whole = whole.as_slice();
    let mut cursor = 4usize; // past "fLaC"
    loop {
        let header = whole
            .get(cursor..cursor + 4)
            .ok_or("truncated flac metadata")?;
        let last = header[0] & 0x80 != 0;
        let len = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
        cursor += 4 + len;
        if last {
            break;
        }
    }
    let codec_private = whole[..cursor].to_vec();

    // One block per FLAC frame, each serialised on its own.
    let mut samples = Vec::new();
    let mut ts_ticks: i64 = 0;
    let mut n = 0usize;
    while let Some(frame) = stream.frame(n) {
        let mut sink = ByteSink::with_capacity(frame.count_bits());
        frame
            .write(&mut sink)
            .map_err(|e| format!("flac frame {n}: {e}"))?;
        let block_samples = frame.header().block_size() as i64;
        samples.push(AvSample {
            // No B-frames in a fixture, so decode order is display order.
            composition_offset_ticks: 0,
            ts_ticks,
            keyframe: true,
            data: sink.as_slice().to_vec(),
        });
        ts_ticks += block_samples;
        n += 1;
    }
    if samples.is_empty() {
        return Err(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the encoder produced no frames",
        )));
    }

    let track = AvTrack {
        number: 1,
        kind: TrackKind::Audio,
        video: None,
        audio: Some(AudioCodec::Flac),
        codec_id: b"A_FLAC".to_vec(),
        codec_private,
        pixel_width: 0,
        pixel_height: 0,
        // The Audio element a real muxer writes. Matroska defaults these to
        // 8 kHz mono, so a track that omits them does not read as
        // "unspecified" downstream -- it reads as wrong.
        sample_rate: RATE as f64,
        channels: CHANNELS as u64,
        bit_depth: 16,
        samples,
    };
    let graph = AvGraph {
        tracks: vec![track],
        timecode_scale_ns: 1_000_000,
    };
    let ebml = openconvert_run::matroska::graph_to_ebml(&graph)?;
    std::fs::write(path, ebml)?;
    Ok(())
}

/// A minimal, valid three-page PDF, written object-by-object with the xref
/// offsets recorded as we go — hand-built so page selection has something
/// honest to select against.
///
/// OBJECTS ARE EMITTED IN STRICT NUMERIC ORDER, and that is not a style
/// choice. An earlier version interleaved each page with its own content
/// stream (1, 2, 3, 6, 4, 7, 5, 8, 9) while writing the xref entries in
/// ascending order, so every entry from object 4 on pointed at the wrong
/// byte. A reader that repairs a broken xref by scanning still finds page 0,
/// which is precisely what made the fixture dangerous: it looked like it
/// worked, and would have been read as "page selection is broken" rather than
/// "the file is".
fn make_three_page_pdf(path: &std::path::Path) -> anyhow::Result<()> {
    let mut pdf: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = vec![0]; // object 0 is the free entry

    let obj = |pdf: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &str| {
        offsets.push(pdf.len());
        pdf.extend_from_slice(body.as_bytes());
    };

    pdf.extend_from_slice(
        b"%PDF-1.4
",
    );

    // 1: catalog. 2: page tree. 3..5: pages. 6..8: contents. 9: font.
    obj(
        &mut pdf,
        &mut offsets,
        "1 0 obj
<< /Type /Catalog /Pages 2 0 R >>
endobj
",
    );
    obj(
        &mut pdf,
        &mut offsets,
        "2 0 obj
<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>
endobj
",
    );

    // Objects 3, 4, 5 -- the pages.
    for page_no in 3..=5u32 {
        let content_no = page_no + 3;
        obj(
            &mut pdf,
            &mut offsets,
            &format!(
                "{page_no} 0 obj
<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792]                  /Contents {content_no} 0 R /Resources << /Font << /F1 9 0 R >> >> >>
endobj
"
            ),
        );
    }

    // Objects 6, 7, 8 -- the content streams, one per page, each naming its
    // own page number so a rendered image can be told apart from its siblings
    // by eye as well as by hash.
    for page_no in 3..=5u32 {
        let content_no = page_no + 3;
        let text = format!(
            "BT /F1 48 Tf 72 700 Td (OpenConvert page {} of 3) Tj ET
",
            page_no - 2
        );
        obj(
            &mut pdf,
            &mut offsets,
            &format!(
                "{content_no} 0 obj
<< /Length {} >>
stream
{text}endstream
endobj
",
                text.len()
            ),
        );
    }

    obj(
        &mut pdf,
        &mut offsets,
        "9 0 obj
<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>
endobj
",
    );

    let xref_at = pdf.len();
    pdf.extend_from_slice(
        b"xref
0 10
0000000000 65535 f 
",
    );
    for at in &offsets[1..] {
        pdf.extend_from_slice(
            format!(
                "{at:010} 00000 n 
"
            )
            .as_bytes(),
        );
    }
    pdf.extend_from_slice(
        format!(
            "trailer
<< /Size 10 /Root 1 0 R >>
startxref
{xref_at}
%%EOF
"
        )
        .as_bytes(),
    );
    std::fs::write(path, pdf)?;
    Ok(())
}
