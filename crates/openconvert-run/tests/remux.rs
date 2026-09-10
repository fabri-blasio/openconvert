//! The flagship, held to its promise: `MKV Ã¢â€ â€™ MP4` selects stream copy,
//! completes in-process, and the coded samples come out BIT-IDENTICAL.
//!
//! The fixture MKV is built here from checked-in code rather than shipped as
//! a binary blob, for the same reason the evil corpus is generated in-repo:
//! fixtures nobody can read are fixtures nobody can trust or extend.

use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;
use openconvert_run::matroska;
use openconvert_run::remux::stream_copy;

// ---------------------------------------------------------------------------
// Minimal EBML writing Ã¢â‚¬â€ enough to emit a structurally honest Matroska file
// ---------------------------------------------------------------------------

fn vint_size(len: usize) -> Vec<u8> {
    // 8-byte form always: marker 0x01 + 7 length bytes.
    let mut out = vec![0x01];
    out.extend_from_slice(&(len as u64).to_be_bytes()[1..]);
    out
}

/// Canonical ID vint (leading zeros trimmed).
fn elem(id: u64, payload: &[u8]) -> Vec<u8> {
    let be = id.to_be_bytes();
    let first_nonzero = be.iter().position(|&b| b != 0).unwrap_or(7);
    let mut out = Vec::with_capacity(payload.len() + 16);
    out.extend_from_slice(&be[first_nonzero..]);
    out.extend_from_slice(&vint_size(payload.len()));
    out.extend_from_slice(payload);
    out
}

fn uint_payload(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![0];
    }
    let be = v.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(7);
    be[start..].to_vec()
}

struct TestMkv {
    video_samples: Vec<Vec<u8>>,
    audio_samples: Vec<Vec<u8>>,
}

