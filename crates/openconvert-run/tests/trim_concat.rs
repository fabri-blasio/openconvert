//! Lossless trim/concat, held to the same promise as the flagship remux:
//! sample bytes travel verbatim, and only PRESENCE and TIMESTAMPS change.
//!
//! Trim must snap its start backward to a keyframe (a cut that begins
//! mid-GOP decodes grey); concat must refuse mismatched codecs rather than
//! splice H.264 onto VP9. The fixture MKV is built here from checked-in
//! code, same as `remux.rs`, so every timestamp in every test is legible.

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::limits::Limits;
use openconvert_run::matroska;
use openconvert_run::matroska::{AvGraph, AvSample, AvTrack, TrackKind};

// ---------------------------------------------------------------------------
// Minimal EBML writing — enough to emit a structurally honest Matroska file
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

/// IDs duplicated here rather than imported from `matroska`'s private module,
/// for the same reason remux.rs does it: the fixture stays independent of the
/// implementation's constants.
mod matroska_id {
    pub const INFO: u64 = 0x1549_A966;
    pub const TIMECODE_SCALE: u64 = 0x002A_D7B1;
    pub const TRACKS: u64 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u64 = 0xAE;
    pub const TRACK_NUMBER: u64 = 0xD7;
    pub const TRACK_TYPE: u64 = 0x83;
    pub const CODEC_ID: u64 = 0x86;
    pub const CODEC_PRIVATE: u64 = 0x63A2;
    pub const PIXEL_WIDTH: u64 = 0xB0;
    pub const PIXEL_HEIGHT: u64 = 0xBA;
    pub const CLUSTER: u64 = 0x1F43_B675;
    pub const CLUSTER_TIMECODE: u64 = 0xE7;
    pub const SIMPLE_BLOCK: u64 = 0xA3;
}

/// One sample to place in the fixture: absolute timestamp in ticks (= ms at
/// the default TimecodeScale), keyframe bit, and payload bytes unique per
/// tag+timestamp so byte-equality assertions actually pin identity.
struct SampleSpec {
    ts: u32,
    keyframe: bool,
    data: Vec<u8>,
}

fn spec(ts: u32, keyframe: bool, tag: &str) -> SampleSpec {
    SampleSpec {
        ts,
        keyframe,
        data: format!("{tag}-{ts}").into_bytes(),
    }
}

struct TestMkv {
    video: Vec<SampleSpec>,
    audio: Vec<SampleSpec>,
}

impl TestMkv {
    /// Video is track 1, audio track 2; each sample rides in its own cluster
    /// whose timecode IS the absolute timestamp (block-relative value zero).
    fn build(&self) -> Vec<u8> {
        const V_TRACK: u64 = 1;
        const A_TRACK: u64 = 2;

        let avcc = {
            let sps = [0x67, 0x64, 0x00, 0x1F, 0xAC];
            let pps = [0x68, 0xEB, 0xEC];
            let mut b = vec![0x01, 0x64, 0x00, 0x1F, 0xFF, 0xE1, 0x00, sps.len() as u8];
            b.extend_from_slice(&sps);
            b.push(0x01);
            b.push(pps.len() as u8);
            b.extend_from_slice(&pps);
            b
        };
        let asc = [0x12, 0x10]; // AudioSpecificConfig: AAC-LC

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

        let tracks = elem(
            matroska_id::TRACKS,
            &[
                elem(matroska_id::TRACK_ENTRY, &video_fields),
                elem(matroska_id::TRACK_ENTRY, &audio_fields),
            ]
            .concat(),
        );

        let cluster_for = |track: u64, s: &SampleSpec| {
            let mut inner = elem(matroska_id::CLUSTER_TIMECODE, &s.ts.to_be_bytes());
            let mut block = vec![0x80 | track as u8];
            block.extend_from_slice(&0_i16.to_be_bytes()); // relative to cluster tc
            block.push(if s.keyframe { 0x80 } else { 0x00 });
            block.extend_from_slice(&s.data);
            inner.extend(&elem(matroska_id::SIMPLE_BLOCK, &block));
            elem(matroska_id::CLUSTER, &inner)
        };

        let clusters = self
            .video
            .iter()
            .map(|s| cluster_for(V_TRACK, s))
            .chain(self.audio.iter().map(|s| cluster_for(A_TRACK, s)))
            .collect::<Vec<Vec<u8>>>();

        let info = elem(
            matroska_id::INFO,
            &elem(matroska_id::TIMECODE_SCALE, &uint_payload(1_000_000)),
        );
        let mut segment_payload = info;
        segment_payload.extend_from_slice(&tracks);
        segment_payload.extend(clusters.concat());

        let mut out = Vec::new();
        out.extend_from_slice(&[0x1A, 0x45, 0xDF, 0xA3]); // EBML header magic id
        out.extend_from_slice(&vint_size(0)); // header payload empty (legal)
        out.extend_from_slice(&[0x18, 0x53, 0x80, 0x67]); // Segment id, canonical
        out.extend_from_slice(&vint_size(segment_payload.len()));
        out.extend_from_slice(&segment_payload);
        out
    }
}

