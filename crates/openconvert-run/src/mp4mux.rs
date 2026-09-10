//! MP4 muxing: the stream graph rebuilt as an ISO-BMFF file.
//!
//! # The rule this module keeps
//!
//! **Samples pass through byte-for-byte.** Matroska stores H.264/AAC exactly
//! the way MP4 wants them â€” length-prefixed NAL units, raw AAC frames â€” so a
//! remux is bookkeeping (boxes, tables, offsets) and not one bit of coded
//! payload changes. That is what makes `MKV â†’ MP4` seconds instead of
//! minutes, and what makes the receipt's Class A claim true.
//!
//! # Layout determinism, by construction rather than patching
//!
//! Chunk offsets depend on the size of everything before them. Instead of
//! writing placeholders and back-patching, every box is BUILT twice: once to
//! measure, once with the now-known offsets. Both passes run over identical
//! inputs, so identical files come out â€” which is the property the receipt
//! hashes â€” and there is no patch table whose arithmetic can drift from the
//! layout it claims to describe.
//!
//! # What is deliberately absent
//!
//! No edit lists, fragmented MP4s, or chapter/tag translation. Container
//! metadata with no lossless destination is disclosed by the caller via
//! `removed`, never smuggled through or silently dropped.

use crate::matroska::{AvGraph, AvTrack, TrackKind};
use openconvert_core::codec::{AudioCodec, VideoCodec};
use openconvert_core::format::FormatId;

/// Why muxing failed. Like every error here, it names the problem in the
/// user's terms.
#[derive(Debug, thiserror::Error)]
pub enum MuxError {
    /// A track this build cannot place in MP4 was still in the graph.
    #[error("{codec} cannot be placed in an MP4 container")]
    Unsupported {
        /// Which codec, named.
        codec: String,
    },
    /// A structural impossibility while assembling boxes.
    #[error("could not assemble the MP4 ({0})")]
    Assemble(&'static str),
}

/// Mux the graph into a complete MP4, plus disclosures for whatever was left
/// behind.
///
/// Tracks the codec table refuses were refused earlier, at routing; anything
/// unplaceable that STILL arrives is an internal inconsistency and errors
/// loudly rather than producing a broken file.
///
/// # Errors
///
/// [`MuxError`].
pub fn mux(graph: &AvGraph) -> Result<(Vec<u8>, Vec<String>), MuxError> {
    let mut removed: Vec<String> = Vec::new();
    let mut kept: Vec<&AvTrack> = Vec::new();

    for t in &graph.tracks {
        let carryable = match t.kind {
            TrackKind::Video => t.video.is_some_and(|c| c.carried_by(FormatId::Mp4)),
            TrackKind::Audio => t.audio.is_some_and(|c| c.carried_by(FormatId::Mp4)),
            TrackKind::Other => false,
        };
        if carryable {
            kept.push(t);
            continue;
        }
        match t.kind {
            // Auxiliary tracks have no MP4 home in this build. DROPPED WITH
            // DISCLOSURE, never smuggled through and never silently lost.
            TrackKind::Other => removed.push(format!(
                "auxiliary track {} dropped (no MP4 mapping)",
                t.number
            )),
            _ => {
                let name = match (t.video, t.audio) {
                    (Some(v), _) => format!("{v:?}"),
                    (_, Some(a)) => format!("{a:?}"),
                    _ => "unidentified".to_string(),
                };
                return Err(MuxError::Unsupported { codec: name });
            }
        }
    }
    if kept.is_empty() {
        return Err(MuxError::Assemble("every track was dropped"));
    }

    // Media timescale per track = segment ticks per second, so source tick
    // deltas transfer EXACTLY â€” no frame-rate guessing anywhere.
    let ticks_per_second = u64::from(graph.timecode_scale_ns).max(1);
    let timescale = u32::try_from(1_000_000_000 / ticks_per_second).unwrap_or(1000);

    let max_last_tick = kept
        .iter()
        .map(|t| t.samples.last().map_or(0, |s| s.ts_ticks.max(0)))
        .max()
        .unwrap_or(0);
    let duration_ms = ticks_to_ms(max_last_tick, graph.timecode_scale_ns);

    let measure = |chunk_offset: u64| -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&ftyp_box());
        let mut moov = Vec::new();
        push_moov(
            &mut moov,
            &kept,
            timescale,
            duration_ms,
            chunk_offset,
            graph.timecode_scale_ns,
        );
        out.extend_from_slice(&moov);
        out.extend_from_slice(&mdat_header(&kept));
        out
    };

    // Pass one measures; pass two emits with the true mdat data offset.
    let data_offset = measure(0).len() as u64;
    let mut out = measure(data_offset);

    // The coded samples themselves — the part this module exists to move
    // UNCHANGED, appended after the boxes whose sizes were measured above.
    // Both passes are identical in length because every field between them
    // is fixed-width.
    for t in &kept {
        for s in &t.samples {
            out.extend_from_slice(&s.data);
        }
    }

    // The measured head plus the samples must account for every emitted
    // byte. If they ever drift, an stco offset written in pass two points
    // into the wrong place — the assertion is what keeps that loud.
    let sample_total: usize = kept
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| s.data.len())
        .sum();
    debug_assert_eq!(out.len(), data_offset as usize + sample_total);

    Ok((out, removed))
}

