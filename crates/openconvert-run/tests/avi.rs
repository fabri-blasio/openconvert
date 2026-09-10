//! **An AVI is a RIFF file, and its H.264 is in the other bitstream format.**
//!
//! # What this holds that the spike found
//!
//! Four real AVI files were walked before `avidemux` was written — H.264 with
//! MP3, H.264 with PCM, MPEG-4 Part 2 with MP3, and Motion JPEG. Three facts
//! came out of it, and each has a test here:
//!
//! 1. **H.264 in AVI is Annex-B**, with the parameter sets in band and the
//!    `strf` extradata empty. MP4 needs AVCC and an `avcC` built from those
//!    parameter sets. Get it wrong and the output is a file no player opens.
//! 2. **`dwSampleSize` decides what the clock counts.** Assuming audio is
//!    byte-counted turned a two-second file into one claiming 420 seconds.
//! 3. **A codec we cannot carry is dropped and named**, and a file with
//!    nothing carryable is refused — the rule `mp4demux` already follows.
//!
//! # Why the fixture is built here
//!
//! Same reason as `remux.rs` and `quicktime.rs`: a fixture nobody can read is
//! one nobody can trust or extend. This writes a real RIFF by hand, so the
//! offsets, the padding and the header layout are all exercised rather than
//! taken on faith from a file somebody else produced.

use openconvert_run::avidemux::{demux, AviError};

/// A RIFF chunk: id, payload, padded to an even length.
fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 9);
    out.extend_from_slice(id);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(body);
    // ODD LENGTHS ARE PADDED, and forgetting it puts every later chunk one
    // byte out. The fixture bodies below are deliberately odd for that reason.
    if body.len() % 2 == 1 {
        out.push(0);
    }
    out
}

fn list(form: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut inner = form.to_vec();
    inner.extend_from_slice(body);
    chunk(b"LIST", &inner)
}

/// An `AVISTREAMHEADER`.
fn strh(kind: &[u8; 4], handler: &[u8; 4], scale: u32, rate: u32, sample_size: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(56);
    b.extend_from_slice(kind);
    b.extend_from_slice(handler);
    b.extend_from_slice(&[0u8; 12]); // flags, priority, language, initial frames
    b.extend_from_slice(&scale.to_le_bytes()); // 20
    b.extend_from_slice(&rate.to_le_bytes()); // 24
    b.extend_from_slice(&0u32.to_le_bytes()); // 28 start
    b.extend_from_slice(&0u32.to_le_bytes()); // 32 length
    b.extend_from_slice(&0u32.to_le_bytes()); // 36 suggested buffer
    b.extend_from_slice(&0u32.to_le_bytes()); // 40 quality
    b.extend_from_slice(&sample_size.to_le_bytes()); // 44
    b.extend_from_slice(&[0u8; 8]); // 48 rcFrame
    b
}

/// A `BITMAPINFOHEADER` naming a codec.
fn video_format(fourcc: &[u8; 4], w: i32, h: i32) -> Vec<u8> {
    let mut b = Vec::with_capacity(40);
    b.extend_from_slice(&40u32.to_le_bytes());
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // planes
    b.extend_from_slice(&24u16.to_le_bytes()); // bit count
    b.extend_from_slice(fourcc);
    b.extend_from_slice(&[0u8; 20]);
    b
}

/// A `WAVEFORMATEX` naming a format tag.
fn audio_format(tag: u16, channels: u16, rate: u32) -> Vec<u8> {
    let mut b = Vec::with_capacity(18);
    b.extend_from_slice(&tag.to_le_bytes());
    b.extend_from_slice(&channels.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes()); // bytes per second
    b.extend_from_slice(&1u16.to_le_bytes()); // block align
    b.extend_from_slice(&16u16.to_le_bytes()); // bits
    b.extend_from_slice(&0u16.to_le_bytes()); // cbSize
    b
}