/// `n` video frames every 80 ms starting at 0 (keyframe on even indices) plus
/// `n` audio frames on the midpoints — two tracks, interleaved cadence. Also
/// returns both payloads' data so tests can assert byte-for-byte survival.
fn pair_fixture(n: usize) -> (TestMkv, Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let video: Vec<SampleSpec> = (0..n)
        .map(|i| spec((i as u32) * 80, i % 2 == 0, "video"))
        .collect();
    let audio: Vec<SampleSpec> = (0..n)
        .map(|i| spec((i as u32) * 80 + 40, false, "audio"))
        .collect();
    let vdata = video.iter().map(|s| s.data.clone()).collect();
    let adata = audio.iter().map(|s| s.data.clone()).collect();
    (TestMkv { video, audio }, vdata, adata)
}

/// Video-only fixture: frames every `step` ms starting at 0, keyframe on even
/// indices. Audio track exists but carries nothing.
fn video_fixture(count: usize, step: u32) -> TestMkv {
    TestMkv {
        video: (0..count)
            .map(|i| spec((i as u32) * step, i % 2 == 0, "video"))
            .collect(),
        audio: Vec::new(),
    }
}

fn graph_of(bytes: &[u8]) -> matroska::AvGraph {
    matroska::demux(bytes, Limits::defaults().memory_bytes).expect("fixture demuxes")
}

/// A hand-built track for cases the EBML fixture cannot express cheaply:
/// degenerate cadences, odd track counts. H264/AAC identity follows `kind`.
fn hand_track(number: u64, kind: TrackKind, ticks: &[i64]) -> AvTrack {
    let (video, audio) = match kind {
        TrackKind::Video => (Some(VideoCodec::H264), None),
        TrackKind::Audio => (None, Some(AudioCodec::Aac)),
        TrackKind::Other => (None, None),
    };
    AvTrack {
        number,
        kind,
        video,
        audio,
        codec_id: Vec::new(),
        codec_private: Vec::new(),
        pixel_width: 0,
        pixel_height: 0,
        // Left unset on purpose: these tests are about block cadence and
        // track pairing, and a track that never reported an Audio element is
        // exactly what `write_tracks` must not invent one for.
        sample_rate: 0.0,
        channels: 0,
        bit_depth: 0,
        samples: ticks
            .iter()
            .map(|&t| AvSample {
                // No B-frames in a fixture, so decode order is display order.
                composition_offset_ticks: 0,
                ts_ticks: t,
                keyframe: false,
                data: format!("hand-{number}-{t}").into_bytes(),
            })
            .collect(),
    }
}

fn hand_graph(tracks: Vec<AvTrack>) -> AvGraph {
    AvGraph {
        tracks,
        timecode_scale_ns: 1_000_000,
    }
}

// ---------------------------------------------------------------------------
// The gates
// ---------------------------------------------------------------------------

/// Trimming [0, ∞) keeps everything: counts, timestamps, keyframe bits and
/// bytes all identical to the source graph.
#[test]
fn trim_at_zero_is_identity() {
    let (fx, _, _) = pair_fixture(6);
    let g = graph_of(&fx.build());
    let trimmed = matroska::trim(&g, 0, u64::MAX).expect("full window keeps everything");

    assert_eq!(trimmed.timecode_scale_ns, g.timecode_scale_ns);
    assert_eq!(trimmed.tracks.len(), g.tracks.len());
    for (t, orig) in trimmed.tracks.iter().zip(&g.tracks) {
        assert_eq!(
            t.samples.len(),
            orig.samples.len(),
            "identity trim changed a track's sample count"
        );
        for (s, o) in t.samples.iter().zip(&orig.samples) {
            assert_eq!((s.ts_ticks, s.keyframe), (o.ts_ticks, o.keyframe));
            assert_eq!(s.data, o.data, "identity trim changed sample bytes");
        }
    }
}

