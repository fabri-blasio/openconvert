//! Container headers: which codecs are inside, read without decoding anything.
//!
//! # What this exists for
//!
//! `core::codec` decides whether a remux is lossless, and it needs codecs to
//! decide from. Nothing produced them, so `Properties::Video` was never built
//! and `01` §154's flagship — `MKV → MP4`, bit-identical — was a route the
//! table could describe and the product could never select.
//!
//! This is a **header reader, not a demuxer**. It walks to the track list, reads
//! the codec identifiers and the pixel dimensions, and stops. No frame is
//! touched, no sample table is walked, and nothing here could produce output.
//!
//! # Why it is safe to run in the host
//!
//! Pure Rust over a byte slice, with no allocation driven by any number the
//! file supplies, bounded recursion and a bounded element count. That is the
//! standard `sniff` already meets, and it is why `02` §3.1 puts Class A video in
//! core rather than behind the FFmpeg module.
//!
//! Three bounds, because a container is a nested structure an attacker writes:
//!
//! - **Depth** — a box that contains itself is a stack overflow otherwise.
//! - **Element count** — the depth bound alone permits a flat file of a million
//!   empty boxes, which is a denial of service with no nesting at all.
//! - **A non-advancing cursor terminates the scan.** An element declaring zero
//!   size advances the cursor by zero, and a loop that trusts it never ends.
//!   That is the bug that turns a parser into a hang, and no legitimate file
//!   reaches it.
//!
//! Nothing here returns a partial answer as a whole one: an unrecognised codec
//! is [`VideoCodec::Other`], which `core::codec` refuses to call carryable.

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::format::FormatId;

/// How deep a container may nest before we stop believing it.
///
/// Real files reach about six: `moov` → `trak` → `mdia` → `minf` → `stbl` →
/// `stsd` → the sample entry. Sixteen is generous and still finite.
const MAX_DEPTH: u8 = 16;

/// How many elements the whole scan may visit.
///
/// The depth bound says nothing about breadth. A file of a million zero-payload
/// boxes nests one level deep and costs a million iterations.
const MAX_ELEMENTS: u32 = 100_000;

/// What a container header claims about its streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Streams {
    /// The video codec, if a video track was found.
    pub video: Option<VideoCodec>,
    /// The audio codec, if an audio track was found.
    pub audio: Option<AudioCodec>,
    /// Pixel width, or 0 for "not read".
    ///
    /// Zero is not a legal width, so it is an unambiguous sentinel — the same
    /// convention `Response::Properties::frames` uses. A container whose track
    /// header we did not reach reports 0 rather than a guess.
    pub width: u32,
    /// Pixel height, or 0 for "not read".
    pub height: u32,
}