impl TestMkv {
    /// H.264 "samples" are length-prefixed NALs (AVCC), exactly what both
    /// Matroska and MP4 store; their CONTENT is arbitrary because this file
    /// never decodes them.
    fn build(&self) -> Vec<u8> {
        const V_TRACK: u64 = 1;
        const A_TRACK: u64 = 2;

        let avcc = {
            // avcC-shaped private data: config version, profile bits, then a
            // plausible SPS/PPS pair so muxing has something to carry.
            let sps = [0x67, 0x64, 0x00, 0x1F, 0xAC];
            let pps = [0x68, 0xEB, 0xEC];
            let mut b = vec![0x01, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, sps.len() as u8];
            b.extend_from_slice(&sps);
            b.push(0x01);
            b.push(pps.len() as u8);
            b.extend_from_slice(&pps);
            b
        };
        let asc = [0x12, 0x10]; // AudioSpecificConfig: AAC-LC, stereo, 44.1k

        // Tracks
        let mut video_fields = elem(matroska_id::TRACK_NUMBER, &uint_payload(V_TRACK));
        video_fields.extend(&elem(matroska_id::TRACK_TYPE, &[0x01]));
        video_fields.extend(&elem(matroska_id::CODEC_ID, b"V_MPEG4/ISO/AVC"));
        video_fields.extend(&elem(matroska_id::CODEC_PRIVATE, &avcc));
        video_fields.extend(&elem(matroska_id::PIXEL_WIDTH, &uint_payload(192)));
        video_fields.extend(&elem(matroska_id::PIXEL_HEIGHT, &uint_payload(108)));

        let mut audio_fields = elem(matroska_id::TRACK_NUMBER, &uint_payload(A_TRACK));
        audio_fields.extend(&elem(matroska_id::TRACK_TYPE, &[0x02]));
        audio_fields.extend(&elem(matroska_id::CODEC_ID, b"A_AAC"));
        audio_fields.extend(&elem(matroska_id::CODEC_PRIVATE, &asc));

        // Wrap each track's fields in the TrackEntry element that carries
        // them.
        let video_track = elem(matroska_id::TRACK_ENTRY, &video_fields);
        let audio_track = elem(matroska_id::TRACK_ENTRY, &audio_fields);
        let tracks = elem(matroska_id::TRACKS, &[video_track, audio_track].concat());

        // Clusters: one per sample pair, alternating keyframes on video.
        let mut clusters = Vec::new();
        for (ci, (v, a)) in self
            .video_samples
            .iter()
            .zip(&self.audio_samples)
            .enumerate()
        {
            let mut cluster_tc = elem(
                matroska_id::CLUSTER_TIMECODE,
                &(u32::try_from(ci * 40).unwrap().to_be_bytes()),
            );

            let flags = if ci % 2 == 0 { 0x80 } else { 0x00 }; // keyframe bit
            let mut vb = vec![0x80 | V_TRACK as u8]; // track numbers are ID-form vints
            vb.extend_from_slice(&(ci as u16 * 40).to_be_bytes());
            vb.push(flags);
            vb.extend_from_slice(v);
            cluster_tc.extend(&elem(matroska_id::SIMPLE_BLOCK, &vb));

            let mut ab = vec![0x80 | A_TRACK as u8];
            ab.extend_from_slice(&(ci as u16 * 40 + 20).to_be_bytes());
            ab.push(0x00);
            ab.extend_from_slice(a);
            cluster_tc.extend(&elem(matroska_id::SIMPLE_BLOCK, &ab));

            clusters.push(elem(matroska_id::CLUSTER, &cluster_tc));
        }

        let info = elem(
            matroska_id::INFO,
            &elem(0x002A_D7B1, &uint_payload(1_000_000)),
        );
        let mut segment_payload = info;
        segment_payload.extend_from_slice(&tracks);
        segment_payload.extend(clusters.concat());
        // Segment with UNKNOWN size would be refused by our own reader, so
        // the fixture uses an honest long-form size like any muxer should.
        let mut out = Vec::new();
        out.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3]); // EBML header magic id
        out.extend_from_slice(&vint_size(0)); // header payload empty (legal for our reader)
        out.extend_from_slice(&[0x18, 0x53, 0x80, 0x67]); // Segment id, canonical
        out.extend_from_slice(&vint_size(segment_payload.len()));
        out.extend_from_slice(&segment_payload);
        out
    }
}

/// IDs duplicated here rather than imported from `matroska`'s private module:
/// the fixture is deliberately independent of implementation constants, so a
/// renamed constant cannot silently desync the test's understanding of EBML.
mod matroska_id {
    pub const CLUSTER: u64 = 0x1F43_B675;
    pub const CLUSTER_TIMECODE: u64 = 0xE7;
    pub const SIMPLE_BLOCK: u64 = 0xA3;
    pub const TRACKS: u64 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u64 = 0xAE;
    pub const TRACK_NUMBER: u64 = 0xD7;
    pub const TRACK_TYPE: u64 = 0x83;
    pub const CODEC_ID: u64 = 0x86;
    pub const CODEC_PRIVATE: u64 = 0x63A2;
    pub const PIXEL_WIDTH: u64 = 0xB0;
    pub const PIXEL_HEIGHT: u64 = 0xBA;
    pub const INFO: u64 = 0x1549_A966;
}