/// Timestamps 0..400 step 80, keyframes at 0/160/320: trimming [100, 300)
/// keeps exactly t=160 and t=240. The first frame at/after 100ms is already
/// a keyframe, so no snapback prefix appears and nothing outside survives.
#[test]
fn trim_drops_samples_outside_range() {
    let g = graph_of(&video_fixture(6, 80).build());
    let trimmed = matroska::trim(&g, 100, 300).expect("window intersects the track");

    let v = trimmed.video().expect("video track");
    let ts: Vec<i64> = v.samples.iter().map(|s| s.ts_ticks).collect();
    assert_eq!(ts, vec![160, 240], "exactly the in-range samples survive");
    assert!(v.samples[0].keyframe, "output begins on a keyframe");
    for t in &ts {
        assert!(
            (100_i64..300_i64).contains(t),
            "kept sample at {t} outside [100, 300)"
        );
    }
}

/// t=0 is a keyframe, t=80 is not: cutting at 40ms must pull the start back
/// to t=0, because beginning at t=80 hands the player a frame it cannot
/// decode. The whole remainder of the track follows.
#[test]
fn trim_snaps_to_keyframe_boundary() {
    let fx = TestMkv {
        video: vec![
            spec(0, true, "v"),
            spec(80, false, "v"),
            spec(160, false, "v"),
        ],
        audio: Vec::new(),
    };
    let g = graph_of(&fx.build());
    let trimmed = matroska::trim(&g, 40, u64::MAX).expect("window intersects the track");

    let v = trimmed.video().expect("video track");
    assert_eq!(v.samples.len(), 3, "snapback prefix plus the rest");
    assert_eq!(
        v.samples[0].ts_ticks, 0,
        "start must snap back to the keyframe before the cut"
    );
    assert!(v.samples[0].keyframe);
}

/// Two three-sample graphs join into six samples per track, every timeline
/// strictly increasing across the seam.
#[test]
fn concat_produces_monotonic_timestamps() {
    let (fa, _, _) = pair_fixture(3);
    let (fb, _, _) = pair_fixture(3);
    let a = graph_of(&fa.build());
    let b = graph_of(&fb.build());

    let joined = matroska::concat(&a, &b).expect("identical codecs concatenate");
    assert_eq!(joined.timecode_scale_ns, a.timecode_scale_ns);
    for t in &joined.tracks {
        assert_eq!(t.samples.len(), 6, "three samples plus three samples");
        for w in t.samples.windows(2) {
            assert!(
                w[0].ts_ticks < w[1].ts_ticks,
                "timestamps must strictly increase across the seam"
            );
        }
    }
}

/// Graph A is H.264+AAC; graph B is VP9+AAC after a full-width CodecID swap.
/// The join must REFUSE by name rather than emit an unplayable hybrid.
#[test]
fn concat_refuses_mismatched_codecs() {
    let (fa, _, _) = pair_fixture(3);
    let mut fb_bytes = pair_fixture(3).0.build();

    let needle = b"V_MPEG4/ISO/AVC";
    let pos = fb_bytes
        .windows(needle.len())
        .position(|w| w == needle)
        .expect("codec id present in fixture");
    // Overwrite the FULL field with VP9 plus NUL padding — a partial
    // overwrite would classify as Other and test the wrong refusal.
    let mut replacement = vec![0u8; needle.len()];
    replacement[..5].copy_from_slice(b"V_VP9");
    fb_bytes[pos..pos + needle.len()].copy_from_slice(&replacement);

    let a = graph_of(&fa.build());
    let b = graph_of(&fb_bytes);
    let err = matroska::concat(&a, &b).expect_err("H.264 onto VP9 must refuse");
    assert!(
        err.to_string().contains("codec sets differ"),
        "the refusal must name the reason: {err}"
    );
}

/// Trim [0,160) plus the tail [160, ∞): together the halves cover every
/// sample exactly once, so their concatenation reproduces ALL original
/// blocks byte-for-byte, in order, on both tracks.
#[test]
fn concat_of_trimmed_segments_preserves_bytes() {
    let (fx, vdata, adata) = pair_fixture(6);
    let g = graph_of(&fx.build());

    let head = matroska::trim(&g, 0, 160).expect("head window has content");
    let tail = matroska::trim(&g, 160, u64::MAX).expect("tail window has content");
    let joined = matroska::concat(&head, &tail).expect("halves of one file share codecs");

    let v = joined.video().expect("video track");
    assert_eq!(v.samples.len(), vdata.len(), "a video block went missing");
    for (s, want) in v.samples.iter().zip(&vdata) {
        assert_eq!(s.data, *want, "video block changed across trim+concat");
    }

    let au = joined.audio().expect("audio track");
    assert_eq!(au.samples.len(), adata.len(), "an audio block went missing");
    for (s, want) in au.samples.iter().zip(&adata) {
        assert_eq!(s.data, *want, "audio block changed across trim+concat");
    }

    for t in &joined.tracks {
        for w in t.samples.windows(2) {
            assert!(w[0].ts_ticks < w[1].ts_ticks);
        }
    }
}

