//! Matroska/WebM: a bounded demuxer into the stream graph, and EBML surgery
//! for the EBML-to-EBML remuxes.
//!
//! # What this is, and refuses to be
//!
//! A **container surgeon**, not a codec. Every byte of every coded sample
//! travels through untouched â€” that is what makes these conversions Class A,
//! and it is why this file never looks inside a block payload.
//!
//! Two operations live here:
//!
//! - [`demux`] â€” parse a Matroska/WebM file into an [`AvGraph`] so the MP4
//!   muxer can rebuild it.
//! - [`ebml_to_ebml`] â€” WebM â‡„ MKV and MKV â†’ MKA by **segment surgery**: the
//!   original elements splice through verbatim, so even timestamps' BYTES
//!   survive; the output differs only where the operation demands.
//!
//! # The bounds, which are the security model
//!
//! Attacker-written nesting with attacker-written sizes, parsed in the host
//! process. Same discipline as the header reader in `container.rs`, plus a
//! retained-byte budget a full demux needs: depth ceiling, element-count
//! ceiling, an always-advancing cursor, size lie-detection before any slice,
//! and every copied byte counted against `memory_bytes`.
//!
//! Laced blocks are REFUSED rather than implemented: lacing appears almost
//! solely in legacy Vorbis tracks, and an implementation that exists to pass
//! one fixture is a liability, not a feature.

use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::format::FormatId;

/// How deep elements may nest â€” the header reader's own ceiling.
const MAX_DEPTH: u8 = 16;

/// How many elements one walk may visit in total.
const MAX_ELEMENTS: u32 = 100_000;

/// Why demuxing failed.
#[derive(Debug, thiserror::Error)]
pub enum DemuxError {
    /// Structure violated one of the walk's bounds.
    #[error("this Matroska/WebM stream exceeds a safety bound ({0})")]
    Bounded(&'static str),
    /// Structure was malformed where the conversion needs it.
    #[error("this Matroska/WebM stream is malformed ({0})")]
    Malformed(String),
    /// Laced blocks exist, and this build refuses them rather than guessing.
    #[error("this stream uses block lacing, which this build does not carry")]
    Laced,
    /// Nothing convertible was found.
    #[error("no convertible track was found in this stream")]
    NoTracks,
}

// ---------------------------------------------------------------------------
// The stream graph
// ---------------------------------------------------------------------------

/// Which kind of track a `TrackEntry` describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    /// Video.
    Video,
    /// Audio.
    Audio,
    /// Subtitles, buttons, anything else: carried by EBML surgery, refused by
    /// the MP4 path.
    Other,
}

/// One demuxed track.
#[derive(Debug)]
pub struct AvTrack {
    /// The Matroska track number blocks reference.
    pub number: u64,
    /// What kind of track it is.
    pub kind: TrackKind,
    /// The closed-enum codec when identified; `None` = unidentified, which
    /// the codec table treats as uncarryable everywhere, forever.
    pub video: Option<VideoCodec>,
    /// As above, for audio.
    pub audio: Option<AudioCodec>,
    /// The CodecID element payload verbatim. The enum narrows it for routing
    /// decisions; serialising back to EBML writes THESE bytes, because a
    /// round trip that re-derives a canonical string from the enum would
    /// quietly rewrite e.g. `A_AAC/MPEG4/LC` into `A_AAC`.
    pub codec_id: Vec<u8>,
    /// CodecPrivate verbatim (`avcC`, AudioSpecificConfig, â€¦).
    pub codec_private: Vec<u8>,
    /// Pixel width, 0 when unread.
    pub pixel_width: u32,
    /// Pixel height, 0 when unread.
    pub pixel_height: u32,
    /// SamplingFrequency from the Audio element; 0.0 when unread.
    ///
    /// Carried so a rewrap can put it back. Matroska gives it a default of
    /// 8000 Hz, which means an audio track that omits it does not read as
    /// "unspecified" downstream -- it reads as 8 kHz, and the file plays at
    /// the wrong speed.
    pub sample_rate: f64,
    /// Channels from the Audio element; 0 when unread (Matroska defaults 1).
    pub channels: u64,
    /// BitDepth from the Audio element; 0 when unread.
    pub bit_depth: u64,
    /// Coded samples, source order preserved.
    pub samples: Vec<AvSample>,
}

/// One coded sample (one Access Unit), byte-identical to its source block.
#[derive(Debug)]
pub struct AvSample {
    /// **Decode time**, in the graph's ticks. Never converted here:
    /// conversion is where timestamps get rounded.
    ///
    /// From Matroska this is the block's own timestamp, which that format
    /// defines as the PRESENTATION time -- see `composition_offset_ticks` for
    /// why the two are the same value there and not in general.
    pub ts_ticks: i64,
    /// Presentation time minus decode time.
    ///
    /// # The defect this exists for
    ///
    /// H.264 with B-frames is stored in DECODE order and shown in a different
    /// one. MP4 says so with two tables -- `stts` for decode deltas and `ctts`
    /// for the offset to presentation -- and this graph carried one number, so
    /// `ctts` was read by nothing and written by nothing.
    ///
    /// What that produced: `mov -> mp4` on a 277-frame export came back with
    /// its frames timed `0, .066, .100, .033, .166` where the source read
    /// `0, .033, .066, .100`. Every frame was present and byte-identical, in
    /// the wrong order, under a Class A receipt.
    ///
    /// Zero for every sample means decode order IS presentation order, which
    /// is true of all audio and of video with no B-frames. Matroska stores
    /// presentation times and states no decode time, so a graph demuxed from
    /// it leaves this zero: that is the honest answer rather than a guess, and
    /// it is why `mkv -> mp4` on B-frame content is still approximate.
    pub composition_offset_ticks: i64,
    /// Whether the source marked it a keyframe.
    pub keyframe: bool,
    /// The coded payload, verbatim.
    pub data: Vec<u8>,
}

/// The whole demuxed file.
#[derive(Debug)]
pub struct AvGraph {
    /// Tracks, in TrackEntry order.
    pub tracks: Vec<AvTrack>,
    /// Segment TimecodeScale (ns per tick); spec default 1,000,000.
    pub timecode_scale_ns: u32,
}

impl AvGraph {
    /// The video track, if any.
    #[must_use]
    pub fn video(&self) -> Option<&AvTrack> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Video)
    }

    /// The first audio track, if any.
    #[must_use]
    pub fn audio(&self) -> Option<&AvTrack> {
        self.tracks.iter().find(|t| t.kind == TrackKind::Audio)
    }
}

// ---------------------------------------------------------------------------
// EBML reader
// ---------------------------------------------------------------------------

