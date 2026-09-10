//! The container header reader, against files built byte by byte here.
//!
//! No fixture files. Every input below is assembled from the published layout,
//! so a test that passes says the reader agrees with the specification rather
//! than with whatever a muxer happened to emit the day someone recorded a blob.
//!
//! Half of these are hostile. A container is a nested structure an attacker
//! writes, and this reader runs **in the host** — the one place in this design
//! where a parser bug is not contained by anything.

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::format::FormatId;
use openconvert_run::container::read_streams;

// ---------------------------------------------------------------------------
// MP4 builders
// ---------------------------------------------------------------------------

/// One ISO BMFF box: 32-bit size, four-character type, payload.
fn box_of(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    out
}

/// A sample-description box naming one codec.
fn stsd(codes: &[&[u8; 4]]) -> Vec<u8> {
    let mut payload = vec![0u8; 4]; // version + flags
    payload.extend_from_slice(&(codes.len() as u32).to_be_bytes());
    for code in codes {
        // A minimal sample entry: size, format, then the six reserved bytes and
        // data-reference index every entry carries.
        let mut entry = Vec::new();
        entry.extend_from_slice(&16u32.to_be_bytes());
        entry.extend_from_slice(*code);
        entry.extend_from_slice(&[0u8; 8]);
        payload.extend_from_slice(&entry);
    }
    box_of(b"stsd", &payload)
}

/// A version-0 track header carrying 16.16 fixed-point dimensions.
fn tkhd(width: u32, height: u32) -> Vec<u8> {
    let mut payload = vec![0u8; 84];
    payload[0] = 0; // version
    payload[76..80].copy_from_slice(&(width << 16).to_be_bytes());
    payload[80..84].copy_from_slice(&(height << 16).to_be_bytes());
    box_of(b"tkhd", &payload)
}

/// A whole file: `ftyp`, then a `moov` holding one track per codec given.
fn mp4(video: Option<&[u8; 4]>, audio: Option<&[u8; 4]>, w: u32, h: u32) -> Vec<u8> {
    let mut moov = Vec::new();
    if let Some(code) = video {
        let mut trak = tkhd(w, h);
        trak.extend_from_slice(&box_of(
            b"mdia",
            &box_of(b"minf", &box_of(b"stbl", &stsd(&[code]))),
        ));
        moov.extend_from_slice(&box_of(b"trak", &trak));
    }
    if let Some(code) = audio {
        // An audio track's tkhd is 0x0 -- the case that would erase the video
        // dimensions if the reader took the last pair rather than the first.
        let mut trak = tkhd(0, 0);
        trak.extend_from_slice(&box_of(
            b"mdia",
            &box_of(b"minf", &box_of(b"stbl", &stsd(&[code]))),
        ));
        moov.extend_from_slice(&box_of(b"trak", &trak));
    }
    let mut file = box_of(b"ftyp", b"isom\0\0\0\0isomiso2");
    file.extend_from_slice(&box_of(b"moov", &moov));
    file
}

// ---------------------------------------------------------------------------
// MKV builders
// ---------------------------------------------------------------------------

/// An EBML element with a one-byte ID and a one-byte size.
fn el(id: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = vec![id, 0x80 | (payload.len() as u8)];
    out.extend_from_slice(payload);
    out
}