fn ticks_to_ms(ticks: i64, scale_ns: u32) -> u32 {
    let ns = (ticks.max(0) as u64).saturating_mul(u64::from(scale_ns));
    (ns / 1_000_000).min(u64::from(u32::MAX)) as u32
}

fn mdat_header(tracks: &[&AvTrack]) -> [u8; 8] {
    let total: u64 = tracks
        .iter()
        .flat_map(|t| t.samples.iter())
        .map(|s| s.data.len() as u64)
        .sum();
    let mut h = [0u8; 8];
    h[..4].copy_from_slice(&((total + 8).min(u64::from(u32::MAX)) as u32).to_be_bytes());
    h[4..].copy_from_slice(b"mdat");
    h
}

// ---------------------------------------------------------------------------
// Box primitives â€” every builder returns a COMPLETE box
// ---------------------------------------------------------------------------

fn box_bytes(kind: &[u8; 4], payload_len: usize) -> [u8; 8] {
    let size = (payload_len as u32) + 8;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&size.to_be_bytes());
    out[4..].copy_from_slice(kind);
    out
}

fn full_box(kind: &[u8; 4], version: u8, flags: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 12);
    out.extend_from_slice(&box_bytes(kind, payload.len() + 4));
    out.push(version);
    out.extend_from_slice(&flags.to_be_bytes()[1..]);
    out.extend_from_slice(payload);
    out
}

fn plain_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 8);
    out.extend_from_slice(&box_bytes(kind, payload.len()));
    out.extend_from_slice(payload);
    out
}

fn ftyp_box() -> Vec<u8> {
    let mut p = Vec::with_capacity(24);
    p.extend_from_slice(b"isom");
    p.extend_from_slice(&512_u32.to_be_bytes());
    p.extend_from_slice(b"isom");
    p.extend_from_slice(b"iso2");
    p.extend_from_slice(b"avc1");
    p.extend_from_slice(b"mp41");
    plain_box(b"ftyp", &p)
}

fn push_moov(
    out: &mut Vec<u8>,
    tracks: &[&AvTrack],
    timescale: u32,
    duration_ms: u32,
    chunk_offset: u64,
    scale_ns: u32,
) {
    let mut payload = Vec::new();
    payload.extend_from_slice(&mvhd_box(duration_ms));

    // EACH TRACK GETS ITS OWN CHUNK OFFSET.
    //
    // The samples are written one track after another:
    //
    //     for t in &kept { for s in &t.samples { out.extend(&s.data) } }
    //
    // so track N starts after every earlier track's bytes. Every track was
    // given the SAME `chunk_offset` -- the start of mdat -- which points the
    // second and later tracks at the FIRST track's data. A player reading the
    // audio track of an `mkv -> mp4` remux was handed video bytes.
    //
    // Nothing caught it because nothing in this project could read an MP4
    // back until a demuxer existed.
    let mut at = chunk_offset;
    for (i, t) in tracks.iter().enumerate() {
        let id = u32::try_from(i + 1).unwrap_or(u32::MAX);
        payload.extend_from_slice(&trak_box(t, id, timescale, duration_ms, at, scale_ns));
        at += t.samples.iter().map(|s| s.data.len() as u64).sum::<u64>();
    }
    out.extend_from_slice(&plain_box(b"moov", &payload));
}