/// Read one vint. ID form keeps marker bits; size form strips them.
fn vint(b: &[u8], keep_marker: bool) -> Option<(u64, usize)> {
    let first = *b.first()?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || b.len() < len {
        return None;
    }
    // The strip mask is computed wide because an 8-byte vint shifts by 8,
    // which overflows a u8 â€” the exact case every long-form size hits.
    let mut value = if keep_marker {
        u64::from(first)
    } else {
        let mask = if len >= 8 { 0 } else { (0xFF_u64 >> len) as u8 };
        u64::from(first & mask)
    };
    for &byte in &b[1..len] {
        value = (value << 8) | u64::from(byte);
    }
    Some((value, len))
}

/// Element IDs, next to the reader that consumes them.
mod id {
    pub const SEGMENT: u64 = 0x1853_8067;
    pub const INFO: u64 = 0x1549_A966;
    pub const TIMECODE_SCALE: u64 = 0x002A_D7B1;
    pub const CLUSTER: u64 = 0x1F43_B675;
    pub const CLUSTER_TIMECODE: u64 = 0xE7;
    pub const SIMPLE_BLOCK: u64 = 0xA3;
    pub const BLOCK_GROUP: u64 = 0xA0;
    pub const BLOCK: u64 = 0xA1;
    pub const TRACKS: u64 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u64 = 0xAE;
    pub const TRACK_NUMBER: u64 = 0xD7;
    pub const TRACK_TYPE: u64 = 0x83;
    pub const CODEC_ID: u64 = 0x86;
    pub const CODEC_PRIVATE: u64 = 0x63A2;
    /// The Video master element inside a TrackEntry, and the geometry that
    /// lives inside it.
    ///
    /// PixelWidth and PixelHeight are NOT TrackEntry children. They were
    /// written as if they were, and ffmpeg refused the whole file: "Unknown
    /// entry 0xB0". Every `mp4 -> mkv` this build produced was undecodable by
    /// anything but itself, and the reader below could not see the geometry in
    /// a real Matroska file either, because a real muxer nests them here.
    ///
    /// Exactly the mistake the `AUDIO` comment records, one element over.
    pub const VIDEO: u64 = 0xE0;
    pub const PIXEL_WIDTH: u64 = 0xB0;
    pub const PIXEL_HEIGHT: u64 = 0xBA;
    /// The Audio master element inside a TrackEntry, and the two children
    /// every real muxer writes. Omitting these is what made our own `.mka`
    /// output undecodable by symphonia -- see `write_tracks`.
    pub const AUDIO: u64 = 0xE1;
    pub const SAMPLING_FREQUENCY: u64 = 0xB5;
    pub const CHANNELS: u64 = 0x9F;
    /// BitDepth. Optional in general, REQUIRED in practice for the raw-PCM
    /// CodecIDs, whose sample width is not recoverable from anything else.
    pub const BIT_DEPTH: u64 = 0x6264;
}

/// A cursor over one element payload, carrying the walk-wide budgets.
struct Reader<'a, 'e> {
    b: &'a [u8],
    pos: usize,
    elements: &'e mut u32,
    depth: u8,
    /// Start offset of the last returned element (its id byte), so callers
    /// re-emit `[elem_start, pos)` verbatim without recomputing headers.
    elem_start: usize,
}

impl<'a, 'e> Reader<'a, 'e> {
    fn top(b: &'a [u8], elements: &'e mut u32) -> Self {
        Self {
            b,
            pos: 0,
            elements,
            depth: 0,
            elem_start: 0,
        }
    }

    fn child(&mut self, payload: &'a [u8]) -> Reader<'a, '_> {
        Reader {
            b: payload,
            pos: 0,
            elements: self.elements,
            depth: self.depth + 1,
            elem_start: 0,
        }
    }

    fn spend(&mut self) -> Result<(), DemuxError> {
        if *self.elements == 0 {
            return Err(DemuxError::Bounded("element count"));
        }
        *self.elements -= 1;
        Ok(())
    }

    /// Next `(id, payload)`. Enforces the depth ceiling, lie-detects declared
    /// sizes, refuses unknown-size elements (files have honest ends), and
    /// always advances the cursor â€” id_len â‰¥ 1 guarantees termination on any
    /// input, including one whose every element declares zero size.
    fn next_element(&mut self) -> Result<Option<(u64, &'a [u8])>, DemuxError> {
        self.spend()?;
        if self.depth > MAX_DEPTH || self.pos >= self.b.len() {
            return Ok(None);
        }
        self.elem_start = self.pos;
        let Some((eid, id_len)) = vint(&self.b[self.pos..], true) else {
            let near: &[u8] = &self.b[self.pos.min(self.b.len())..self.b.len().min(self.pos + 12)];
            return Err(DemuxError::Malformed(format!(
                "element id at pos {} depth {} near {near:02x?}",
                self.pos, self.depth
            )));
        };
        let Some((size, size_len)) = vint(&self.b[self.pos + id_len..], false) else {
            return Err(DemuxError::Malformed("element size".to_string()));
        };
        if size_len < 8 && size == (1u64 << (7 * size_len)) - 1 {
            return Err(DemuxError::Malformed("unknown-size element".to_string()));
        }
        let Ok(size) = usize::try_from(size) else {
            return Err(DemuxError::Malformed("element size overflow".to_string()));
        };
        let header = id_len + size_len;
        if self.pos + header + size > self.b.len() {
            return Err(DemuxError::Malformed(
                "element larger than its parent".to_string(),
            ));
        }
        let payload = &self.b[self.pos + header..self.pos + header + size];
        self.pos += header + size;
        Ok(Some((eid, payload)))
    }

    fn uint(payload: &[u8]) -> u64 {
        payload
            .iter()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
    }

    /// An EBML float: big-endian IEEE 754, 4 or 8 bytes.
    ///
    /// Any other length is 0.0 rather than a guess. EBML permits a zero-length
    /// float meaning "use the default", and a 3- or 7-byte one is a malformed
    /// file -- both answer "the source did not tell us", which is exactly what
    /// the caller checks for before writing the element back.
    fn float(payload: &[u8]) -> f64 {
        match payload.len() {
            4 => f64::from(f32::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3],
            ])),
            8 => f64::from_be_bytes([
                payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
                payload[7],
            ]),
            _ => 0.0,
        }
    }
}

/// Parse a SimpleBlock/Block header.
fn parse_block_header(b: &[u8]) -> Option<(u64, i64, bool, usize)> {
    let (track, tn_len) = vint(b, false)?;
    if b.len() < tn_len + 3 {
        return None;
    }
    let ts = i16::from_be_bytes([b[tn_len], b[tn_len + 1]]) as i64;
    let flags = b[tn_len + 2];
    if flags & 0b0000_0110 != 0 {
        return None; // laced â€” see module comment
    }
    Some((track, ts, flags & 0b1000_0000 != 0, tn_len + 3))
}