// ---------------------------------------------------------------------------
// Refusal and hardening gates (review follow-ups)
// ---------------------------------------------------------------------------

/// A window past the content — or an inverted one — must be named at the
/// trim layer, not handed downstream as a container of nothing.
#[test]
fn trim_selecting_nothing_refuses_rather_than_emitting_silence() {
    let g = graph_of(&pair_fixture(3).0.build()); // content ends at tick 200

    let err = matroska::trim(&g, 10_000, 20_000).expect_err("past-end window");
    assert!(
        err.to_string().contains("no samples"),
        "the refusal must name the problem: {err}"
    );

    let err = matroska::trim(&g, 300, 100).expect_err("inverted window");
    assert!(err.to_string().contains("no samples"), "{err}");
}

/// Same codec NAME is not the same stream setup: different avcC payloads
/// must refuse, because the output carries only A's setup data.
#[test]
fn concat_refuses_different_codec_private() {
    let (fa, _, _) = pair_fixture(3);
    let mut fb_bytes = pair_fixture(3).0.build();

    // Flip one SPS byte inside B's avcC — codec IDs stay identical.
    let sps = [0x67u8, 0x64, 0x00, 0x1F, 0xAC];
    let pos = fb_bytes
        .windows(sps.len())
        .position(|w| w == sps)
        .expect("avcC SPS present in fixture");
    fb_bytes[pos + 4] = 0xAD;

    let a = graph_of(&fa.build());
    let b = graph_of(&fb_bytes);
    let err = matroska::concat(&a, &b).expect_err("differing CodecPrivate must refuse");
    assert!(
        err.to_string().contains("codec configurations differ"),
        "{err}"
    );
}

/// Differing TimecodeScale would resample one side's timestamps; refusal
/// path pinned here since it has no dedicated brief test.
#[test]
fn concat_refuses_differing_timecode_scale() {
    let (fa, _, _) = pair_fixture(3);
    let mut b = graph_of(&pair_fixture(3).0.build());
    b.timecode_scale_ns = 500_000;
    let a = graph_of(&fa.build());

    let err = matroska::concat(&a, &b).expect_err("scale mismatch must refuse");
    assert!(err.to_string().contains("timecode scales differ"), "{err}");
}

/// Track-count mismatch gets its OWN refusal naming the layout, not the
/// codecs — the reason must match the actual failure.
#[test]
fn concat_refuses_mismatched_track_layouts_by_name() {
    let a = graph_of(&pair_fixture(3).0.build()); // two tracks
    let b = hand_graph(vec![hand_track(1, TrackKind::Video, &[0])]);

    let err = matroska::concat(&a, &b).expect_err("layout mismatch must refuse");
    assert!(err.to_string().contains("track layouts differ"), "{err}");
}

/// A with single-sample tracks has NO delta to offer; the offset floor must
/// still land B strictly after A instead of on top of it.
#[test]
fn concat_with_degenerate_delta_keeps_b_strictly_after_a() {
    // A's furthest last sample (50) sits on the VIDEO track; with no median
    // delta available the naive offset equals 50, which would duplicate the
    // timestamp when B's first video frame sits at 0.
    let a = hand_graph(vec![
        hand_track(1, TrackKind::Video, &[50]),
        hand_track(2, TrackKind::Audio, &[20]),
    ]);
    let b = hand_graph(vec![
        hand_track(1, TrackKind::Video, &[0]),
        hand_track(2, TrackKind::Audio, &[40]),
    ]);

    let joined = matroska::concat(&a, &b).expect("identical configs concatenate");
    for t in &joined.tracks {
        assert_eq!(t.samples.len(), 2);
        assert!(
            t.samples[0].ts_ticks < t.samples[1].ts_ticks,
            "B landed ON A: {:?}",
            t.samples.iter().map(|s| s.ts_ticks).collect::<Vec<_>>()
        );
    }
}

// ---------------------------------------------------------------------------
// graph_to_ebml: the writer half of demux()
// ---------------------------------------------------------------------------