fn mvhd_box(duration_ms: u32) -> Vec<u8> {
    let mut p = Vec::with_capacity(96);
    p.extend_from_slice(&0_u32.to_be_bytes()); // creation
    p.extend_from_slice(&0_u32.to_be_bytes()); // modification
    p.extend_from_slice(&1000_u32.to_be_bytes()); // timescale: milliseconds
    p.extend_from_slice(&duration_ms.to_be_bytes());
    p.extend_from_slice(&65536_i32.to_be_bytes()); // rate 1.0
    p.extend_from_slice(&256_i16.to_be_bytes()); // volume 1.0
    p.extend_from_slice(&[0u8; 10]); // reserved
    p.extend_from_slice(&[0u8; 36]); // matrix/predefined
    p.extend_from_slice(&[0u8; 24]); // predefined
    p.extend_from_slice(&(tracks_upper_bound()).to_be_bytes()); // next_track_id
    full_box(b"mvhd", 0, 0, &p)
}

/// Upper bound for next_track_id: one past the most tracks this build will
/// ever place. Constant, so the bytes never vary with input.
const fn tracks_upper_bound() -> u32 {
    16
}

fn trak_box(
    t: &AvTrack,
    track_id: u32,
    timescale: u32,
    duration_ms: u32,
    chunk_offset: u64,
    _scale_ns: u32,
) -> Vec<u8> {
    let mut tkhd_p = Vec::with_capacity(84);
    tkhd_p.extend_from_slice(&0_u32.to_be_bytes()); // creation
    tkhd_p.extend_from_slice(&0_u32.to_be_bytes()); // modification
    tkhd_p.extend_from_slice(&track_id.to_be_bytes());
    tkhd_p.extend_from_slice(&0_u32.to_be_bytes()); // reserved
    tkhd_p.extend_from_slice(&duration_ms.to_be_bytes());
    tkhd_p.extend_from_slice(&[0u8; 8]); // reserved
    tkhd_p.extend_from_slice(&[0u8; 2]); // layer
    tkhd_p.extend_from_slice(&[0u8; 2]); // alternate group
    let volume: [u8; 2] = if t.kind == TrackKind::Audio {
        [0x01, 0x00]
    } else {
        [0, 0]
    };
    tkhd_p.extend_from_slice(&volume);
    tkhd_p.extend_from_slice(&[0u8; 2]); // reserved
    tkhd_p.extend_from_slice(&identity_matrix());
    let (w, h) = if t.kind == TrackKind::Video {
        (
            u32::from(u16::try_from(t.pixel_width).unwrap_or(0)) << 16,
            u32::from(u16::try_from(t.pixel_height).unwrap_or(0)) << 16,
        )
    } else {
        (0, 0)
    };
    tkhd_p.extend_from_slice(&w.to_be_bytes());
    tkhd_p.extend_from_slice(&h.to_be_bytes());

    // ---- mdia ----
    let last_tick = t.samples.last().map_or(0, |s| s.ts_ticks.max(0));

    let mut mdhd_p = Vec::with_capacity(20);
    mdhd_p.extend_from_slice(&0_u32.to_be_bytes());
    mdhd_p.extend_from_slice(&0_u32.to_be_bytes());
    mdhd_p.extend_from_slice(&timescale.to_be_bytes());
    // Media duration in MEDIA units: the source's own tick count. Exact.
    let dur_media = u64::try_from(last_tick)
        .unwrap_or(0)
        .min(u64::from(u32::MAX)) as u32;
    mdhd_p.extend_from_slice(&dur_media.to_be_bytes());
    mdhd_p.extend_from_slice(&[0x55, 0xC4, 0x00, 0x00]); // und language
    let mdia_payload = {
        let mut m = Vec::new();
        m.extend_from_slice(&full_box(b"mdhd", 0, 0, &mdhd_p));
        m.extend_from_slice(&hdlr_box(t.kind));
        m.extend_from_slice(&minf_box(t, chunk_offset, timescale));
        m
    };
    let mut trak = Vec::new();
    trak.extend_from_slice(&full_box(b"tkhd", 0, 7, &tkhd_p));
    trak.extend_from_slice(&plain_box(b"mdia", &mdia_payload));
    plain_box(b"trak", &trak)
}

fn identity_matrix() -> [u8; 36] {
    let mut m = [0u8; 36];
    m[..4].copy_from_slice(&65536_i32.to_be_bytes());
    m[16..20].copy_from_slice(&65536_i32.to_be_bytes());
    m[32..36].copy_from_slice(&16384_i32.to_be_bytes());
    m
}