// ---------------------------------------------------------------------------
// Codec identification â€” same classification the header reader uses
// ---------------------------------------------------------------------------

fn classify_video(codec_id: &[u8]) -> Option<VideoCodec> {
    let id = core::str::from_utf8(codec_id)
        .ok()?
        .trim_end_matches('\u{0}');
    let rest = id.strip_prefix("V_")?;
    Some(match rest {
        "MPEG4/ISO/AVC" => VideoCodec::H264,
        "MPEGH/ISO/HEVC" => VideoCodec::H265,
        "AV1" => VideoCodec::Av1,
        "VP8" => VideoCodec::Vp8,
        "VP9" => VideoCodec::Vp9,
        "PRORES" => VideoCodec::ProRes,
        _ => VideoCodec::Other,
    })
}

fn classify_audio(codec_id: &[u8]) -> Option<AudioCodec> {
    let id = core::str::from_utf8(codec_id)
        .ok()?
        .trim_end_matches('\u{0}');
    let rest = id.strip_prefix("A_")?;
    Some(match rest {
        "AAC" => AudioCodec::Aac,
        r if r.starts_with("AAC/") => AudioCodec::Aac,
        "MPEG/L3" => AudioCodec::Mp3,
        "OPUS" => AudioCodec::Opus,
        "VORBIS" => AudioCodec::Vorbis,
        "FLAC" => AudioCodec::Flac,
        "ALAC" => AudioCodec::Alac,
        r if r.starts_with("PCM/") => AudioCodec::Pcm,
        _ => AudioCodec::Other,
    })
}

/// Strip ONE element wrapper: given full element bytes (id + size + body),
/// return the body. The inverse of [`wrap_element`], and the missing half
/// of every call site that hands a full element to a walker expecting a
/// payload â€” without it, the wrapper itself parses as a bogus child.
fn unwrap_once(full: &[u8]) -> Option<&[u8]> {
    let (_id, id_len) = vint(full, true)?;
    let (size, size_len) = vint(&full[id_len..], false)?;
    let start = id_len + size_len;
    let end = start.checked_add(usize::try_from(size).ok()?)?;
    full.get(start..end)
}

