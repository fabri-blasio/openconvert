//! **A `.mov` is an MP4 with a different brand, and this holds that claim.**
//!
//! # The spike behind the route
//!
//! Before `Mov` existed as a format, real QuickTime files were run through the
//! existing `mp4demux` unchanged: two screen-recording exports, a phone-shaped
//! H.264/AAC file, a ProRes export and a QuickTime RLE animation. Every one
//! opened with no QuickTime-specific box handling in the sample tables, which
//! is why `Mov -> Mp4` is a stream copy and there is no `.mov` demuxer.
//!
//! What the spike also found, and what the tests below are:
//!
//! - the AudioSampleEntry is **version 1** in a `.mov` and version 0 in an
//!   `.mp4`, moving the child boxes sixteen bytes;
//! - the `esds` is wrapped in a QuickTime **`wave`** atom;
//! - `ctts` carries **negative** offsets inside a version 0 table, which the
//!   specification says are unsigned.
//!
//! The first two are unit-tested in `mp4demux` against hand-built boxes. This
//! file holds the end-to-end claim: a qt-branded file is detected as `mov`,
//! stream-copies to MP4, and comes out with its samples and their timing
//! intact.
//!
//! # Why the fixture is built rather than committed
//!
//! Same reason as `remux.rs`: a fixture nobody can read is a fixture nobody can
//! trust or extend. This one is muxed by `mp4mux` and then re-branded, so it is
//! honest ISO base media that a `.mov` reader must accept — and it deliberately
//! does NOT reproduce the QuickTime-only shapes above, which have their own
//! tests where they can be built exactly.

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;
use openconvert_run::matroska::{AvGraph, AvSample, AvTrack, TrackKind};
use openconvert_run::remux::stream_copy;

/// A two-track graph whose video samples are reordered, so the composition
/// offsets have something to survive.
fn graph(n: usize) -> AvGraph {
    let track = |number: u64, kind: TrackKind| AvTrack {
        number,
        kind,
        video: (kind == TrackKind::Video).then_some(VideoCodec::H264),
        audio: (kind == TrackKind::Audio).then_some(AudioCodec::Aac),
        codec_id: if kind == TrackKind::Video {
            b"V_MPEG4/ISO/AVC".to_vec()
        } else {
            b"A_AAC".to_vec()
        },
        codec_private: vec![0x11, 0x90, 0x56, 0xE5],
        pixel_width: if kind == TrackKind::Video { 320 } else { 0 },
        pixel_height: if kind == TrackKind::Video { 240 } else { 0 },
        sample_rate: if kind == TrackKind::Audio {
            48_000.0
        } else {
            0.0
        },
        channels: u64::from(kind == TrackKind::Audio) * 2,
        bit_depth: u64::from(kind == TrackKind::Audio) * 16,
        samples: (0..n)
            .map(|i| AvSample {
                ts_ticks: (i as i64) * 33,
                composition_offset_ticks: if kind == TrackKind::Video {
                    [0_i64, 66, -33][i % 3]
                } else {
                    0
                },
                keyframe: i % 4 == 0,
                data: vec![(number as u8) << 4 | (i as u8 & 0x0F); 9 + i * 5],
            })
            .collect(),
    };
    AvGraph {
        tracks: vec![track(1, TrackKind::Video), track(2, TrackKind::Audio)],
        timecode_scale_ns: 1_000_000,
    }
}

/// Muxed MP4, re-branded QuickTime.
///
/// The brand lives in the `ftyp` box's major_brand field, four bytes at offset
/// 8, and nothing else in the file depends on it — which is the whole finding
/// the `Mov` row rests on.
fn quicktime(n: usize) -> Vec<u8> {
    let (mut bytes, _) = openconvert_run::mp4mux::mux(&graph(n)).expect("mux the fixture");
    assert_eq!(
        &bytes[4..8],
        b"ftyp",
        "the fixture must start with an ftyp box"
    );
    bytes[8..12].copy_from_slice(b"qt  ");
    bytes
}

#[test]
fn a_quicktime_brand_is_detected_as_mov() {
    let bytes = quicktime(6);
    let sniffed = openconvert_core::sniff::sniff_bytes(&bytes);
    assert_eq!(
        sniffed.detected,
        FormatId::Mov,
        "a qt-branded ISO base media file is QuickTime, not audio"
    );

    // THE ROW THIS REPLACED. `ftypqt  ` was on the `M4a` signature list, so
    // every `.mov` on a machine -- video, audio and timecode tracks alike --
    // was detected as an AUDIO file and offered only the audio routes.
    assert_ne!(sniffed.detected, FormatId::M4a);
}

#[test]
fn a_quicktime_file_stream_copies_into_mp4_with_its_samples_intact() {
    let source = graph(9);
    let bytes = quicktime(9);

    let (out, removed) = stream_copy(&bytes, FormatId::Mov, FormatId::Mp4, &Limits::defaults())
        .expect("mov -> mp4 is a stream copy");
    assert!(
        removed.is_empty(),
        "nothing in this fixture is uncarryable: {removed:?}"
    );

    let (back, dropped) = openconvert_run::mp4demux::demux(&out, 1 << 24).expect("read it back");
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(back.tracks.len(), source.tracks.len());
    for (a, b) in back.tracks.iter().zip(&source.tracks) {
        assert_eq!(a.kind, b.kind);
        assert_eq!(a.codec_id, b.codec_id, "CodecID must survive verbatim");
        assert_eq!(a.samples.len(), b.samples.len());
        for (i, (x, y)) in a.samples.iter().zip(&b.samples).enumerate() {
            assert_eq!(x.data, y.data, "sample {i} payload");
            assert_eq!(x.keyframe, y.keyframe, "sample {i} sync flag");
            // WHEN it is shown, which is the half `ctts` carries. Without it
            // every frame survived and the video played in the wrong order.
            assert_eq!(
                x.composition_offset_ticks, y.composition_offset_ticks,
                "sample {i} composition offset"
            );
        }
    }
}

#[test]
fn a_quicktime_file_stream_copies_into_matroska() {
    let bytes = quicktime(6);
    let (out, _) = stream_copy(&bytes, FormatId::Mov, FormatId::Mkv, &Limits::defaults())
        .expect("mov -> mkv is a stream copy");
    assert_eq!(
        &out[..4],
        &[0x1A, 0x45, 0xDF, 0xA3],
        "the output must be EBML"
    );

    let back = openconvert_run::matroska::demux(&out, 1 << 24).expect("read the Matroska back");
    assert_eq!(back.tracks.len(), 2);
    // Matroska stores PRESENTATION times, so a reordered video track's first
    // block is at its own composition time rather than at its decode time.
    let video = back.video().expect("a video track");
    assert_eq!(
        (video.pixel_width, video.pixel_height),
        (320, 240),
        "the geometry must survive, which means it must be nested in a Video \
         element where the reader looks for it"
    );
}