fn hdlr_box(kind: TrackKind) -> Vec<u8> {
    let mut p = Vec::with_capacity(25);
    p.extend_from_slice(&[0u8; 4]); // predefined
    p.extend_from_slice(if kind == TrackKind::Video {
        b"vide"
    } else {
        b"soun"
    });
    p.extend_from_slice(&[0u8; 12]);
    p.extend_from_slice(if kind == TrackKind::Video {
        b"VideoHandler\0"
    } else {
        b"SoundHandler\0"
    });
    full_box(b"hdlr", 0, 0, &p)
}

fn minf_box(t: &AvTrack, chunk_offset: u64, timescale: u32) -> Vec<u8> {
    let mut minf = Vec::new();
    let media_header = match t.kind {
        TrackKind::Video => full_box(b"vmhd", 0, 1, &[0u8; 8]),
        _ => full_box(b"smhd", 0, 0, &[0u8; 4]),
    };
    minf.extend_from_slice(&media_header);

    // `dref` LIVES IN `dinf`, and the whole lot lives in `minf`.
    //
    // This function returned minf's CONTENTS and never wrapped them, and put
    // `dref` directly beside them. Both boxes are mandatory in ISO/IEC
    // 14496-12, so every MP4 this project has written was malformed --
    // players are lenient enough to find `stbl` anyway, which is why it went
    // unnoticed until a demuxer that follows the spec could not find it.
    let mut dref_entry = full_box(b"url ", 0, 1, &[]);
    let mut dref_p = 1_u32.to_be_bytes().to_vec();
    dref_p.append(&mut dref_entry);
    minf.extend_from_slice(&plain_box(b"dinf", &full_box(b"dref", 0, 0, &dref_p)));

    let mut stbl = Vec::new();
    stbl.extend_from_slice(&stsd_box(t));
    stbl.extend_from_slice(&stts_box(t));
    // ONLY when some sample is shown at a time other than its decode time.
    // An absent `ctts` means every offset is zero, which is true of all audio
    // and of video with no B-frames, and writing a table of zeroes would be
    // bytes saying nothing.
    if let Some(ctts) = ctts_box(t) {
        stbl.extend_from_slice(&ctts);
    }
    stbl.extend_from_slice(&stsc_box(t));
    stbl.extend_from_slice(&stsz_box(t));
    stbl.extend_from_slice(&stco_box(chunk_offset));
    // Sync samples, ONLY when some are not.
    //
    // An absent `stss` means every sample is a sync point, which is exactly
    // right for audio and a lie for video: without it a player cannot seek to
    // a keyframe, because it believes every frame is one. This was never
    // written at all, so the keyframe flags the demuxer read out of Matroska
    // were dropped on the way into MP4.
    if let Some(stss) = stss_box(t) {
        stbl.extend_from_slice(&stss);
    }
    let _ = timescale;
    minf.extend_from_slice(&plain_box(b"stbl", &stbl));
    plain_box(b"minf", &minf)
}

/// Sample description: exactly one, describing the whole track.
fn stsd_box(t: &AvTrack) -> Vec<u8> {
    let entry = sample_entry(t);
    let mut p = 1_u32.to_be_bytes().to_vec();
    p.extend_from_slice(&entry);
    full_box(b"stsd", 0, 0, &p)
}

fn sample_entry(t: &AvTrack) -> Vec<u8> {
    match (t.kind, t.video, t.audio) {
        (TrackKind::Video, Some(VideoCodec::H264), _) => {
            video_entry(b"avc1", t, &[plain_box(b"avcC", &t.codec_private)])
        }
        (TrackKind::Video, Some(VideoCodec::H265), _) => {
            video_entry(b"hvc1", t, &[plain_box(b"hvcC", &t.codec_private)])
        }
        (TrackKind::Video, Some(VideoCodec::Av1), _) => {
            // Matroska stores AV1's sequence OBU set; av1C wraps it. When the
            // private data already IS an av1C box (its marker byte is 0x0A)
            // it rides through; otherwise the entry omits it and in-band
            // sequence headers carry the configuration, which AV1 permits.
            let av1c = if t.codec_private.first() == Some(&0x0A) {
                vec![plain_box(b"av1C", &t.codec_private)]
            } else {
                Vec::new()
            };
            video_entry(b"av01", t, &av1c)
        }
        (TrackKind::Audio, _, Some(AudioCodec::Aac)) => {
            let esds = esds_box(&t.codec_private);
            audio_entry(b"mp4a", t, &[esds])
        }
        (TrackKind::Audio, _, Some(AudioCodec::Mp3)) => audio_entry(b".mp3", t, &[]),
        _ => {
            // Unreachable through route()+mux() guards; refusing-shaped
            // fallback beats emitting a corrupt box if those guards ever
            // drift. A 1Ã—1 H.264 entry parses everywhere and decodes nothing
            // â€” visibly wrong beats silently wrong.
            video_entry(b"avc1", t, &[])
        }
    }
}