fn budget_check(retained: u64, budget: u64) -> Result<(), DemuxError> {
    if retained > budget {
        Err(DemuxError::Bounded("memory"))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Demux
// ---------------------------------------------------------------------------

/// Demux a whole Matroska/WebM file into the graph.
pub fn demux(bytes: &[u8], budget: u64) -> Result<AvGraph, DemuxError> {
    let mut elements = MAX_ELEMENTS;
    let mut top = Reader::top(bytes, &mut elements);
    let mut graph = AvGraph {
        tracks: Vec::new(),
        timecode_scale_ns: 1_000_000,
    };
    let mut retained: u64 = 0;

    while let Some((eid, payload)) = top.next_element()? {
        match eid {
            id::SEGMENT => {
                let mut seg = top.child(payload);
                while let Some((eid, inner)) = seg.next_element()? {
                    match eid {
                        id::TRACKS => {
                            demux_tracks(&mut seg.child(inner), &mut graph, &mut retained, budget)?
                        }
                        id::CLUSTER => {
                            demux_cluster(&mut seg.child(inner), &mut graph, &mut retained, budget)?
                        }
                        _ => {}
                    }
                }
                // One segment per file in practice; converting beyond the
                // first is an accumulation bound nobody asked for.
                break;
            }
            id::INFO => {
                let mut info = top.child(payload);
                while let Some((eid, ip)) = info.next_element()? {
                    if eid == id::TIMECODE_SCALE {
                        graph.timecode_scale_ns =
                            u32::try_from(Reader::uint(ip)).unwrap_or(1_000_000);
                    }
                }
            }
            _ => {}
        }
    }

    if graph.tracks.is_empty() {
        return Err(DemuxError::NoTracks);
    }
    Ok(graph)
}

fn demux_tracks(
    r: &mut Reader<'_, '_>,
    graph: &mut AvGraph,
    retained: &mut u64,
    budget: u64,
) -> Result<(), DemuxError> {
    while let Some((eid, payload)) = r.next_element()? {
        if eid != id::TRACK_ENTRY {
            continue;
        }
        let mut tr = r.child(payload);
        let mut track = AvTrack {
            number: 0,
            kind: TrackKind::Other,
            video: None,
            audio: None,
            codec_id: Vec::new(),
            codec_private: Vec::new(),
            pixel_width: 0,
            pixel_height: 0,
            sample_rate: 0.0,
            channels: 0,
            bit_depth: 0,
            samples: Vec::new(),
        };
        while let Some((eid, ep)) = tr.next_element()? {
            match eid {
                // TrackNumber is stored ID-form (marker bit set): 0x81 means
                // track 1. Reading it as a plain integer turned every REAL
                // file's tracks into 128+n ghosts whose blocks never matched
                // â€” found by running the CLI against an actual file, not by
                // the fixture, whose own [0x01] bytes happened to survive
                // the wrong reader. Both sides are spec-form now.
                id::TRACK_NUMBER => {
                    track.number = vint(ep, false).map_or(Reader::uint(ep), |(v, _)| v);
                }
                id::TRACK_TYPE => match Reader::uint(ep) {
                    1 => track.kind = TrackKind::Video,
                    2 => track.kind = TrackKind::Audio,
                    _ => track.kind = TrackKind::Other,
                },
                id::CODEC_ID => {
                    track.codec_id = ep.to_vec();
                    if let Some(v) = classify_video(ep) {
                        track.video.get_or_insert(v);
                        track.kind = TrackKind::Video;
                    } else if let Some(a) = classify_audio(ep) {
                        track.audio.get_or_insert(a);
                        track.kind = TrackKind::Audio;
                    }
                }
                id::CODEC_PRIVATE => {
                    *retained += ep.len() as u64;
                    budget_check(*retained, budget)?;
                    track.codec_private = ep.to_vec();
                }
                // Descend into Audio for the two fields a rewrap must carry.
                id::AUDIO => {
                    let mut au = tr.child(ep);
                    while let Some((aid, ap)) = au.next_element()? {
                        match aid {
                            id::SAMPLING_FREQUENCY => track.sample_rate = Reader::float(ap),
                            id::CHANNELS => track.channels = Reader::uint(ap),
                            id::BIT_DEPTH => track.bit_depth = Reader::uint(ap),
                            _ => {}
                        }
                    }
                }
                // Descend into Video for the same reason as Audio. This read
                // PixelWidth and PixelHeight as TrackEntry children, which is
                // where THIS build used to write them and where no other muxer
                // ever has -- so the geometry of every real Matroska file was
                // silently zero.
                id::VIDEO => {
                    let mut vi = tr.child(ep);
                    while let Some((vid, vp)) = vi.next_element()? {
                        match vid {
                            id::PIXEL_WIDTH => {
                                track.pixel_width = u32::try_from(Reader::uint(vp)).unwrap_or(0);
                            }
                            id::PIXEL_HEIGHT => {
                                track.pixel_height = u32::try_from(Reader::uint(vp)).unwrap_or(0);
                            }
                            _ => {}
                        }
                    }
                }
                // And the flat form as well, which is what the files this
                // build wrote before the fix above look like. They are not
                // valid Matroska and they exist, so reading them back is a
                // kindness that costs two arms.
                id::PIXEL_WIDTH => track.pixel_width = u32::try_from(Reader::uint(ep)).unwrap_or(0),
                id::PIXEL_HEIGHT => {
                    track.pixel_height = u32::try_from(Reader::uint(ep)).unwrap_or(0)
                }
                _ => {}
            }
        }
        graph.tracks.push(track);
    }
    Ok(())
}

/// One parsed block awaiting its cluster timecode.
struct PendingBlock {
    track: u64,
    rel_ts: i64,
    keyframe: bool,
    data: Vec<u8>,
}

fn demux_cluster(
    r: &mut Reader<'_, '_>,
    graph: &mut AvGraph,
    retained: &mut u64,
    budget: u64,
) -> Result<(), DemuxError> {
    // The cluster timecode may FOLLOW blocks in pathological files, so
    // blocks collect first and resolve once the cluster value is known.
    let mut cluster_ts: i64 = 0;
    let mut pending: Vec<PendingBlock> = Vec::new();

    while let Some((eid, payload)) = r.next_element()? {
        match eid {
            id::CLUSTER_TIMECODE => {
                cluster_ts = if payload.is_empty() || payload.len() > 8 {
                    return Err(DemuxError::Malformed("cluster timecode".to_string()));
                } else {
                    let mut wide = [0u8; 8];
                    wide[8 - payload.len()..].copy_from_slice(payload);
                    let unsigned = u64::from_be_bytes(wide);
                    let shift = 64 - payload.len() * 8;
                    ((unsigned << shift) as i64) >> shift
                };
            }
            id::SIMPLE_BLOCK => {
                let (track, rel, key, off) = parse_block_header(payload)
                    .ok_or(DemuxError::Malformed("block header".to_string()))?;
                *retained += (payload.len() - off) as u64;
                budget_check(*retained, budget)?;
                pending.push(PendingBlock {
                    track,
                    rel_ts: rel,
                    keyframe: key,
                    data: payload[off..].to_vec(),
                });
            }
            id::BLOCK_GROUP => {
                let mut bg = r.child(payload);
                while let Some((eid, bp)) = bg.next_element()? {
                    if eid != id::BLOCK {
                        // BlockDuration ignored ON PURPOSE: playback
                        // decoration, and parsing it means trusting it.
                        continue;
                    }
                    let (track, rel, key, off) = parse_block_header(bp)
                        .ok_or(DemuxError::Malformed("block header".to_string()))?;
                    *retained += (bp.len() - off) as u64;
                    budget_check(*retained, budget)?;
                    pending.push(PendingBlock {
                        track,
                        rel_ts: rel,
                        keyframe: key,
                        data: bp[off..].to_vec(),
                    });
                }
            }
            _ => {}
        }
    }

    for p in pending {
        if let Some(track) = graph.tracks.iter_mut().find(|t| t.number == p.track) {
            track.samples.push(AvSample {
                ts_ticks: cluster_ts.saturating_add(p.rel_ts),
                // Matroska states presentation times and no decode times, so
                // there is nothing to offset BY. See the field's own comment.
                composition_offset_ticks: 0,
                keyframe: p.keyframe,
                data: p.data,
            });
        }
        // Blocks for unknown tracks drop by rule: existing tracks still
        // convert, and nothing about THEM changes silently.
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// EBML â†’ EBML surgery
// ---------------------------------------------------------------------------

/// Splice one Matroska file into another. When `audio_only`, non-audio
/// tracks are excised â€” entries AND blocks both, because leaving silent
/// video bytes inside an "audio-only" file is the quiet dishonesty this
/// product refuses.
pub fn ebml_to_ebml(bytes: &[u8], audio_only: bool, budget: u64) -> Result<Vec<u8>, DemuxError> {
    let mut elements = MAX_ELEMENTS;
    let mut top = Reader::top(bytes, &mut elements);
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut retained: u64 = 0;
    let mut wrote_segment = false;

    while let Some((eid, payload)) = top.next_element()? {
        if eid != id::SEGMENT {
            continue;
        }
        // Everything before the segment (the EBML header, usually) rides
        // through untouched. Only reached once thanks to `break` below.
        out.extend_from_slice(&bytes[..top.elem_start]);

        let audio_tracks = collect_audio_tracks(payload)?;
        let inner = rebuild_segment(payload, audio_only, &audio_tracks, &mut retained, budget)?;
        out.extend_from_slice(&wrap_element(id::SEGMENT, &inner));
        wrote_segment = true;
        break;
    }

    if !wrote_segment {
        return Err(DemuxError::Malformed("no segment found".to_string()));
    }
    budget_check(retained, budget)?;
    Ok(out)
}

fn collect_audio_tracks(segment_payload: &[u8]) -> Result<Vec<u64>, DemuxError> {
    let mut elements = MAX_ELEMENTS;
    let mut seg = Reader::top(segment_payload, &mut elements);
    let mut found = Vec::new();
    while let Some((eid, payload)) = seg.next_element()? {
        if eid != id::TRACKS {
            continue;
        }
        let mut tr = seg.child(payload);
        while let Some((eid, tp)) = tr.next_element()? {
            if eid != id::TRACK_ENTRY {
                continue;
            }
            // `tp` is already the TrackEntry BODY here â€” `next_element`
            // returned the payload of the AE element. Unwrapping again was
            // the bug: it re-parsed the body's own first byte as an ID and
            // handed back garbage.
            let mut te = tr.child(tp);
            let mut number = 0u64;
            let mut is_audio = false;
            while let Some((eid, ep)) = te.next_element()? {
                match eid {
                    id::TRACK_NUMBER => {
                        number = vint(ep, false).map_or(Reader::uint(ep), |(v, _)| v);
                    }
                    id::TRACK_TYPE => is_audio = Reader::uint(ep) == 2,
                    _ => {}
                }
            }
            if is_audio {
                found.push(number);
            }
        }
    }
    Ok(found)
}

fn rebuild_segment(
    payload: &[u8],
    audio_only: bool,
    audio_tracks: &[u64],
    retained: &mut u64,
    budget: u64,
) -> Result<Vec<u8>, DemuxError> {
    let mut elements = MAX_ELEMENTS;
    let mut seg = Reader::top(payload, &mut elements);
    let mut out: Vec<u8> = Vec::new();
    while let Some((eid, inner)) = seg.next_element()? {
        *retained += inner.len() as u64;
        match eid {
            id::TRACKS => {
                out.extend_from_slice(&filter_track_entries(inner, audio_only, audio_tracks))
            }
            id::CLUSTER if audio_only => {
                // filter_cluster_blocks returns a COMPLETE element; wrapping
                // it again produced clusters whose only child was another
                // cluster â€” structurally pretty, semantically empty.
                let kept = filter_cluster_blocks(inner, audio_tracks)?;
                *retained += kept.len() as u64;
                out.extend_from_slice(&kept);
            }
            other => out.extend_from_slice(&wrap_element(other, inner)),
        }
        budget_check(*retained, budget)?;
    }
    Ok(out)
}

fn filter_track_entries(payload: &[u8], audio_only: bool, audio_tracks: &[u64]) -> Vec<u8> {
    let mut elements = MAX_ELEMENTS;
    let mut tr = Reader::top(payload, &mut elements);
    let mut inner: Vec<u8> = Vec::new();
    while let Ok(Some((eid, _))) = tr.next_element() {
        let keep = !audio_only
            || eid != id::TRACK_ENTRY
            || unwrap_once(last_payload(&tr))
                .is_some_and(|fields| entry_is_audio(fields, audio_tracks));
        if keep {
            inner.extend_from_slice(&payload[tr.elem_start..tr.pos]);
        }
    }
    wrap_element(id::TRACKS, &inner)
}

/// The slice of THIS element most recently returned, borrowed from the
/// buffer the reader walks.
///
/// Written as a free function because returning `&'a [u8]` through a method
/// on `&mut self` fights the borrow checker over a lifetime that is, in
/// fact, fine: the slice derives solely from `'a`.
fn last_payload<'a>(r: &Reader<'a, '_>) -> &'a [u8] {
    &r.b[r.elem_start..r.pos]
}

fn entry_is_audio(entry_payload: &[u8], audio_tracks: &[u64]) -> bool {
    let mut elements = MAX_ELEMENTS;
    let mut te = Reader::top(entry_payload, &mut elements);
    // A parse error here reads as "not provably audio": filtering drops it,
    // which is the conservative direction.
    while let Ok(Some((eid, ep))) = te.next_element() {
        if eid == id::TRACK_NUMBER && audio_tracks.contains(&Reader::uint(ep)) {
            return true;
        }
    }
    false
}

fn filter_cluster_blocks(payload: &[u8], audio_tracks: &[u64]) -> Result<Vec<u8>, DemuxError> {
    let mut elements = MAX_ELEMENTS;
    let mut cl = Reader::top(payload, &mut elements);
    let mut inner: Vec<u8> = Vec::new();
    while let Ok(Some((eid, cp))) = cl.next_element() {
        let keep = match eid {
            id::SIMPLE_BLOCK => block_track(cp).is_some_and(|t| audio_tracks.contains(&t)),
            id::BLOCK_GROUP => group_has_kept_block(cp, audio_tracks),
            _ => true,
        };
        if keep {
            inner.extend_from_slice(&payload[cl.elem_start..cl.pos]);
        }
    }
    Ok(wrap_element(id::CLUSTER, &inner))
}

/// Groups without a parseable block drop as noise â€” demux() is where
/// malformed structure becomes an error, and filtering is not demux().
fn group_has_kept_block(group: &[u8], audio_tracks: &[u64]) -> bool {
    let mut elements = MAX_ELEMENTS;
    let mut bg = Reader::top(group, &mut elements);
    while let Ok(Some((eid, bp))) = bg.next_element() {
        if eid == id::BLOCK {
            return block_track(bp).is_some_and(|t| audio_tracks.contains(&t));
        }
    }
    false
}

fn block_track(block_payload: &[u8]) -> Option<u64> {
    parse_block_header(block_payload).map(|(track, _, _, _)| track)
}

/// Wrap a payload as a complete element: canonical ID vint plus an 8-byte
/// size vint. Seven spare bytes per element buys never back-patching a
/// length â€” determinism over compactness.
fn wrap_element(eid: u64, payload: &[u8]) -> Vec<u8> {
    let be = eid.to_be_bytes();
    let first_nonzero = be.iter().position(|&b| b != 0).unwrap_or(7);
    let mut out = Vec::with_capacity(payload.len() + 16);
    out.extend_from_slice(&be[first_nonzero..]);
    out.extend_from_slice(&encode_long_vint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

/// An 8-byte-form EBML size vint: marker `0x01`, then 7 big-endian bytes.
const fn encode_long_vint(len: u64) -> [u8; 8] {
    let be = len.to_be_bytes();
    [0x01, be[1], be[2], be[3], be[4], be[5], be[6], be[7]]
}

/// Which pairs this module's surgery covers.
#[must_use]
pub const fn supports_surgery(from: FormatId, to: FormatId) -> bool {
    matches!(
        (from, to),
        (FormatId::Webm, FormatId::Mkv)
            | (FormatId::Mkv, FormatId::Webm)
            | (FormatId::Mkv, FormatId::Mka)
            | (FormatId::Webm, FormatId::Mka)
    )
}

// ---------------------------------------------------------------------------
// Graph-level editing: trim and concat
//
// Both operate on the DEMUXED graph rather than on raw bytes, because both
// decide WHICH samples exist and WHEN they claim to play — exactly the two
// things byte surgery exists to never fake. The payloads themselves still
// travel verbatim; losslessness survives, the timeline is what gets edited.
// ---------------------------------------------------------------------------

/// Milliseconds to the graph's tick space, saturating rather than overflowing:
/// an absurd bound means "everything", not a panic and not a wrapped negative.
fn ms_to_ticks(ms: u64, scale_ns: u32) -> i64 {
    let scale = u64::from(scale_ns.max(1));
    i64::try_from(ms.saturating_mul(1_000_000) / scale).unwrap_or(i64::MAX)
}

/// Keep only samples whose timestamp falls within [start_ms, end_ms).
///
/// The start boundary snaps BACKWARD to the nearest keyframe so the output
/// begins on a frame a player can actually decode. Without keyframe
/// alignment, the first N frames of a trimmed video are grey/corrupted.
///
/// Timestamps are in MILLISECONDS. The graph's timecode_scale_ns converts
/// internal ticks to nanoseconds; ms = ticks * scale_ns / 1_000_000.
///
/// Kept samples carry their SOURCE positions — trim does not rebase to zero.
/// Rebasing would discard information (where in the original timeline this
/// content lived), and a player idling through the kept lead-in is honest;
/// callers wanting a zero-based clip can shift deliberately afterwards.
///
/// Errors when the window selects nothing at all: an operation that produced
/// an empty container would surface downstream as a mysterious dead file,
/// and this is the layer that knows why.
pub fn trim(graph: &AvGraph, start_ms: u64, end_ms: u64) -> Result<AvGraph, DemuxError> {
    let start_ticks = ms_to_ticks(start_ms, graph.timecode_scale_ns);
    let end_ticks = ms_to_ticks(end_ms, graph.timecode_scale_ns);
    let tracks = graph
        .tracks
        .iter()
        .map(|t| AvTrack {
            number: t.number,
            kind: t.kind,
            video: t.video,
            audio: t.audio,
            codec_id: t.codec_id.clone(),
            codec_private: t.codec_private.clone(),
            pixel_width: t.pixel_width,
            pixel_height: t.pixel_height,
            sample_rate: t.sample_rate,
            channels: t.channels,
            bit_depth: t.bit_depth,
            samples: trimmed_samples(&t.samples, start_ticks, end_ticks),
        })
        .collect();
    let trimmed = AvGraph {
        tracks,
        timecode_scale_ns: graph.timecode_scale_ns,
    };
    if trimmed.tracks.iter().all(|t| t.samples.is_empty()) {
        return Err(DemuxError::Malformed(
            "this trim window selects no samples; refusing to emit an empty file".to_string(),
        ));
    }
    Ok(trimmed)
}

/// One track's window. The naive cut is the first sample at/after the
/// requested start; it snaps BACKWARD to the nearest keyframe at-or-before
/// that point so playback begins decodable. A track offering no such keyframe
/// (audio never sets the bit; some streams have none at all) starts at the
/// cut itself — keeping data beats guessing it undecodable.
///
/// The exclusive end bound is applied as a FILTER, not a scan-stop: source
/// order is preserved but never assumed ascending, so one out-of-order
/// timestamp cannot silently truncate every sample after it.
fn trimmed_samples(samples: &[AvSample], start_ticks: i64, end_ticks: i64) -> Vec<AvSample> {
    let Some(cut) = samples.iter().position(|s| s.ts_ticks >= start_ticks) else {
        return Vec::new(); // the whole track ends before the window opens
    };
    let begin = samples[..=cut]
        .iter()
        .rposition(|s| s.keyframe)
        .unwrap_or(cut);
    samples[begin..]
        .iter()
        .filter(|s| s.ts_ticks < end_ticks)
        .map(|s| AvSample {
            ts_ticks: s.ts_ticks,
            composition_offset_ticks: s.composition_offset_ticks,
            keyframe: s.keyframe,
            data: s.data.clone(),
        })
        .collect()
}

/// The median gap between consecutive samples across a graph's tracks: the
/// one cadence number that a stray gap or a single odd frame cannot move.
fn median_delta(graph: &AvGraph) -> Option<i64> {
    let mut deltas: Vec<i64> = graph
        .tracks
        .iter()
        .flat_map(|t| t.samples.windows(2).map(|w| w[1].ts_ticks - w[0].ts_ticks))
        .collect();
    if deltas.is_empty() {
        return None;
    }
    deltas.sort_unstable();
    Some(deltas[deltas.len() / 2])
}

/// Concatenate two graphs with matching codecs into one.
///
/// B's timestamps shift past A's last sample by at least one tick, and by one
/// median delta period whenever A's cadence says what that is, preserving
/// monotonicity. Refuses when the files cannot honestly share a timeline:
///
/// - different track counts, or paired tracks whose codec SET differs —
///   concatenating H.264 onto VP9 produces a file no decoder will play, and
///   pretending otherwise is worse than refusing;
/// - paired tracks whose CODEC CONFIGURATION differs (CodecPrivate bytes,
///   declared dimensions) — same codec name is not the same stream setup,
///   and the output carries only A's setup data;
/// - differing TimecodeScale — one side's timestamps would be resampled.
///
/// ONE shift applies to every B track: B's internal A/V alignment rides
/// through untouched. The price is that when A's tracks END at different
/// times, whichever ended earliest shows a gap before B arrives; per-track
/// offsets would close that gap by desynchronising B, the worse trade.
pub fn concat(a: &AvGraph, b: &AvGraph) -> Result<AvGraph, DemuxError> {
    const MISMATCH: &str = "codec sets differ between files; concat requires identical codecs";
    const CONFIG_MISMATCH: &str =
        "codec configurations differ between files; concat would splice incompatible \
         CodecPrivate or dimensions into one track";
    const LAYOUT_MISMATCH: &str =
        "track layouts differ between files; concat pairs tracks by position";
    if a.tracks.len() != b.tracks.len() {
        return Err(DemuxError::Malformed(LAYOUT_MISMATCH.to_string()));
    }
    if a.timecode_scale_ns != b.timecode_scale_ns {
        return Err(DemuxError::Malformed(
            "timecode scales differ between files; one side's timestamps would be silently \
             resampled"
                .to_string(),
        ));
    }
    for (ta, tb) in a.tracks.iter().zip(&b.tracks) {
        if (ta.kind, ta.video, ta.audio) != (tb.kind, tb.video, tb.audio) {
            return Err(DemuxError::Malformed(MISMATCH.to_string()));
        }
        if ta.codec_private != tb.codec_private
            || (ta.pixel_width, ta.pixel_height) != (tb.pixel_width, tb.pixel_height)
        {
            return Err(DemuxError::Malformed(CONFIG_MISMATCH.to_string()));
        }
    }

    // ONE offset for all of B — A's furthest last sample plus one median
    // delta — so B begins a beat after A ends instead of ON it. Floored at
    // one tick past A: a degenerate cadence (single-sample tracks, deltas
    // that do not ascend) must still leave B strictly AFTER A.
    let last_a = a
        .tracks
        .iter()
        .filter_map(|t| t.samples.last())
        .map(|s| s.ts_ticks)
        .max()
        .unwrap_or(0);
    let offset = last_a
        .saturating_add(median_delta(a).unwrap_or(0))
        .max(last_a.saturating_add(1));

    let tracks = a
        .tracks
        .iter()
        .zip(&b.tracks)
        .map(|(ta, tb)| {
            let mut samples = ta
                .samples
                .iter()
                .map(|s| AvSample {
                    ts_ticks: s.ts_ticks,
                    composition_offset_ticks: s.composition_offset_ticks,
                    keyframe: s.keyframe,
                    data: s.data.clone(),
                })
                .collect::<Vec<AvSample>>();
            samples.extend(tb.samples.iter().map(|s| AvSample {
                ts_ticks: s.ts_ticks.saturating_add(offset),
                // Shifting when a sample is decoded does not change how far
                // after that it is shown.
                composition_offset_ticks: s.composition_offset_ticks,
                keyframe: s.keyframe,
                data: s.data.clone(),
            }));
            AvTrack {
                number: ta.number,
                kind: ta.kind,
                video: ta.video,
                audio: ta.audio,
                codec_id: ta.codec_id.clone(),
                codec_private: ta.codec_private.clone(),
                pixel_width: ta.pixel_width,
                pixel_height: ta.pixel_height,
                sample_rate: ta.sample_rate,
                channels: ta.channels,
                bit_depth: ta.bit_depth,
                samples,
            }
        })
        .collect();
    Ok(AvGraph {
        tracks,
        timecode_scale_ns: a.timecode_scale_ns,
    })
}

// ---------------------------------------------------------------------------
// Graph → EBML
//
// The writer half of [`demux`]: serialises an edited graph back into a
// Matroska file so trim/concat results reach disk without a re-encode. Like
// every writer in this project it is deterministic — 8-byte size vints
// throughout, no back-patching — and it carries only what the graph can prove.
// ---------------------------------------------------------------------------

/// Element IDs used only by the writer.
mod write_id {
    pub const EBML_HEADER: u64 = 0x1A45_DFA3;
    pub const EBML_VERSION: u64 = 0x4286;
    pub const EBML_READ_VERSION: u64 = 0x42F7;
    pub const EBML_MAX_ID_LENGTH: u64 = 0x42F2;
    pub const EBML_MAX_SIZE_LENGTH: u64 = 0x42F3;
    pub const DOCTYPE: u64 = 0x4282;
    pub const DOCTYPE_VERSION: u64 = 0x4287;
    pub const DOCTYPE_READ_VERSION: u64 = 0x4285;
    pub const MUXING_APP: u64 = 0x4D80;
    pub const WRITING_APP: u64 = 0x5741;
    pub const TRACK_UID: u64 = 0x73C5;
}

/// Serialise a graph into a structurally complete Matroska file.
///
/// Sample payloads, timestamps, CodecID and CodecPrivate bytes travel
/// verbatim; everything else (the EBML header, Info, TrackUIDs) is written
/// canonically by this file rather than inherited from a source that may not
/// exist — a concatenated or hand-edited graph has no single origin document.
///
/// DocType is always "matroska": the superset profile. Claiming "webm" for a
/// graph whose codecs happen to be VP9 today would break the day someone
/// trims an H.264 clip, and the strictness belongs to whoever names a target,
/// not to a writer that cannot see one.
///
/// Refuses tracks whose identity cannot be written honestly: non-A/V kinds
/// (whose spec TrackType this module does not emit) and any track whose
/// CodecID was never read. A refusal here is why trim cannot silently drop a
/// subtitle track on its way through.
pub fn graph_to_ebml(graph: &AvGraph) -> Result<Vec<u8>, DemuxError> {
    for t in &graph.tracks {
        if t.kind == TrackKind::Other {
            return Err(DemuxError::Malformed(format!(
                "track {} is neither video nor audio; this build does not re-emit such \
                 tracks, so the conversion refuses rather than dropping them",
                t.number
            )));
        }
        if t.codec_id.is_empty() {
            return Err(DemuxError::Malformed(format!(
                "track {} never reported a CodecID; refusing to emit an unidentified stream",
                t.number
            )));
        }
    }

    let mut out = Vec::new();
    out.extend_from_slice(&wrap_element(write_id::EBML_HEADER, &ebml_header()));
    out.extend_from_slice(&wrap_element(id::SEGMENT, &segment_body(graph)));
    Ok(out)
}

/// A minimal, spec-honest EBML header.
fn ebml_header() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&wrap_element(write_id::EBML_VERSION, &[1]));
    out.extend_from_slice(&wrap_element(write_id::EBML_READ_VERSION, &[1]));
    out.extend_from_slice(&wrap_element(write_id::EBML_MAX_ID_LENGTH, &[4]));
    out.extend_from_slice(&wrap_element(write_id::EBML_MAX_SIZE_LENGTH, &[8]));
    out.extend_from_slice(&wrap_element(write_id::DOCTYPE, b"matroska"));
    out.extend_from_slice(&wrap_element(write_id::DOCTYPE_VERSION, &[2]));
    out.extend_from_slice(&wrap_element(write_id::DOCTYPE_READ_VERSION, &[2]));
    out
}

fn segment_body(graph: &AvGraph) -> Vec<u8> {
    let mut seg = info_element(graph);
    seg.extend_from_slice(&tracks_element(graph));
    for track in &graph.tracks {
        seg.extend_from_slice(&clusters_for(track));
    }
    seg
}

/// TimecodeScale plus the two strings the spec marks mandatory. Naming this
/// crate as the muxer is exactly what a receipt should be able to assume.
fn info_element(graph: &AvGraph) -> Vec<u8> {
    let mut inner = wrap_element(
        id::TIMECODE_SCALE,
        &uint_payload(u64::from(graph.timecode_scale_ns)),
    );
    inner.extend_from_slice(&wrap_element(
        write_id::MUXING_APP,
        env!("CARGO_PKG_NAME").as_bytes(),
    ));
    inner.extend_from_slice(&wrap_element(
        write_id::WRITING_APP,
        env!("CARGO_PKG_NAME").as_bytes(),
    ));
    wrap_element(id::INFO, &inner)
}

fn tracks_element(graph: &AvGraph) -> Vec<u8> {
    let mut inner = Vec::new();
    for (i, t) in graph.tracks.iter().enumerate() {
        // TrackNumber is a PLAIN unsigned integer, not a vint.
        //
        // The vint form belongs to the BLOCK HEADER, where track 1 really is
        // `0x81`, and writing that form here put `0x81` in the TrackEntry too.
        // Our own reader tries vint first and so read it back correctly, which
        // is exactly why this survived: the round trip was self-consistent and
        // the file was not conformant. symphonia read the same track as number
        // 129, matched it against blocks that said 1, discarded every packet,
        // and reported "no decodable audio".
        //
        // Reading stays tolerant of both forms -- real files exist that this
        // wrote -- but what we EMIT is now what the spec says.
        let mut entry = wrap_element(id::TRACK_NUMBER, &uint_payload(t.number));
        entry.extend_from_slice(&wrap_element(
            write_id::TRACK_UID,
            &uint_payload(i as u64 + 1),
        ));
        entry.extend_from_slice(&wrap_element(
            id::TRACK_TYPE,
            &[match t.kind {
                TrackKind::Video => 1,
                TrackKind::Audio => 2,
                TrackKind::Other => unreachable!("refused by graph_to_ebml above"),
            }],
        ));
        entry.extend_from_slice(&wrap_element(id::CODEC_ID, &t.codec_id));
        if !t.codec_private.is_empty() {
            entry.extend_from_slice(&wrap_element(id::CODEC_PRIVATE, &t.codec_private));
        }
        // THE AUDIO ELEMENT, without which our own output was undecodable.
        //
        // `graph_to_ebml` wrote TrackNumber, TrackType, CodecID and
        // CodecPrivate and stopped. That is enough for a demuxer that only
        // moves blocks around -- which is all this module did -- and not
        // enough for anything that decodes: symphonia read the track, found
        // no Audio element, and reported CODEC_TYPE_NULL, so `mkv -> mka`
        // produced files this project could not itself read back.
        //
        // Matroska defaults SamplingFrequency to 8000 and Channels to 1, so
        // omitting them is not "unspecified" downstream, it is wrong. Written
        // only when the source actually reported them: inventing 48 kHz for a
        // track that never said would be the same mistake pointed the other
        // way.
        if t.kind == TrackKind::Audio {
            let mut audio = Vec::new();
            if t.sample_rate > 0.0 {
                audio.extend_from_slice(&wrap_element(
                    id::SAMPLING_FREQUENCY,
                    &t.sample_rate.to_be_bytes(),
                ));
            }
            if t.channels > 0 {
                audio.extend_from_slice(&wrap_element(id::CHANNELS, &uint_payload(t.channels)));
            }
            if t.bit_depth > 0 {
                audio.extend_from_slice(&wrap_element(id::BIT_DEPTH, &uint_payload(t.bit_depth)));
            }
            if !audio.is_empty() {
                entry.extend_from_slice(&wrap_element(id::AUDIO, &audio));
            }
        }
        // INSIDE A Video MASTER, which is where the specification puts them
        // and where every muxer writes them. Emitted at TrackEntry level, they
        // are not merely ignored: ffmpeg reports "Unknown entry 0xB0" and
        // refuses the file, so every `mp4 -> mkv` this build wrote was
        // readable only by itself.
        if t.kind == TrackKind::Video {
            let mut video = Vec::new();
            if t.pixel_width > 0 {
                video.extend_from_slice(&wrap_element(
                    id::PIXEL_WIDTH,
                    &uint_payload(u64::from(t.pixel_width)),
                ));
            }
            if t.pixel_height > 0 {
                video.extend_from_slice(&wrap_element(
                    id::PIXEL_HEIGHT,
                    &uint_payload(u64::from(t.pixel_height)),
                ));
            }
            if !video.is_empty() {
                entry.extend_from_slice(&wrap_element(id::VIDEO, &video));
            }
        }
        inner.extend_from_slice(&wrap_element(id::TRACK_ENTRY, &entry));
    }
    wrap_element(id::TRACKS, &inner)
}

/// One track's samples as clusters. A SimpleBlock's relative timestamp is
/// 16 bits; whenever a sample would fall outside that window from the current
/// cluster's base, the cluster closes and a new one opens at the sample.
///
/// **Matroska timestamps are PRESENTATION times**, so each block is written at
/// `ts_ticks + composition_offset_ticks`. Writing the decode time was what put
/// a B-frame MP4's frames into a Matroska file with the wrong times on them;
/// the offset is zero for audio and for video with no B-frames, which is why
/// nothing noticed until an MP4 source arrived.
fn clusters_for(track: &AvTrack) -> Vec<u8> {
    let tv = encode_id_vint(track.number);
    let mut out = Vec::new();
    let mut cluster_inner: Vec<u8> = Vec::new();
    let mut base: Option<i64> = None;

    let flush = |base: i64, inner: &mut Vec<u8>, out: &mut Vec<u8>| {
        if !inner.is_empty() {
            let mut with_tc = wrap_element(id::CLUSTER_TIMECODE, &(base as u64).to_be_bytes());
            with_tc.append(inner);
            out.extend_from_slice(&wrap_element(id::CLUSTER, &with_tc));
            *inner = Vec::new();
        }
    };

    for s in &track.samples {
        let pts = s.ts_ticks.saturating_add(s.composition_offset_ticks);
        let b = base.unwrap_or(pts);
        let rel = pts - b;
        if base.is_some() && !(i64::from(i16::MIN)..=i64::from(i16::MAX)).contains(&rel) {
            flush(base.unwrap_or(0), &mut cluster_inner, &mut out);
            base = None;
        }
        let b = base.unwrap_or(pts);
        if base.is_none() {
            base = Some(pts);
        }
        cluster_inner.extend_from_slice(&simple_block(&tv, (pts - b) as i16, s.keyframe, &s.data));
    }
    flush(base.unwrap_or(0), &mut cluster_inner, &mut out);
    out
}

/// A SimpleBlock: track vint, signed relative timestamp, flags (keyframe bit
/// only — lacing is never produced here), payload verbatim.
fn simple_block(track_vint: &[u8], rel_ts: i16, keyframe: bool, data: &[u8]) -> Vec<u8> {
    let mut block = Vec::with_capacity(track_vint.len() + 3 + data.len());
    block.extend_from_slice(track_vint);
    block.extend_from_slice(&rel_ts.to_be_bytes());
    block.push(if keyframe { 0b1000_0000 } else { 0 });
    block.extend_from_slice(data);
    wrap_element(id::SIMPLE_BLOCK, &block)
}

/// Minimal big-endian unsigned payload (no leading zero bytes).
fn uint_payload(v: u64) -> Vec<u8> {
    let be = v.to_be_bytes();
    let start = be.iter().position(|&b| b != 0).unwrap_or(7);
    be[start..].to_vec()
}

/// An ID-form vint: marker bit set in the first byte's top position, value in
/// the remaining bits — the form TrackNumber uses, and what `vint(_, false)`
/// expects to strip on the way back through [`demux`].
fn encode_id_vint(v: u64) -> Vec<u8> {
    debug_assert!(v > 0, "track numbers start at one");
    let mut len = 1usize;
    while len < 8 && v > ((1u64 << (7 * len)) - 1) {
        len += 1;
    }
    let mut out = vec![0u8; len];
    out.copy_from_slice(&v.to_be_bytes()[8 - len..]);
    out[0] |= 0x80 >> (len - 1);
    out
}