fn fixture() -> (TestMkv, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let video: Vec<Vec<u8>> = (0..6)
        .map(|i| {
            let mut s = vec![0x65]; // NAL type 5 (IDR) high bits, arbitrary body
            s.extend_from_slice(format!("video-frame-{i}").as_bytes());
            s
        })
        .collect();
    let audio: Vec<Vec<u8>> = (0..6)
        .map(|i| format!("aac-frame-{i}").into_bytes())
        .collect();
    (
        TestMkv {
            video_samples: video.clone(),
            audio_samples: audio.clone(),
        },
        video,
        audio,
    )
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

/// **THE FLAGSHIP GATE.** Every coded sample survives `MKV Ã¢â€ â€™ MP4` unchanged,
/// asserted against the fixture's own bytes.
#[test]
fn mkv_to_mp4_preserves_every_sample_bit_for_bit() {
    let (mkv, video, audio) = fixture();
    let bytes = mkv.build();

    let (mp4, removed) = stream_copy(&bytes, FormatId::Mkv, FormatId::Mp4, &Limits::defaults())
        .expect("the flagship conversion must succeed");

    assert!(removed.is_empty(), "a pure remux discloses nothing lost");
    assert!(mp4.windows(4).any(|w| w == b"ftyp"), "no ftyp box");
    assert!(mp4.windows(4).any(|w| w == b"moov"), "no moov box");

    // mdat starts at its declared offset; its payload must contain every
    // sample concatenated in track order (video track first, then audio),
    // which is how the single-chunk-per-track layout writes it.
    let mdat_at = mp4
        .windows(4)
        .position(|w| w == b"mdat")
        .expect("mdat present");
    let declared_len = u32::from_be_bytes([
        mp4[mdat_at - 4],
        mp4[mdat_at - 4 + 1],
        mp4[mdat_at - 4 + 2],
        mp4[mdat_at - 4 + 3],
    ]) as usize;
    assert!(mdat_at - 4 + 8 <= mp4.len());
    let payload = &mp4[mdat_at + 4..mdat_at - 4 + declared_len];

    let expected: Vec<u8> = video.iter().chain(&audio).flatten().copied().collect();
    assert_eq!(
        payload.len(),
        expected.len(),
        "mdat payload size changed: {} vs {}",
        payload.len(),
        expected.len()
    );
    assert_eq!(payload, expected, "SAMPLES CHANGED ACROSS THE REMUX");
}

/// The stts table coalesces constant-rate deltas into ONE run Ã¢â‚¬â€ the property
/// that makes output size predictable and players happy.
#[test]
fn constant_rate_timestamps_produce_a_single_stts_run() {
    let (mkv, _, _) = fixture();
    let bytes = mkv.build();
    let (mp4, _) =
        stream_copy(&bytes, FormatId::Mkv, FormatId::Mp4, &Limits::defaults()).expect("mux");

    let stts = mp4.windows(4).position(|w| w == b"stts").expect("stts");
    // full-box: size+name(8) | ver+flags(4) | entry_count(4) Ã¢â€ â€™ count at +12
    let count = u32::from_be_bytes([mp4[stts + 8], mp4[stts + 9], mp4[stts + 10], mp4[stts + 11]]);
    assert_eq!(
        count, 1,
        "uniform deltas must coalesce into one run, got {count}"
    );
}

/// WebM Ã¢â€ â€™ MKV surgery keeps every block byte-identical: same elements, new
/// header only.
#[test]
fn webm_to_mkv_surgery_preserves_blocks_verbatim() {
    let (mut mkv, _, _) = fixture();
    // Flip nothing but the DocType story Ã¢â‚¬â€ the fixture IS Matroska already,
    // so treat it as the "webm input" stand-in; the operation under test is
    // identity-of-blocks across surgery, which is the claim.
    let src = mkv.build();

    let (out, removed) =
        stream_copy(&src, FormatId::Webm, FormatId::Mkv, &Limits::defaults()).expect("surgery");
    assert!(removed.is_empty());

    // Every SIMPLE_BLOCK element in the source must appear in the output
    // byte-for-byte, in order.
    let blocks = collect_simple_blocks(&src);
    assert!(!blocks.is_empty(), "fixture produced no blocks");
    for block in blocks {
        assert!(
            out.windows(block.len()).any(|w| w == block.as_slice()),
            "block went missing or changed during surgery"
        );
    }
    mkv.audio_samples.clear(); // silence unused-mut lint while keeping shape
}

/// MKA extraction drops the video track AND its blocks, keeps the audio
/// intact, and DISCLOSES what was removed.
#[test]
fn mka_extraction_drops_video_and_discloses_it() {
    let (mkv, _, audio) = fixture();
    let src = mkv.build();

    let (out, removed) =
        stream_copy(&src, FormatId::Mkv, FormatId::Mka, &Limits::defaults()).expect("extraction");
    assert!(
        removed.iter().any(|r| r.contains("non-audio")),
        "dropping tracks without disclosure breaks the promise: {removed:?}"
    );

    // Audio blocks survive.
    for a in &audio {
        assert!(
            out.windows(a.len()).any(|w| w == a.as_slice()),
            "an audio sample was lost"
        );
    }
    // Video samples do not.
    let graph = matroska::demux(&out, Limits::defaults().memory_bytes).expect("re-demux");
    assert!(
        graph.video().is_none(),
        "video survived into an audio-only container"
    );
    assert_eq!(graph.audio().map_or(0, |t| t.samples.len()), audio.len());
}

/// VP9-in-MKV refuses MP4 at ROUTING time Ã¢â‚¬â€ tested here at the layer below
/// routing by asking the muxer directly, since routing was pinned elsewhere.
#[test]
fn vp9_video_refuses_mp4_by_name_rather_than_producing_a_dead_file() {
    let mut mkv = fixture().0.build();
    // Swap the video CodecID to VP9 inside the raw fixture bytes.
    let needle = b"V_MPEG4/ISO/AVC";
    let pos = mkv
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("codec id present");
    // Overwrite the FULL field with VP9 plus NUL padding, the way a real
    // remux would rewrite it â€” a partial overwrite leaves residue that
    // classifies as Other and tests nothing about VP9.
    let mut replacement = vec![0u8; needle.len()];
    replacement[..5].copy_from_slice(b"V_VP9");
    mkv[pos..pos + needle.len()].copy_from_slice(&replacement);

    let err = stream_copy(&mkv, FormatId::Mkv, FormatId::Mp4, &Limits::defaults())
        .expect_err("VP9 must not remux into MP4");
    assert!(
        err.to_string().contains("Vp9"),
        "the refusal must name the codec: {err}"
    );
}

/// Collect every SIMPLE_BLOCK element (its FULL element bytes) from raw EBML,
/// descending into Segment and Cluster containers only.
fn collect_simple_blocks(bytes: &[u8]) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    walk_blocks(bytes, &mut found);
    found
}