/// Read the stream list from a container header.
///
/// `None` means "not a container this parses". `Some(Streams::default())` means
/// "parsed it and found nothing". Only the second is evidence, and collapsing
/// them would let a format we never look at masquerade as one we examined.
#[must_use]
pub fn read_streams(format: FormatId, bytes: &[u8]) -> Option<Streams> {
    match format {
        // One reader for both: a `.mov` is the same ISO base media container
        // under a different `ftyp` brand, and `read_mp4` walks boxes rather
        // than brands.
        FormatId::Mp4 | FormatId::Mov => Some(read_mp4(bytes)),
        // RIFF, not ISO base media, so it gets its own walk. The question
        // asked of it is the same one: which codecs are in here, so
        // `StreamsCompatible` can answer before anything runs.
        FormatId::Avi => Some(read_avi(bytes)),
        FormatId::Mkv | FormatId::Webm => Some(read_mkv(bytes)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// RIFF (AVI)
// ---------------------------------------------------------------------------

/// The codecs and geometry an AVI declares, from its `hdrl` list.
///
/// A HEADER READ, like every other function here: it walks the stream headers
/// and stops, so `StreamsCompatible` can be answered before a byte of picture
/// data is touched. The full walk lives in `avidemux`, where it is needed.
///
/// The fourcc mapping is deliberately the same set `avidemux::carry` accepts.
/// A probe that recognised more than the demuxer carries would let `route()`
/// plan a conversion the engine then refuses — the exact shape of defect the
/// declared-routes gate exists for.
fn read_avi(bytes: &[u8]) -> Streams {
    let mut out = Streams::default();
    // `hdrl` sits near the front, before any sample data, so a bounded window
    // covers it on every real file and costs nothing on a large one.
    let window = &bytes[..bytes.len().min(1 << 16)];
    let Some(hdrl) = find(window, b"hdrl") else {
        return out;
    };

    let mut at = hdrl + 4;
    while at + 8 <= window.len() {
        let Some(id) = window.get(at..at + 4) else {
            break;
        };
        let Some(size) = window
            .get(at + 4..at + 8)
            .and_then(|s| s.try_into().ok())
            .map(u32::from_le_bytes)
            .map(|s| s as usize)
        else {
            break;
        };
        let body = at + 8;
        if id == b"LIST" {
            // Descend: `strl` lists hold the headers this is looking for.
            at = body + 4;
            continue;
        }
        if id == b"strf" {
            if let Some(f) = window.get(body..body + size.min(64)) {
                classify_avi_format(f, &mut out);
            }
        }
        at = body + size + (size & 1);
    }
    out
}

/// One `strf` payload: a `BITMAPINFOHEADER` if it is long enough to be one and
/// its fourcc is a codec we carry, a `WAVEFORMATEX` otherwise.
fn classify_avi_format(f: &[u8], out: &mut Streams) {
    // A BITMAPINFOHEADER is 40 bytes and declares its own size first; a
    // WAVEFORMATEX is 16 or 18 and starts with a format tag. The declared size
    // is what separates them without guessing from length alone.
    let declared = f
        .get(..4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .unwrap_or(0);
    if declared >= 40 && f.len() >= 20 {
        let mut fourcc: [u8; 4] = f[16..20].try_into().unwrap_or([0; 4]);
        fourcc.make_ascii_uppercase();
        if matches!(&fourcc, b"H264" | b"X264" | b"AVC1" | b"DAVC" | b"VSSH") {
            out.video = Some(VideoCodec::H264);
            out.width = u32::from_le_bytes(f[4..8].try_into().unwrap_or([0; 4]));
            #[allow(clippy::cast_possible_wrap)]
            {
                out.height = (u32::from_le_bytes(f[8..12].try_into().unwrap_or([0; 4])) as i32)
                    .unsigned_abs();
            }
        }
        return;
    }
    // 0x0055 is MPEG Layer 3, the one audio format an AVI carries that MP4 and
    // Matroska both hold.
    if f.len() >= 2 && u16::from_le_bytes(f[..2].try_into().unwrap_or([0; 2])) == 0x0055 {
        out.audio = Some(AudioCodec::Mp3);
    }
}

/// First index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// An element budget shared across one whole walk.
struct Budget {
    elements: u32,
}

impl Budget {
    /// Spend one element. `false` once the budget is gone.
    fn spend(&mut self) -> bool {
        if self.elements == 0 {
            return false;
        }
        self.elements -= 1;
        true
    }
}

// ---------------------------------------------------------------------------
// ISO base media format (MP4)
// ---------------------------------------------------------------------------

fn read_mp4(bytes: &[u8]) -> Streams {
    let mut out = Streams::default();
    let mut budget = Budget {
        elements: MAX_ELEMENTS,
    };
    walk_mp4(bytes, 0, &mut budget, &mut out);
    out
}

/// Box types whose payload is more boxes.
///
/// A whitelist rather than a guess: descending into a box we do not recognise
/// would mean reading a codec's private data as though it were structure.
const fn mp4_is_container(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"mvex"
    )
}

fn walk_mp4(bytes: &[u8], depth: u8, budget: &mut Budget, out: &mut Streams) {
    if depth >= MAX_DEPTH {
        return;
    }
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        if !budget.spend() {
            return;
        }
        let size32 = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&bytes[at + 4..at + 8]);

        // 1 means a 64-bit size follows the type; 0 means "to the end".
        let (header, size) = match size32 {
            1 => {
                if at + 16 > bytes.len() {
                    return;
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&bytes[at + 8..at + 16]);
                (16usize, u64::from_be_bytes(n))
            }
            0 => (8usize, (bytes.len() - at) as u64),
            n => (8usize, u64::from(n)),
        };

        // A box smaller than its own header, or larger than what is left, is a
        // lie. Stop rather than clamp: clamping turns a malformed file into a
        // differently-malformed one and keeps reading it.
        let Ok(size) = usize::try_from(size) else {
            return;
        };
        if size < header || at + size > bytes.len() {
            return;
        }

        let payload = &bytes[at + header..at + size];
        if mp4_is_container(&kind) {
            walk_mp4(payload, depth + 1, budget, out);
        } else if &kind == b"stsd" {
            read_stsd(payload, out);
        } else if &kind == b"tkhd" {
            read_tkhd(payload, out);
        }

        at += size;
    }
}

/// Sample descriptions: the four-character codes naming each track's codec.
fn read_stsd(payload: &[u8], out: &mut Streams) {
    // version+flags (4), entry_count (4), then the entries.
    if payload.len() < 16 {
        return;
    }
    let mut at = 8usize;
    while at + 8 <= payload.len() {
        let size = u32::from_be_bytes([
            payload[at],
            payload[at + 1],
            payload[at + 2],
            payload[at + 3],
        ]) as usize;
        let mut code = [0u8; 4];
        code.copy_from_slice(&payload[at + 4..at + 8]);

        if let Some(v) = video_from_fourcc(&code) {
            out.video.get_or_insert(v);
        } else if let Some(a) = audio_from_fourcc(&code) {
            out.audio.get_or_insert(a);
        }

        // A zero-size entry would loop forever; a size past the end is a lie.
        if size < 8 || at + size > payload.len() {
            return;
        }
        at += size;
    }
}

/// Track header: pixel dimensions, as 16.16 fixed point.
fn read_tkhd(payload: &[u8], out: &mut Streams) {
    if payload.is_empty() {
        return;
    }
    // The version byte selects two published layouts. A payload too short for
    // the version it declares is not parsed at all.
    let (need, at) = match payload[0] {
        0 => (84usize, 76usize),
        1 => (96usize, 88usize),
        _ => return,
    };
    if payload.len() < need {
        return;
    }
    let fixed = |o: usize| -> u32 {
        u32::from_be_bytes([payload[o], payload[o + 1], payload[o + 2], payload[o + 3]]) >> 16
    };
    let (w, h) = (fixed(at), fixed(at + 4));
    // An audio track's tkhd carries 0x0, so the first non-zero pair belongs to
    // the video track. Taking the last would let a trailing audio track erase
    // the dimensions.
    if w > 0 && h > 0 && out.width == 0 {
        out.width = w;
        out.height = h;
    }
}

const fn video_from_fourcc(code: &[u8; 4]) -> Option<VideoCodec> {
    match code {
        b"avc1" | b"avc3" => Some(VideoCodec::H264),
        b"hvc1" | b"hev1" => Some(VideoCodec::H265),
        b"av01" => Some(VideoCodec::Av1),
        b"vp08" => Some(VideoCodec::Vp8),
        b"vp09" => Some(VideoCodec::Vp9),
        b"apch" | b"apcn" | b"apcs" | b"apco" | b"ap4h" => Some(VideoCodec::ProRes),
        _ => None,
    }
}

const fn audio_from_fourcc(code: &[u8; 4]) -> Option<AudioCodec> {
    match code {
        // `mp4a` is an MPEG-4 audio object and is overwhelmingly AAC.
        // Distinguishing AAC from MP3-in-MP4 needs the `esds` object-type byte,
        // which this does not read. The imprecision is bounded and recorded
        // rather than hidden: MP4 carries both losslessly, so the remux
        // decision is identical either way.
        b"mp4a" => Some(AudioCodec::Aac),
        b"alac" => Some(AudioCodec::Alac),
        b"Opus" => Some(AudioCodec::Opus),
        b"fLaC" => Some(AudioCodec::Flac),
        b".mp3" | b"mp3 " => Some(AudioCodec::Mp3),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// EBML / Matroska (MKV, WebM)
// ---------------------------------------------------------------------------

/// Element IDs, with their marker bits, as they appear on the wire.
const ID_EBML_HEADER: u64 = 0x1A45_DFA3;
const ID_SEGMENT: u64 = 0x1853_8067;
const ID_TRACKS: u64 = 0x1654_AE6B;
const ID_TRACK_ENTRY: u64 = 0xAE;
const ID_CODEC_ID: u64 = 0x86;
const ID_VIDEO: u64 = 0xE0;
const ID_PIXEL_WIDTH: u64 = 0xB0;
const ID_PIXEL_HEIGHT: u64 = 0xBA;

fn read_mkv(bytes: &[u8]) -> Streams {
    let mut out = Streams::default();
    let mut budget = Budget {
        elements: MAX_ELEMENTS,
    };
    walk_mkv(bytes, 0, &mut budget, &mut out);
    out
}

/// Read one EBML variable-length integer.
///
/// Returns `(value, bytes_consumed)`. `keep_marker` distinguishes an **ID**,
/// where the leading marker bit is part of the identity, from a **size**, where
/// it is length metadata and must be stripped. Using one for the other is the
/// classic EBML bug and produces IDs that match nothing.
fn vint(bytes: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    if first == 0 {
        // Eight leading zero bits means a length this does not read, and no
        // real muxer emits one.
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || bytes.len() < len {
        return None;
    }
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        // Wide mask: an 8-byte vint shifts by 8, which overflows a u8 —
        // the exact case every long-form size hits.
        let mask = if len >= 8 {
            0u8
        } else {
            (0xFF_u16 >> len) as u8
        };
        u64::from(first & mask)
    };
    for &b in &bytes[1..len] {
        value = (value << 8) | u64::from(b);
    }
    Some((value, len))
}

/// Whether an element's payload is more elements.
const fn mkv_is_container(id: u64) -> bool {
    matches!(
        id,
        ID_EBML_HEADER | ID_SEGMENT | ID_TRACKS | ID_TRACK_ENTRY | ID_VIDEO
    )
}

fn walk_mkv(bytes: &[u8], depth: u8, budget: &mut Budget, out: &mut Streams) {
    if depth >= MAX_DEPTH {
        return;
    }
    let mut at = 0usize;
    while at < bytes.len() {
        if !budget.spend() {
            return;
        }
        let Some((id, id_len)) = vint(&bytes[at..], true) else {
            return;
        };
        let Some((size, size_len)) = vint(&bytes[at + id_len..], false) else {
            return;
        };
        let header = id_len + size_len;
        let Ok(size) = usize::try_from(size) else {
            return;
        };
        // Checked with the header included, so an element claiming the whole
        // remainder plus one byte stops the scan instead of slicing past it.
        if at + header + size > bytes.len() {
            return;
        }
        let payload = &bytes[at + header..at + header + size];

        if mkv_is_container(id) {
            walk_mkv(payload, depth + 1, budget, out);
        } else if id == ID_CODEC_ID {
            classify_mkv_codec(payload, out);
        } else if id == ID_PIXEL_WIDTH && out.width == 0 {
            out.width = u32::try_from(uint_be(payload)).unwrap_or(0);
        } else if id == ID_PIXEL_HEIGHT && out.height == 0 {
            out.height = u32::try_from(uint_be(payload)).unwrap_or(0);
        }

        // An ID is at least one byte, so `header` is never zero and the cursor
        // always advances. That is what makes this loop terminate on any input,
        // including one whose every element declares zero size.
        at += header + size;
    }
}

/// An EBML unsigned integer: big-endian, one to eight bytes, length implied.
fn uint_be(payload: &[u8]) -> u64 {
    if payload.is_empty() || payload.len() > 8 {
        return 0;
    }
    payload
        .iter()
        .fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

/// Map a Matroska CodecID string onto the closed codec enums.
fn classify_mkv_codec(payload: &[u8], out: &mut Streams) {
    let Ok(id) = core::str::from_utf8(payload) else {
        return;
    };
    // Trailing NULs are legal padding in EBML strings.
    let id = id.trim_end_matches('\u{0}');

    if let Some(rest) = id.strip_prefix("V_") {
        let codec = match rest {
            "MPEG4/ISO/AVC" => VideoCodec::H264,
            "MPEGH/ISO/HEVC" => VideoCodec::H265,
            "AV1" => VideoCodec::Av1,
            "VP8" => VideoCodec::Vp8,
            "VP9" => VideoCodec::Vp9,
            "PRORES" => VideoCodec::ProRes,
            // Identified as video and no further. `core::codec` refuses to call
            // this carryable, which is the safe direction.
            _ => VideoCodec::Other,
        };
        out.video.get_or_insert(codec);
    } else if let Some(rest) = id.strip_prefix("A_") {
        let codec = match rest {
            "AAC" => AudioCodec::Aac,
            "MPEG/L3" => AudioCodec::Mp3,
            "OPUS" => AudioCodec::Opus,
            "VORBIS" => AudioCodec::Vorbis,
            "FLAC" => AudioCodec::Flac,
            "ALAC" => AudioCodec::Alac,
            // AAC profiles are written "A_AAC/MPEG4/LC" and similar.
            _ if rest.starts_with("AAC/") => AudioCodec::Aac,
            _ if rest.starts_with("PCM/") => AudioCodec::Pcm,
            _ => AudioCodec::Other,
        };
        out.audio.get_or_insert(codec);
    }
}