/// An Annex-B access unit: the parameter sets, then a picture.
///
/// The SPS is what `avcC` is built from, so its first four bytes are real
/// values — `avcC` copies bytes 1..4 verbatim and a decoder reads the profile
/// and level from them.
fn access_unit(idr: bool, payload: u8, with_params: bool) -> Vec<u8> {
    let mut b = Vec::new();
    if with_params {
        // SPS: NAL type 7, profile 0x42 (baseline), constraints 0xE0, level 30.
        b.extend_from_slice(&[0, 0, 0, 1, 0x67, 0x42, 0xE0, 0x1E, 0xAA, 0xBB]);
        // PPS: NAL type 8.
        b.extend_from_slice(&[0, 0, 0, 1, 0x68, 0xCE, 0x3C, 0x80]);
    }
    // An access unit delimiter, which MP4 does not want.
    b.extend_from_slice(&[0, 0, 0, 1, 0x09, 0x10]);
    // The picture: type 5 is an IDR, type 1 is not.
    b.extend_from_slice(&[0, 0, 1, if idr { 0x65 } else { 0x41 }, payload, payload]);
    b
}

/// A whole AVI.
fn avi(video: Option<(&[u8; 4], Vec<Vec<u8>>)>, audio: Option<(u16, Vec<Vec<u8>>)>) -> Vec<u8> {
    let mut hdrl = chunk(b"avih", &[0u8; 56]);
    let mut movi = Vec::new();

    let mut stream_index = 0u8;
    if let Some((fourcc, frames)) = &video {
        let mut strl = strh(b"vids", fourcc, 1, 25, 0);
        strl = chunk(b"strh", &strl);
        strl.extend_from_slice(&chunk(b"strf", &video_format(fourcc, 320, 240)));
        hdrl.extend_from_slice(&list(b"strl", &strl));
        for f in frames {
            let id = [b'0' + stream_index, b'0', b'd', b'c'];
            movi.extend_from_slice(&chunk(&id, f));
        }
        stream_index += 1;
    }
    if let Some((tag, frames)) = &audio {
        // `dwSampleSize` 0: the clock counts CHUNKS, which is what a
        // variable-bitrate MP3 stream in an AVI actually declares.
        let mut strl = chunk(b"strh", &strh(b"auds", &[1, 0, 0, 0], 32, 1225, 0));
        strl.extend_from_slice(&chunk(b"strf", &audio_format(*tag, 1, 44100)));
        hdrl.extend_from_slice(&list(b"strl", &strl));
        for f in frames {
            let id = [b'0' + stream_index, b'0', b'w', b'b'];
            movi.extend_from_slice(&chunk(&id, f));
        }
    }

    let mut body = b"AVI ".to_vec();
    body.extend_from_slice(&list(b"hdrl", &hdrl));
    body.extend_from_slice(&list(b"movi", &movi));

    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&body);
    out
}

/// H.264 comes out as AVCC, and the `avcC` is built from the stream's own
/// parameter sets — which is the whole job, because the container carries none.
#[test]
fn h264_is_reframed_from_annex_b_and_its_config_is_built_from_the_stream() {
    let frames = vec![
        access_unit(true, 0xA1, true),
        access_unit(false, 0xB2, false),
        access_unit(false, 0xC3, false),
    ];
    let (graph, dropped) = demux(&avi(Some((b"H264", frames)), None), 1 << 20).expect("demux");
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(graph.tracks.len(), 1);

    let t = &graph.tracks[0];
    assert_eq!(t.codec_id, b"V_MPEG4/ISO/AVC");
    assert_eq!((t.pixel_width, t.pixel_height), (320, 240));

    // The configuration: version 1, then the SPS's own profile/constraint/level
    // bytes, then `0xFF` — six reserved bits and lengthSizeMinusOne = 3, which
    // must agree with the four-byte prefixes below or every frame is read at
    // the wrong offset.
    let cfg = &t.codec_private;
    assert_eq!(cfg[0], 1, "configurationVersion");
    assert_eq!(
        &cfg[1..4],
        &[0x42, 0xE0, 0x1E],
        "profile, constraints, level"
    );
    assert_eq!(cfg[4], 0xFF, "lengthSizeMinusOne = 3");
    assert_eq!(cfg[5], 0xE1, "one SPS");

    // Every sample is length-prefixed, and neither the parameter sets nor the
    // access unit delimiter is in it.
    for (i, s) in t.samples.iter().enumerate() {
        let len = u32::from_be_bytes(s.data[..4].try_into().unwrap()) as usize;
        assert_eq!(
            len + 4,
            s.data.len(),
            "sample {i} is one length-prefixed NAL"
        );
        let nal_type = s.data[4] & 0x1F;
        assert!(nal_type == 1 || nal_type == 5, "sample {i} is a picture");
    }
    assert!(t.samples[0].keyframe, "the IDR is a sync point");
    assert!(!t.samples[1].keyframe, "and a non-IDR is not");

    // 25 frames a second, so 40 ms apart.
    assert_eq!(
        t.samples.iter().map(|s| s.ts_ticks).collect::<Vec<_>>(),
        vec![0, 40, 80]
    );
}