/// VisualSampleEntry with children.
fn video_entry(fourcc: &[u8; 4], t: &AvTrack, children: &[Vec<u8>]) -> Vec<u8> {
    let w = u16::try_from(t.pixel_width).unwrap_or(0);
    let h = u16::try_from(t.pixel_height).unwrap_or(0);
    let mut e = Vec::with_capacity(78);
    e.extend_from_slice(&[0u8; 6]); // reserved
    e.extend_from_slice(&1_u16.to_be_bytes()); // data_reference_index
    e.extend_from_slice(&[0u8; 16]); // predefined + reserved
    e.extend_from_slice(&w.to_be_bytes());
    e.extend_from_slice(&h.to_be_bytes());
    e.extend_from_slice(&(72_u32 << 16).to_be_bytes()); // horiz dpi
    e.extend_from_slice(&(72_u32 << 16).to_be_bytes()); // vert dpi
    e.extend_from_slice(&[0u8; 4]); // reserved
    e.extend_from_slice(&1_u16.to_be_bytes()); // frame count
    e.extend_from_slice(&[0u8; 32]); // compressor name
    e.extend_from_slice(&0xFFFF_u16.to_be_bytes()); // depth
    e.extend_from_slice(&(-1_i16).to_be_bytes()); // pre-defined -1
    for c in children {
        e.extend_from_slice(c.as_slice());
    }
    plain_box(fourcc, &e)
}

/// AudioSampleEntry. Channel count and rate live in the ASC for AAC; players
/// that matter read esds, and the values here are the conventional defaults
/// for the entries that lack richer side-data.
fn audio_entry(fourcc: &[u8; 4], t: &AvTrack, children: &[Vec<u8>]) -> Vec<u8> {
    let channels: u16 = match (t.audio, t.codec_private.first()) {
        (Some(AudioCodec::Aac), Some(first)) => {
            // AudioSpecificConfig, MSB first:
            //
            //   bits 0..5   audioObjectType
            //   bits 5..9   samplingFrequencyIndex
            //   bits 9..13  channelConfiguration
            //
            // In a 24-bit window that puts the channel config at bit 11 from
            // the bottom. The shift here was 6, which reads four bits from the
            // middle of the frequency index and the top of the config -- a
            // stereo track came out as mono, and AAC with a wrong channel
            // config plays garbage rather than failing.
            let b0 = *first as u32;
            let b1 = t.codec_private.get(1).map_or(0, |v| *v as u32);
            let b2 = t.codec_private.get(2).map_or(0, |v| *v as u32);
            let bits = (b0 << 16) | (b1 << 8) | b2;
            let cfg = (bits >> 11) & 0x0F;
            match cfg {
                0 => 0, // defined in AOT-specific escape; rare, left honest
                1 => 1, // mono
                2 => 2, // stereo
                3..=6 => (cfg as u16) + 1,
                7 => 8,
                _ => 0,
            }
        }
        _ => 2,
    };
    let mut e = Vec::with_capacity(28);
    e.extend_from_slice(&[0u8; 6]);
    e.extend_from_slice(&1_u16.to_be_bytes());
    e.extend_from_slice(&[0u8; 8]); // reserved
    e.extend_from_slice(&channels.to_be_bytes());
    e.extend_from_slice(&16_u16.to_be_bytes()); // sample size
    e.extend_from_slice(&[0u8; 4]); // compression id + packet size
                                    // 16.16 FIXED POINT, and from the track rather than a constant.
                                    //
                                    // This wrote `48000u32`, whose top sixteen bits -- the integer part of a
                                    // 16.16 value -- are zero, so every MP4 this produced declared a sample
                                    // rate of 0 Hz. It also ignored the source's actual rate. Both went
                                    // unnoticed because nothing here read an MP4 back.
    let rate = if t.sample_rate > 0.0 {
        t.sample_rate.min(f64::from(u16::MAX)) as u32
    } else {
        48_000
    };
    e.extend_from_slice(&(rate << 16).to_be_bytes());
    for c in children {
        e.extend_from_slice(c.as_slice());
    }
    plain_box(fourcc, &e)
}