/// **The video geometry is nested in a Video master element.**
///
/// A round trip through our own reader cannot catch this: the reader accepts
/// both the nested form and the flat one, deliberately, because files written
/// before the fix exist. What caught it was ffmpeg, which reports
/// "Unknown entry 0xB0" and refuses the file outright — so every `mp4 -> mkv`
/// this build produced was readable by nothing but itself.
///
/// So this asserts the BYTES: PixelWidth appears, and it appears inside an
/// element 0xE0.
#[test]
fn the_writer_nests_video_geometry_where_a_player_looks_for_it() {
    let (fx, _, _) = pair_fixture(4);
    let out = matroska::graph_to_ebml(&graph_of(&fx.build())).expect("serialise");

    // 0xE0 Video, whose payload begins with 0xB0 PixelWidth. The length VINT
    // between them is 8 bytes with the writer's fixed-width encoding.
    // The writer encodes every length as a fixed 8-byte VINT, so a master
    // element is one id byte, eight length bytes, then its first child.
    let video_at = out
        .windows(2)
        .position(|w| w == [0xE0, 0x01])
        .expect("a Video master element (0xE0) in the written tracks");
    assert_eq!(
        out.get(video_at + 9),
        Some(&0xB0),
        "PixelWidth must be the Video element's first child, not a TrackEntry child"
    );
    assert!(
        out[video_at + 9..].windows(1).take(32).any(|w| w == [0xBA]),
        "PixelHeight must be in there with it"
    );
}

/// A demux → serialise → demux round trip preserves everything the graph
/// carries: track identity (number, kind, CodecID, CodecPrivate bytes),
/// sample payloads byte-for-byte, timestamps and keyframe bits. This is the
/// gate that makes trim/concat output files honest Matroska rather than
/// structurally plausible noise.
#[test]
fn graph_to_ebml_round_trips_through_demux() {
    let (fx, vdata, adata) = pair_fixture(6);
    let g = graph_of(&fx.build());

    let serialised = matroska::graph_to_ebml(&g).expect("a demuxed fixture re-serialises");
    let back = graph_of(&serialised);

    assert_eq!(back.timecode_scale_ns, g.timecode_scale_ns);
    assert_eq!(back.tracks.len(), g.tracks.len());
    for (t, orig) in back.tracks.iter().zip(&g.tracks) {
        assert_eq!(t.number, orig.number);
        assert_eq!(t.kind, orig.kind);
        assert_eq!(t.video, orig.video);
        assert_eq!(t.audio, orig.audio);
        assert_eq!(
            t.codec_id, orig.codec_id,
            "CodecID bytes must survive verbatim"
        );
        assert_eq!(t.codec_private, orig.codec_private);
        assert_eq!(
            (t.pixel_width, t.pixel_height),
            (orig.pixel_width, orig.pixel_height)
        );
        assert_eq!(t.samples.len(), orig.samples.len());
        for (s, o) in t.samples.iter().zip(&orig.samples) {
            assert_eq!((s.ts_ticks, s.keyframe), (o.ts_ticks, o.keyframe));
            assert_eq!(
                s.data, o.data,
                "sample payload changed across the round trip"
            );
        }
    }
    // And the payloads really are the fixture's unique per-tag bytes.
    let v = back.video().expect("video track");
    for (s, want) in v.samples.iter().zip(&vdata) {
        assert_eq!(s.data, *want);
    }
    let au = back.audio().expect("audio track");
    for (s, want) in au.samples.iter().zip(&adata) {
        assert_eq!(s.data, *want);
    }
}

/// The output is a file our OWN reader accepts from the top level — EBML
/// header first, segment second — and trim-through-serialise produces exactly
/// the kept samples.
#[test]
fn trimmed_graph_serialises_and_re_demuxes() {
    let fx = TestMkv {
        video: vec![
            spec(0, true, "v"),
            spec(80, false, "v"),
            spec(160, false, "v"),
        ],
        audio: Vec::new(),
    };
    let g = graph_of(&fx.build());
    let trimmed = matroska::trim(&g, 40, u64::MAX).expect("window intersects");

    let out = matroska::graph_to_ebml(&trimmed).expect("trimmed graph serialises");
    let back = graph_of(&out);
    let v = back.video().expect("video track");
    assert_eq!(v.samples.len(), 3);
    assert_eq!(v.samples[0].ts_ticks, 0, "snapback keyframe leads the file");
    assert_eq!(
        v.samples.iter().map(|s| s.data.clone()).collect::<Vec<_>>(),
        vec![b"v-0".to_vec(), b"v-80".to_vec(), b"v-160".to_vec()]
    );
}