fn walk_blocks(bytes: &[u8], found: &mut Vec<Vec<u8>>) {
    let mut pos = 0usize;
    while pos < bytes.len() {
        let Some((_id, ilen)) = read_vint_keep_marker(bytes, pos) else {
            break;
        };
        let Some((size, slen)) = read_vint(bytes, pos + ilen) else {
            break;
        };
        let body_start = pos + ilen + slen;
        let Some(body_end) = body_start.checked_add(size as usize) else {
            break;
        };
        if body_end > bytes.len() {
            break;
        }
        match _id {
            0xA3 => found.push(bytes[body_start..body_end].to_vec()),
            0x1853_8067 | 0x1F43_B675 => walk_blocks(&bytes[body_start..body_end], found),
            _ => {}
        }
        pos = body_end;
    }
}
/// ID-form vint (marker kept) and size-form vint (marker stripped).
fn read_vint_keep_marker(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let first = *bytes.get(at)?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || bytes.len() < at + len {
        return None;
    }
    let mut val = u64::from(first);
    for i in 1..len {
        val = (val << 8) | u64::from(bytes[at + i]);
    }
    Some((val, len))
}

fn read_vint(bytes: &[u8], at: usize) -> Option<(u64, usize)> {
    let first = *bytes.get(at)?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || bytes.len() < at + len {
        return None;
    }
    let mask = if len >= 8 {
        0u8
    } else {
        (0xFF_u16 >> len) as u8
    };
    let mut val = u64::from(first & mask);
    for i in 1..len {
        val = (val << 8) | u64::from(bytes[at + i]);
    }
    Some((val, len))
}