/// An EBML element with a four-byte ID and a four-byte size.
fn el4(id: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&id.to_be_bytes());
    // 0x10000000 marks a four-byte length; the low 28 bits carry the value.
    out.extend_from_slice(&(0x1000_0000u32 | payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A Matroska file: EBML header, then a Segment holding Tracks.
fn mkv(video: Option<&str>, audio: Option<&str>, w: u64, h: u64) -> Vec<u8> {
    let mut tracks = Vec::new();
    if let Some(codec) = video {
        let mut entry = el(0x86, codec.as_bytes()); // CodecID
        let mut vid = el(0xB0, &[w as u8]); // PixelWidth
        vid.extend_from_slice(&el(0xBA, &[h as u8])); // PixelHeight
        entry.extend_from_slice(&el(0xE0, &vid)); // Video
        tracks.extend_from_slice(&el(0xAE, &entry)); // TrackEntry
    }
    if let Some(codec) = audio {
        tracks.extend_from_slice(&el(0xAE, &el(0x86, codec.as_bytes())));
    }
    let mut file = el4(0x1A45_DFA3, b"");
    file.extend_from_slice(&el4(0x1853_8067, &el4(0x1654_AE6B, &tracks)));
    file
}

// ---------------------------------------------------------------------------
// The flagship
// ---------------------------------------------------------------------------

/// **`MKV` with H.264 + AAC reads as remuxable into MP4.**
///
/// `01` §154's promise, from the file's actual bytes. Until this reader
/// existed, `Properties::Video` was never constructed and the Class A route
/// could not be selected by anything.
#[test]
fn an_h264_aac_matroska_reports_both_codecs() {
    let s = read_streams(
        FormatId::Mkv,
        &mkv(Some("V_MPEG4/ISO/AVC"), Some("A_AAC"), 64, 48),
    )
    .expect("mkv is a container this parses");
    assert_eq!(s.video, Some(VideoCodec::H264));
    assert_eq!(s.audio, Some(AudioCodec::Aac));
    assert_eq!((s.width, s.height), (64, 48));
    assert!(openconvert_core::codec::streams_carryable(
        FormatId::Mp4,
        s.video,
        s.audio
    ));
}

/// The same container with VP9 + Opus is **not** remuxable into MP4.
#[test]
fn a_vp9_opus_matroska_is_not_carryable_into_mp4() {
    let s = read_streams(FormatId::Mkv, &mkv(Some("V_VP9"), Some("A_OPUS"), 64, 48)).unwrap();
    assert_eq!(s.video, Some(VideoCodec::Vp9));
    assert_eq!(s.audio, Some(AudioCodec::Opus));
    assert!(!openconvert_core::codec::streams_carryable(
        FormatId::Mp4,
        s.video,
        s.audio
    ));
    // ...and is carryable into WebM, which is the control that proves the
    // refusal above is about the destination and not about the file.
    assert!(openconvert_core::codec::streams_carryable(
        FormatId::Webm,
        s.video,
        s.audio
    ));
}

/// MP4 sample descriptions are read, and an audio track does not erase the
/// video track's dimensions.
#[test]
fn an_mp4_reports_its_codecs_and_keeps_the_video_dimensions() {
    let s = read_streams(
        FormatId::Mp4,
        &mp4(Some(b"avc1"), Some(b"mp4a"), 1920, 1080),
    )
    .unwrap();
    assert_eq!(s.video, Some(VideoCodec::H264));
    assert_eq!(s.audio, Some(AudioCodec::Aac));
    assert_eq!(
        (s.width, s.height),
        (1920, 1080),
        "the audio track's 0x0 tkhd overwrote the video track's"
    );
}

/// An AAC profile string is still AAC.
///
/// Real muxers write `A_AAC/MPEG4/LC`, not `A_AAC`. An exact-match table reads
/// that as an unidentified codec and refuses a remux that is perfectly legal —
/// the failure being a silent extra re-encode, which nobody files a bug about.
#[test]
fn an_aac_profile_string_is_recognised() {
    let s = read_streams(
        FormatId::Mkv,
        &mkv(Some("V_MPEG4/ISO/AVC"), Some("A_AAC/MPEG4/LC"), 8, 8),
    )
    .unwrap();
    assert_eq!(s.audio, Some(AudioCodec::Aac));
}

/// A codec we do not know is `Other`, and `Other` is never carryable.
#[test]
fn an_unknown_codec_id_becomes_other_and_blocks_the_remux() {
    let s = read_streams(FormatId::Mkv, &mkv(Some("V_SOMETHING_NEW"), None, 8, 8)).unwrap();
    assert_eq!(s.video, Some(VideoCodec::Other));
    assert!(!openconvert_core::codec::streams_carryable(
        FormatId::Mkv,
        s.video,
        s.audio
    ));
}

/// A format this does not parse is `None`, not an empty answer.
///
/// "We did not look" and "we looked and found nothing" are different claims,
/// and only the second is evidence.
#[test]
fn a_non_container_format_is_not_parsed_at_all() {
    assert!(read_streams(FormatId::Png, &[1, 2, 3]).is_none());
    assert!(read_streams(FormatId::Mp4, &[]).is_some());
}

// ---------------------------------------------------------------------------
// Hostile input. This parser runs in the host.
// ---------------------------------------------------------------------------

/// **A box that declares zero size does not hang the reader.**
///
/// The classic container parser bug: a cursor advanced by a length the file
/// chose, where the file chose zero. A `size < header` check catches it, and
/// this is the test that proves the check is load-bearing.
#[test]
fn a_zero_length_box_terminates_rather_than_looping() {
    let mut evil = Vec::new();
    evil.extend_from_slice(&0u32.to_be_bytes()); // size 0 -- "to the end"
    evil.extend_from_slice(b"free");
    evil.extend_from_slice(&[0u8; 16]);
    // A size of 4 is smaller than the 8-byte header: not advanceable.
    evil.extend_from_slice(&4u32.to_be_bytes());
    evil.extend_from_slice(b"junk");
    let _ = read_streams(FormatId::Mp4, &evil);
}

/// The same, in EBML: every element declares zero payload.
#[test]
fn zero_length_ebml_elements_terminate() {
    // 1000 elements, each a one-byte ID and a one-byte zero size.
    let evil: Vec<u8> = (0..1000).flat_map(|_| [0xEC, 0x80]).collect();
    let _ = read_streams(FormatId::Mkv, &evil);
}

/// **A box nested inside itself does not overflow the stack.**
///
/// Built by wrapping a `moov` in a `moov` two hundred times — well past the
/// depth bound, and the shape a stack-overflow crash is written as.
#[test]
fn deep_nesting_is_bounded_rather_than_recursed() {
    let mut inner = box_of(b"free", &[0u8; 4]);
    for _ in 0..200 {
        inner = box_of(b"moov", &inner);
    }
    let s = read_streams(FormatId::Mp4, &inner).unwrap();
    assert_eq!(s.video, None, "nothing was found, and nothing crashed");
}

/// The same for EBML, whose Segment is the element that nests.
#[test]
fn deep_ebml_nesting_is_bounded() {
    let mut inner = el(0x86, b"V_VP9");
    for _ in 0..200 {
        inner = el4(0x1853_8067, &inner);
    }
    let _ = read_streams(FormatId::Mkv, &inner);
}

/// **A declared size larger than the file does not slice past the end.**
///
/// The allocation-from-a-number class, in its container form. A `u32::MAX` box
/// size is a correct decode and a fatal read.
#[test]
fn a_box_larger_than_the_file_is_refused() {
    let mut evil = Vec::new();
    evil.extend_from_slice(&u32::MAX.to_be_bytes());
    evil.extend_from_slice(b"moov");
    evil.extend_from_slice(&[0u8; 32]);
    let s = read_streams(FormatId::Mp4, &evil).unwrap();
    assert_eq!(s.video, None);

    // And the 64-bit form, which is the one that reaches past usize on a
    // 32-bit target.
    let mut evil64 = Vec::new();
    evil64.extend_from_slice(&1u32.to_be_bytes());
    evil64.extend_from_slice(b"moov");
    evil64.extend_from_slice(&u64::MAX.to_be_bytes());
    evil64.extend_from_slice(&[0u8; 32]);
    assert_eq!(read_streams(FormatId::Mp4, &evil64).unwrap().video, None);
}

/// An EBML element declaring more than the file holds is refused.
#[test]
fn an_ebml_element_larger_than_the_file_is_refused() {
    let mut evil = Vec::new();
    evil.extend_from_slice(&0x1853_8067u32.to_be_bytes());
    evil.extend_from_slice(&(0x1000_0000u32 | 0x0FFF_FFFF).to_be_bytes());
    evil.extend_from_slice(&[0u8; 8]);
    let s = read_streams(FormatId::Mkv, &evil).unwrap();
    assert_eq!(s.video, None);
}

/// A flat file of a hundred thousand tiny boxes is bounded by the element
/// budget, not by depth.
///
/// The depth limit alone permits this: it nests one level and costs a million
/// iterations. Two different bounds because there are two different attacks.
#[test]
fn a_flat_file_of_many_boxes_is_bounded_by_the_element_budget() {
    let mut evil = Vec::new();
    for _ in 0..200_000 {
        evil.extend_from_slice(&8u32.to_be_bytes());
        evil.extend_from_slice(b"free");
    }
    let s = read_streams(FormatId::Mp4, &evil).unwrap();
    assert_eq!(s.video, None);
}

/// Truncation at every length is survivable.
///
/// A real file cut short at an arbitrary byte is the commonest malformed input
/// there is — an interrupted download — and every prefix must terminate.
#[test]
fn every_truncation_of_a_valid_file_terminates() {
    for file in [
        mp4(Some(b"avc1"), Some(b"mp4a"), 320, 240),
        mkv(Some("V_MPEG4/ISO/AVC"), Some("A_AAC"), 32, 24),
    ] {
        for cut in 0..file.len() {
            let format = if file[0] == 0x1A {
                FormatId::Mkv
            } else {
                FormatId::Mp4
            };
            let _ = read_streams(format, &file[..cut]);
        }
    }
}