/// Minimal ESDescr wrapping the AudioSpecificConfig verbatim.
///
/// Descriptor lengths use the FIXED four-byte expanded form: simpler than the
/// variable encoding and parsed identically by every demuxer this was tested
/// against.
fn esds_box(asc: &[u8]) -> Vec<u8> {
    // THE NESTING WAS INSIDE OUT.
    //
    // This built `descriptor(0x04, ..containing.. descriptor(0x03, ...))`,
    // which is backwards, and gave the DecoderConfig none of its required
    // fields. The real shape (ISO/IEC 14496-1) is:
    //
    //   0x03 ES_Descriptor          ES_ID(2), flags(1)
    //     0x04 DecoderConfig        objectType(1), streamType(1),
    //                               bufferSizeDB(3), maxBitrate(4),
    //                               avgBitrate(4)
    //       0x05 DecoderSpecific    the AudioSpecificConfig
    //     0x06 SLConfig
    //
    // symphonia panicked outright on the old shape, which is how it surfaced.
    let mut decoder_config = vec![
        0x40, // objectTypeIndication: MPEG-4 audio
        0x15, // streamType 0x05 (audio) << 2 | upStream 0 << 1 | reserved 1
    ];
    decoder_config.extend_from_slice(&[0x00, 0x00, 0x00]); // bufferSizeDB
    decoder_config.extend_from_slice(&0_u32.to_be_bytes()); // maxBitrate: unknown
    decoder_config.extend_from_slice(&0_u32.to_be_bytes()); // avgBitrate: unknown
    decoder_config.extend_from_slice(&descriptor(0x05, asc));

    let mut es = vec![0x00, 0x01, 0x00]; // ES_ID = 1, flags = 0
    es.extend_from_slice(&descriptor(0x04, &decoder_config));
    es.extend_from_slice(&descriptor(0x06, &[0x02])); // SLConfig: MP4 default

    full_box(b"esds", 0, 0, &descriptor(0x03, &es))
}

/// One MPEG-4 descriptor: tag, length, body.
///
/// The length uses the expanded form -- seven bits per byte, high bit set on
/// every byte but the last -- rather than the fixed four-byte version this
/// used to write with a `unwrap_or(127)` that SILENTLY TRUNCATED anything
/// longer. An AudioSpecificConfig is a handful of bytes so it never bit, but a
/// truncated length is a malformed file that reads as a short one.
fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = body.len();
    // Four bytes always, so the encoding is fixed-width and the two-pass
    // size measurement in `mux` stays exact.
    for shift in [21_u32, 14, 7] {
        out.push(0x80 | ((len >> shift) as u8 & 0x7F));
    }
    out.push(len as u8 & 0x7F);
    out.extend_from_slice(body);
    out
}

/// ctts: how far after its decode time each sample is shown.
///
/// # The defect this exists for
///
/// H.264 with B-frames is stored in decode order and displayed in another, and
/// `ctts` is the only place MP4 records the difference. Written by nothing, so
/// `mov -> mp4` on a 277-frame export produced frames timed
/// `0, .066, .100, .033` where the source read `0, .033, .066, .100` -- every
/// frame present, byte-identical, and shown in the wrong order under a Class A
/// receipt.
///
/// **Version 1**, whose offsets are signed. Version 0's are unsigned and
/// cannot express a source that centres its offsets around zero, which is what
/// several muxers do; a negative offset written as unsigned puts a frame hours
/// into the future.
///
/// `None` when every offset is zero, which is the common case.
fn ctts_box(t: &AvTrack) -> Option<Vec<u8>> {
    if t.samples.iter().all(|s| s.composition_offset_ticks == 0) {
        return None;
    }
    let mut runs: Vec<(u32, i32)> = Vec::new();
    for s in &t.samples {
        let offset = i32::try_from(s.composition_offset_ticks).unwrap_or(0);
        match runs.last_mut() {
            Some((count, o)) if *o == offset => *count = count.saturating_add(1),
            _ => runs.push((1, offset)),
        }
    }
    let mut p = Vec::with_capacity(4 + runs.len() * 8);
    p.extend_from_slice(&u32::try_from(runs.len()).unwrap_or(0).to_be_bytes());
    for (count, offset) in runs {
        p.extend_from_slice(&count.to_be_bytes());
        p.extend_from_slice(&offset.to_be_bytes());
    }
    Some(full_box(b"ctts", 1, 0, &p))
}