/// The clock counts what `dwSampleSize` says it counts.
///
/// **The 420-second bug.** Audio was assumed byte-counted, so a chunk-counted
/// MP3 stream's timestamps were multiplied by its byte count: a two-second file
/// reported seven minutes.
#[test]
fn the_audio_clock_counts_chunks_when_the_header_says_so() {
    let frames: Vec<Vec<u8>> = (0..8).map(|i| vec![0xFF, 0xFB, i, i, i]).collect();
    let (graph, _) = demux(&avi(None, Some((0x0055, frames))), 1 << 20).expect("demux");
    let t = &graph.tracks[0];
    assert_eq!(t.codec_id, b"A_MPEG/L3");

    // scale 32, rate 1225 => 26.1 ms per chunk. Eight of them is 209 ms, not
    // the 5 seconds a byte-counted reading would produce for 40 bytes.
    let last = t.samples.last().expect("samples").ts_ticks;
    assert!(
        (180..=220).contains(&last),
        "eight chunks at 32/1225 s should end near 183 ms, got {last}"
    );
}

/// A codec this build cannot carry is dropped BY NAME, and a file with nothing
/// carryable is refused rather than emptied.
#[test]
fn an_uncarryable_stream_is_named_and_an_empty_result_is_refused() {
    let frames = vec![access_unit(true, 1, true)];
    let (graph, dropped) = demux(
        &avi(
            Some((b"H264", frames)),
            Some((0x0001, vec![vec![1, 2, 3, 4]])),
        ),
        1 << 20,
    )
    .expect("the video still carries");
    assert_eq!(graph.tracks.len(), 1, "only the video");
    assert!(
        dropped.iter().any(|d| d.contains("0x0001")),
        "the PCM track must be named: {dropped:?}"
    );

    // Motion JPEG: nothing carryable at all.
    let err = demux(&avi(Some((b"MJPG", vec![vec![0xFF, 0xD8]])), None), 1 << 20)
        .expect_err("nothing to carry");
    assert!(matches!(err, AviError::NoTracks), "{err}");
}

/// The budget bounds what is retained, and a malformed file stops rather than
/// reading past its own end.
#[test]
fn the_budget_bites_and_malformed_input_terminates() {
    let frames: Vec<Vec<u8>> = (0..40)
        .map(|i| access_unit(i == 0, i as u8, i == 0))
        .collect();
    assert!(
        matches!(
            demux(&avi(Some((b"H264", frames)), None), 64),
            Err(AviError::TooLarge { .. })
        ),
        "64 bytes is not enough for forty frames"
    );

    assert!(demux(b"", 1 << 20).is_err());
    assert!(
        demux(b"RIFF\x00\x00\x00\x00WAVE", 1 << 20).is_err(),
        "a WAV is not an AVI"
    );
    // Truncation at every point stops, and never panics.
    let whole = avi(Some((b"H264", vec![access_unit(true, 1, true)])), None);
    for cut in 0..whole.len() {
        let _ = demux(&whole[..cut], 1 << 20);
    }
}