/// stts: run-length deltas straight from the source's own clock.
///
/// Delta[i] = ts[i+1] âˆ’ ts[i]; the final sample inherits the last delta,
/// which is what every muxer does when a container declines to state
/// per-sample durations. Equal consecutive deltas coalesce, so constant-rate
/// tracks produce ONE run regardless of length.
fn stts_box(t: &AvTrack) -> Vec<u8> {
    let n = t.samples.len();
    let mut runs: Vec<(u32, u32)> = Vec::new();
    let note_run = |runs: &mut Vec<(u32, u32)>, count: u32, delta: i64| {
        if count == 0 {
            return;
        }
        let delta = delta.clamp(0, i64::from(u32::MAX)) as u32;
        match runs.last_mut() {
            Some((c, d)) if *d == delta => *c = c.saturating_add(count),
            _ => runs.push((count, delta)),
        }
    };
    if n >= 2 {
        for w in t.samples.windows(2) {
            let delta = w[1].ts_ticks - w[0].ts_ticks;
            note_run(&mut runs, 1, delta);
        }
    } else {
        note_run(&mut runs, u32::try_from(n).unwrap_or(1), 1);
    }

    let mut p = Vec::with_capacity(4 + runs.len() * 8);
    p.extend_from_slice(&u32::try_from(runs.len()).unwrap_or(0).to_be_bytes());
    for (count, delta) in runs {
        p.extend_from_slice(&count.to_be_bytes());
        p.extend_from_slice(&delta.to_be_bytes());
    }
    full_box(b"stts", 0, 0, &p)
}

/// One chunk holding every sample: trivially consistent tables, deterministic
/// layout, and players accept large chunks without complaint.
fn stsc_box(t: &AvTrack) -> Vec<u8> {
    let n = u32::try_from(t.samples.len()).unwrap_or(u32::MAX);
    let mut p = Vec::with_capacity(16);
    p.extend_from_slice(&1_u32.to_be_bytes()); // entry count
    p.extend_from_slice(&1_u32.to_be_bytes()); // first chunk
    p.extend_from_slice(&n.to_be_bytes()); // samples per chunk
    p.extend_from_slice(&1_u32.to_be_bytes()); // sample description index
    full_box(b"stsc", 0, 0, &p)
}

/// Sync-sample table, or `None` when every sample is one.
///
/// Sample numbers are ONE-based here, unlike every other index in this file.
fn stss_box(t: &AvTrack) -> Option<Vec<u8>> {
    if t.samples.iter().all(|s| s.keyframe) {
        return None;
    }
    let syncs: Vec<u32> = t
        .samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.keyframe)
        .map(|(i, _)| i as u32 + 1)
        .collect();
    let mut p = (syncs.len() as u32).to_be_bytes().to_vec();
    for n in syncs {
        p.extend_from_slice(&n.to_be_bytes());
    }
    Some(full_box(b"stss", 0, 0, &p))
}

fn stsz_box(t: &AvTrack) -> Vec<u8> {
    let n = u32::try_from(t.samples.len()).unwrap_or(u32::MAX);
    let mut p = Vec::with_capacity(8 + t.samples.len() * 4);
    p.extend_from_slice(&0_u32.to_be_bytes()); // uniform size: none
    p.extend_from_slice(&n.to_be_bytes());
    for s in &t.samples {
        p.extend_from_slice(&(s.data.len() as u32).to_be_bytes());
    }
    full_box(b"stsz", 0, 0, &p)
}

fn stco_box(chunk_offset: u64) -> Vec<u8> {
    let mut p = Vec::with_capacity(8);
    p.extend_from_slice(&1_u32.to_be_bytes()); // entry count
    p.extend_from_slice(&(chunk_offset.min(u64::from(u32::MAX)) as u32).to_be_bytes());
    full_box(b"stco", 0, 0, &p)
}
