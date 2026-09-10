//! MP4 demuxing: an ISO-BMFF file to the same [`AvGraph`] Matroska produces.
//!
//! # Why this exists
//!
//! `mp4mux.rs` has muxed for a long time and nothing read one back, so MP4 was
//! a destination and never a source: `mkv -> mp4` worked and `mp4 -> mkv` did
//! not exist. For the most common video container in the world that is the
//! wrong way round.
//!
//! Audio extraction from MP4 landed earlier through oc-audio, which decodes.
//! This is the Class A half — coded samples move byte-for-byte into another
//! container, and nothing is re-encoded.
//!
//! # The sample tables are the whole job
//!
//! Matroska stores each frame beside its timestamp. MP4 does not: it stores
//! the media in one `mdat` blob and describes it with five parallel tables in
//! `stbl`, each compressed in its own way.
//!
//! - `stts` — how long each sample lasts, run-length encoded
//! - `stsz` — how big each sample is, or one size for all of them
//! - `stsc` — how many samples are in each *chunk*, run-length encoded
//! - `stco` / `co64` — where each chunk starts (32- or 64-bit offsets)
//! - `stss` — which samples are sync points; ABSENT means they all are
//!
//! Reassembling a sample means walking all five at once. Every one of them is
//! attacker-controlled, so every index is checked rather than trusted: a
//! `stsc` claiming a million samples per chunk must cost an error, not an
//! allocation.

use openconvert_core::codec::{AudioCodec, VideoCodec};

use crate::matroska::{AvGraph, AvSample, AvTrack, TrackKind};

/// Why a file could not be demuxed.
#[derive(Debug, thiserror::Error)]
pub enum Mp4Error {
    /// Not an ISO-BMFF file, or missing the boxes that describe the media.
    #[error("this is not an MP4 this build can read: {0}")]
    Malformed(String),
    /// The file describes more sample data than the caller allowed.
    #[error("this file's samples exceed the {limit}-byte budget")]
    TooLarge {
        /// The budget that was exceeded.
        limit: u64,
    },
    /// Nothing in the file is a track we can carry.
    #[error("this MP4 has no audio or video track this build can carry")]
    NoTracks,
}

/// Boxes whose payload is more boxes.
///
/// A whitelist, not a guess: descending into an unrecognised box means reading
/// codec private data as though it were structure.
const fn is_container(kind: &[u8; 4]) -> bool {
    matches!(
        kind,
        b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"edts" | b"mvex"
    )
}

/// One box: its type, its payload range, and where the next one starts.
struct Boxes<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Iterator for Boxes<'a> {
    type Item = ([u8; 4], &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let b = self.bytes;
        if self.at + 8 > b.len() {
            return None;
        }
        let size32 =
            u32::from_be_bytes([b[self.at], b[self.at + 1], b[self.at + 2], b[self.at + 3]]);
        let mut kind = [0u8; 4];
        kind.copy_from_slice(&b[self.at + 4..self.at + 8]);
        let (header, size) = match size32 {
            1 => {
                if self.at + 16 > b.len() {
                    return None;
                }
                let mut n = [0u8; 8];
                n.copy_from_slice(&b[self.at + 8..self.at + 16]);
                (16usize, usize::try_from(u64::from_be_bytes(n)).ok()?)
            }
            0 => (8usize, b.len() - self.at),
            n => (8usize, n as usize),
        };
        // A box smaller than its own header, or bigger than what is left, is a
        // lie. Stop rather than clamp: clamping turns a malformed file into a
        // differently-malformed one and keeps reading it.
        if size < header || self.at + size > b.len() {
            return None;
        }
        let payload = &b[self.at + header..self.at + size];
        self.at += size;
        Some((kind, payload))
    }
}

const fn boxes(bytes: &[u8]) -> Boxes<'_> {
    Boxes { bytes, at: 0 }
}

/// Find one box by path, descending only through container boxes.
fn find<'a>(bytes: &'a [u8], path: &[&[u8; 4]]) -> Option<&'a [u8]> {
    let Some((first, rest)) = path.split_first() else {
        return Some(bytes);
    };
    for (kind, payload) in boxes(bytes) {
        if &kind == *first {
            return if rest.is_empty() {
                Some(payload)
            } else {
                find(payload, rest)
            };
        }
        if is_container(&kind) {
            if let Some(found) = find(payload, path) {
                return Some(found);
            }
        }
    }
    None
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8)
        .map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// What became of one track.
///
/// Dropping a track used to be `Ok(None)`, which is the same value for
/// "subtitles", "timecode" and "LPCM audio this build cannot carry" -- and the
/// caller could not tell the user which. A professional `.mov` is H.264 video
/// beside LPCM audio and a timecode track, so that silence took the AUDIO out
/// of the file under a Class A receipt saying nothing was lost.
enum Track {
    /// Carried through, byte-identical.
    Carried(Box<AvTrack>),
    /// Not carried, and why, phrased for the receipt.
    Dropped(String),
}

/// Demux an MP4 into the shared graph, and say what did not come with it.
///
/// `budget` bounds the total sample bytes retained, exactly as the Matroska
/// demuxer's does.
///
/// The second half of the pair is the receipt's `removed` lines for tracks
/// this build cannot carry. Empty means every track in the file is in the
/// graph, which is what Class A claims.
///
/// # Errors
///
/// [`Mp4Error`] — a malformed file, a budget overrun, or nothing carryable.
pub fn demux(bytes: &[u8], budget: u64) -> Result<(AvGraph, Vec<String>), Mp4Error> {
    let moov = find(bytes, &[b"moov"]).ok_or_else(|| Mp4Error::Malformed("no moov box".into()))?;

    let mut tracks = Vec::new();
    let mut dropped = Vec::new();
    let mut retained: u64 = 0;
    let mut number = 1u64;

    for (kind, trak) in boxes(moov) {
        if &kind != b"trak" {
            continue;
        }
        match read_track(trak, bytes, number, budget, &mut retained)? {
            Track::Carried(track) => {
                tracks.push(*track);
                number += 1;
            }
            Track::Dropped(why) => dropped.push(why),
        }
    }
    if tracks.is_empty() {
        return Err(Mp4Error::NoTracks);
    }
    Ok((
        AvGraph {
            tracks,
            // Milliseconds, matching what `graph_to_ebml` writes and what the
            // Matroska demuxer produces, so a graph from either source is the
            // same shape.
            timecode_scale_ns: 1_000_000,
        },
        dropped,
    ))
}

#[allow(clippy::too_many_lines)]
fn read_track(
    trak: &[u8],
    file: &[u8],
    number: u64,
    budget: u64,
    retained: &mut u64,
) -> Result<Track, Mp4Error> {
    let mdhd = find(trak, &[b"mdia", b"mdhd"])
        .ok_or_else(|| Mp4Error::Malformed("a track has no mdhd".into()))?;
    // version 0: timescale at 12; version 1: 64-bit times push it to 20.
    let timescale = match mdhd.first() {
        Some(0) => be32(mdhd, 12),
        Some(1) => be32(mdhd, 20),
        _ => None,
    }
    .filter(|&t| t != 0)
    .ok_or_else(|| Mp4Error::Malformed("a track declares no timescale".into()))?;

    let hdlr = find(trak, &[b"mdia", b"hdlr"])
        .ok_or_else(|| Mp4Error::Malformed("a track has no hdlr".into()))?;
    let handler = hdlr.get(8..12).unwrap_or(&[0; 4]);
    let kind = match handler {
        b"soun" => TrackKind::Audio,
        b"vide" => TrackKind::Video,
        // Subtitles, timed metadata and chapter tracks are dropped. Carrying
        // a track whose bytes nothing here understands would put an
        // unidentified stream in the output under a Class A receipt.
        //
        // The handler is NAMED in the reason: `tmcd` on a QuickTime export is
        // a timecode track and "a timed metadata track" would leave the reader
        // guessing which of several it was.
        other => {
            return Ok(Track::Dropped(format!(
            "a `{}` track, which is neither audio nor video and has no place in the destination",
            String::from_utf8_lossy(other)
        )))
        }
    };

    let stbl = find(trak, &[b"mdia", b"minf", b"stbl"])
        .ok_or_else(|| Mp4Error::Malformed("a track has no sample table".into()))?;
    let stsd = find(stbl, &[b"stsd"])
        .ok_or_else(|| Mp4Error::Malformed("a track has no sample description".into()))?;

    // stsd: version/flags (4), entry count (4), then the entries.
    let entry = stsd
        .get(8..)
        .filter(|e| e.len() >= 8)
        .ok_or_else(|| Mp4Error::Malformed("an empty sample description".into()))?;
    let mut fourcc = [0u8; 4];
    fourcc.copy_from_slice(&entry[4..8]);

    let (codec_id, video, audio) = map_codec(&fourcc, kind);
    // An unidentified codec is not carried. `codec::carried_by` treats
    // `None` as uncarryable everywhere, and emitting a track whose CodecID we
    // invented would be worse than refusing it.
    //
    // The FOURCC is in the reason, because that is the fact: `lpcm` audio and
    // `apcn` ProRes are dropped for entirely different reasons a user might
    // act on, and "an unsupported track" tells them neither.
    if codec_id.is_empty() {
        return Ok(Track::Dropped(format!(
            "the {} track coded as `{}`, which this build cannot carry",
            match kind {
                TrackKind::Audio => "audio",
                TrackKind::Video => "video",
                _ => "other",
            },
            String::from_utf8_lossy(&fourcc)
        )));
    }

    // WHERE THE NESTED BOXES START, counted from the sample entry's own box
    // header. Both numbers are the spec's, and both were wrong first time —
    // an audio entry read at 28 reported a 2 Hz sample rate.
    //
    //   SampleEntry        = 8 (box) + 6 reserved + 2 data_reference_index = 16
    //   AudioSampleEntry  += 2 version + 2 revision + 4 vendor + 2 channels
    //                        + 2 samplesize + 2 compression + 2 packet
    //                        + 4 samplerate                              = 36
    //   VisualSampleEntry += 2 + 2 + 12 + 2 w + 2 h + 4 + 4 + 4 + 2
    //                        + 32 compressorname + 2 depth + 2           = 86
    //
    // AND THE AUDIO ENTRY HAS VERSIONS, which QuickTime uses and MP4 does not.
    // The version is the first field after the 16-byte SampleEntry, and each
    // one appends to the layout before it:
    //
    //   v0  the 20 bytes above                     -> children at 36
    //   v1  + samplesPerPacket, bytesPerPacket,
    //         bytesPerFrame, bytesPerSample (16)   -> children at 52
    //   v2  + a 36-byte descriptor                 -> children at 72
    //
    // A `.mov` from ffmpeg writes v1, so reading its children at 36 landed in
    // the middle of `samplesPerPacket` and found no boxes at all. The AAC
    // config came back EMPTY, and an MP4 muxed from that config is a file no
    // player will open -- which is exactly what `mov -> mp4` produced for
    // every file with an audio track.
    //
    // v2's own geometry fields are read below at their v0 offsets, which is
    // wrong for v2 -- it moves the sample rate to a 64-bit float. Not fixed
    // here and not silently ignored either: v2 exists for high-bit-depth and
    // multichannel PCM, which `map_codec` refuses before this point, so no v2
    // entry reaches this code with a codec we carry.
    let version = if kind == TrackKind::Audio {
        entry
            .get(16..18)
            .map_or(0, |b| u16::from_be_bytes([b[0], b[1]]))
    } else {
        0
    };
    let inner_at = match kind {
        TrackKind::Audio => audio_children_at(version),
        _ => 86,
    };
    let codec_private = entry
        .get(inner_at..)
        .map(|rest| config_in(rest, true))
        .unwrap_or_default();

    // Audio geometry, for the Audio element the Matroska writer emits. The
    // sample rate is 16.16 fixed point, so the integer part is the top half.
    let u16_at = |o: usize| -> u64 {
        entry
            .get(o..o + 2)
            .map_or(0, |b| u64::from(u16::from_be_bytes([b[0], b[1]])))
    };
    let (sample_rate, channels, bit_depth) = if kind == TrackKind::Audio {
        (
            f64::from(be32(entry, 32).unwrap_or(0) >> 16),
            u16_at(24),
            u16_at(26),
        )
    } else {
        (0.0, 0, 0)
    };
    let (pixel_width, pixel_height) = if kind == TrackKind::Video {
        (
            u32::try_from(u16_at(32)).unwrap_or(0),
            u32::try_from(u16_at(34)).unwrap_or(0),
        )
    } else {
        (0, 0)
    };

    let samples = read_samples(stbl, file, timescale, budget, retained)?;
    if samples.is_empty() {
        return Ok(Track::Dropped(format!(
            "an empty `{}` track",
            String::from_utf8_lossy(&fourcc)
        )));
    }

    Ok(Track::Carried(Box::new(AvTrack {
        number,
        kind,
        video,
        audio,
        codec_id,
        codec_private,
        pixel_width,
        pixel_height,
        sample_rate,
        channels,
        bit_depth,
        samples,
    })))
}

/// Where an AudioSampleEntry's child boxes begin, by its version.
///
/// See the comment at the call site for what each version appends and for the
/// `.mov` that made this necessary. Split out so it can be tested: the numbers
/// are the kind that are wrong by sixteen and produce an empty codec config
/// rather than an error.
const fn audio_children_at(version: u16) -> usize {
    match version {
        1 => 52,
        2 => 72,
        _ => 36,
    }
}

/// The decoder configuration among a sample entry's child boxes.
///
/// `descend` allows one level into QuickTime's `wave` atom, which is where a
/// `.mov` puts the `esds` that an `.mp4` puts directly in the sample entry.
/// One level, not a recursive walk: `wave` is the only wrapper this needs, and
/// a general descent through boxes we do not recognise is the thing `boxes`
/// exists to avoid.
fn config_in(rest: &[u8], descend: bool) -> Vec<u8> {
    for (k, payload) in boxes(rest) {
        match &k {
            b"avcC" | b"hvcC" | b"vpcC" | b"av1C" | b"dOps" | b"alac" => return payload.to_vec(),
            b"esds" => return asc_from_esds(payload),
            b"wave" if descend => {
                let inner = config_in(payload, false);
                if !inner.is_empty() {
                    return inner;
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

/// The AudioSpecificConfig inside an `esds` descriptor.
///
/// Matroska's CodecPrivate for `A_AAC` is the bare config, not the descriptor
/// tree around it. The tags use a variable-length size where the top bit
/// continues, so the walk is over bytes rather than a fixed layout.
fn asc_from_esds(esds: &[u8]) -> Vec<u8> {
    // Skip version+flags, then walk descriptors looking for tag 0x05.
    let mut at = 4usize;
    while at < esds.len() {
        let tag = esds[at];
        at += 1;
        let mut len = 0usize;
        for _ in 0..4 {
            let Some(&b) = esds.get(at) else {
                return Vec::new();
            };
            at += 1;
            len = (len << 7) | usize::from(b & 0x7F);
            if b & 0x80 == 0 {
                break;
            }
        }
        match tag {
            // ES_Descriptor: skip its own header, then keep walking.
            0x03 => at += 3,
            // DecoderConfigDescriptor: skip to its nested descriptors.
            0x04 => at += 13,
            // DecoderSpecificInfo: this is the config.
            0x05 => return esds.get(at..at + len).unwrap_or(&[]).to_vec(),
            _ => at += len,
        }
    }
    Vec::new()
}

/// Map an MP4 sample-entry type to a Matroska CodecID.
///
/// Returning an empty id means "not carried", which the caller turns into a
/// dropped track rather than an invented one.
fn map_codec(
    fourcc: &[u8; 4],
    kind: TrackKind,
) -> (Vec<u8>, Option<VideoCodec>, Option<AudioCodec>) {
    match (kind, fourcc) {
        (TrackKind::Video, b"avc1" | b"avc3") => {
            (b"V_MPEG4/ISO/AVC".to_vec(), Some(VideoCodec::H264), None)
        }
        (TrackKind::Video, b"hev1" | b"hvc1") => {
            (b"V_MPEGH/ISO/HEVC".to_vec(), Some(VideoCodec::H265), None)
        }
        (TrackKind::Video, b"av01") => (b"V_AV1".to_vec(), Some(VideoCodec::Av1), None),
        (TrackKind::Video, b"vp09") => (b"V_VP9".to_vec(), Some(VideoCodec::Vp9), None),
        (TrackKind::Audio, b"mp4a") => (b"A_AAC".to_vec(), None, Some(AudioCodec::Aac)),
        (TrackKind::Audio, b"alac") => (b"A_ALAC".to_vec(), None, Some(AudioCodec::Alac)),
        (TrackKind::Audio, b"Opus") => (b"A_OPUS".to_vec(), None, Some(AudioCodec::Opus)),
        (TrackKind::Audio, b"fLaC") => (b"A_FLAC".to_vec(), None, Some(AudioCodec::Flac)),
        _ => (Vec::new(), None, None),
    }
}

/// Walk the five sample tables together and cut the samples out of the file.
fn read_samples(
    stbl: &[u8],
    file: &[u8],
    timescale: u32,
    budget: u64,
    retained: &mut u64,
) -> Result<Vec<AvSample>, Mp4Error> {
    let malformed = |m: &str| Mp4Error::Malformed(m.to_string());

    // ---- sizes ----
    let stsz = find(stbl, &[b"stsz"]).ok_or_else(|| malformed("no stsz"))?;
    let uniform = be32(stsz, 4).ok_or_else(|| malformed("short stsz"))?;
    let count = be32(stsz, 8).ok_or_else(|| malformed("short stsz"))? as usize;
    let sizes: Vec<u32> = if uniform > 0 {
        vec![uniform; count]
    } else {
        (0..count)
            .map(|i| be32(stsz, 12 + i * 4).ok_or_else(|| malformed("truncated stsz")))
            .collect::<Result<_, _>>()?
    };

    // ---- chunk offsets ----
    let (offsets, wide) = match (find(stbl, &[b"stco"]), find(stbl, &[b"co64"])) {
        (Some(stco), _) => (stco, false),
        (None, Some(co64)) => (co64, true),
        (None, None) => return Err(malformed("no chunk offsets")),
    };
    let n_chunks = be32(offsets, 4).ok_or_else(|| malformed("short chunk table"))? as usize;
    let chunk_at: Vec<u64> = (0..n_chunks)
        .map(|i| {
            if wide {
                be64(offsets, 8 + i * 8)
            } else {
                be32(offsets, 8 + i * 4).map(u64::from)
            }
            .ok_or_else(|| malformed("truncated chunk table"))
        })
        .collect::<Result<_, _>>()?;

    // ---- samples per chunk ----
    let stsc = find(stbl, &[b"stsc"]).ok_or_else(|| malformed("no stsc"))?;
    let n_runs = be32(stsc, 4).ok_or_else(|| malformed("short stsc"))? as usize;
    let mut runs = Vec::with_capacity(n_runs.min(1 << 16));
    for i in 0..n_runs {
        let first = be32(stsc, 8 + i * 12).ok_or_else(|| malformed("truncated stsc"))?;
        let per = be32(stsc, 12 + i * 12).ok_or_else(|| malformed("truncated stsc"))?;
        runs.push((first as usize, per as usize));
    }

    // ---- durations ----
    let stts = find(stbl, &[b"stts"]).ok_or_else(|| malformed("no stts"))?;
    let n_dur = be32(stts, 4).ok_or_else(|| malformed("short stts"))? as usize;
    let mut durations: Vec<(usize, u64)> = Vec::with_capacity(n_dur.min(1 << 16));
    for i in 0..n_dur {
        let n = be32(stts, 8 + i * 8).ok_or_else(|| malformed("truncated stts"))? as usize;
        let d = be32(stts, 12 + i * 8).ok_or_else(|| malformed("truncated stts"))?;
        durations.push((n, u64::from(d)));
    }

    // ---- composition offsets ----
    //
    // `ctts` is how MP4 says "this sample is DECODED here and SHOWN there",
    // which is the whole of B-frame reordering. Read by nothing until now, so
    // every conversion out of an MP4 or a MOV wrote decode times as if they
    // were presentation times: the frames were all present, all byte-identical
    // and all shown in the wrong order.
    //
    // NEGATIVE OFFSETS ARRIVE IN VERSION 0 TABLES, whatever 14496-12 says.
    //
    // The spec makes v0's offsets unsigned and v1's signed. Real muxers centre
    // their offsets around zero and write the negative ones into a v0 table
    // anyway -- the sample `.mov` here is ffmpeg's own output, version 0,
    // carrying -512. Read as unsigned that is 4294966784, which after the
    // timescale becomes 279620 SECONDS, and the frame lands three days into
    // the file.
    //
    // So the version is not the deciding fact; the value is. Anything with the
    // top bit set is read as signed, because a genuine offset above 2^31 ticks
    // would be tens of hours of reordering in a file whose whole duration is
    // seconds. This is what every player that opens these files does.
    //
    // Absent is the common case and means every offset is zero.
    let ctts: Vec<(usize, i64)> = find(stbl, &[b"ctts"]).map_or_else(Vec::new, |ctts| {
        let n = be32(ctts, 4).unwrap_or(0) as usize;
        (0..n.min(1 << 20))
            .filter_map(|i| {
                let count = be32(ctts, 8 + i * 8)? as usize;
                let raw = be32(ctts, 12 + i * 8)?;
                Some((count, i64::from(raw as i32)))
            })
            .collect()
    });

    // ---- sync samples ----
    //
    // An ABSENT stss means every sample is a sync point. Treating absence as
    // "none are" would mark a whole audio track as non-keyframe and make any
    // later trim unable to cut anywhere.
    let sync: Option<Vec<u32>> = find(stbl, &[b"stss"]).and_then(|stss| {
        let n = be32(stss, 4)? as usize;
        (0..n).map(|i| be32(stss, 8 + i * 4)).collect()
    });

    // ---- walk ----
    let mut out = Vec::with_capacity(count.min(1 << 20));
    let mut sample = 0usize; // zero-based index
    let mut ticks: u64 = 0;
    let mut dur_run = 0usize;
    let mut dur_left = durations.first().map_or(0, |d| d.0);
    let mut ct_run = 0usize;
    let mut ct_left = ctts.first().map_or(0, |c| c.0);

    for (ci, &chunk_start) in chunk_at.iter().enumerate() {
        // How many samples this chunk holds: the last run whose first_chunk
        // is at or before this one. Chunk numbers in stsc are ONE-based.
        let per = runs
            .iter()
            .rev()
            .find(|(first, _)| *first <= ci + 1)
            .map_or(0, |(_, per)| *per);
        let mut at = chunk_start;
        for _ in 0..per {
            if sample >= sizes.len() {
                break;
            }
            let size = sizes[sample] as usize;
            let start = usize::try_from(at).map_err(|_| malformed("a chunk offset past memory"))?;
            let end = start
                .checked_add(size)
                .ok_or_else(|| malformed("a sample size past memory"))?;
            let data = file
                .get(start..end)
                .ok_or_else(|| malformed("a sample points outside the file"))?;

            *retained += size as u64;
            if *retained > budget {
                return Err(Mp4Error::TooLarge { limit: budget });
            }

            // Duration runs are (count, delta) pairs; step through them in
            // lockstep with the samples rather than expanding them.
            while dur_left == 0 && dur_run + 1 < durations.len() {
                dur_run += 1;
                dur_left = durations[dur_run].0;
            }
            let delta = durations.get(dur_run).map_or(0, |d| d.1);
            dur_left = dur_left.saturating_sub(1);

            // The same run-length walk, over the composition table.
            while ct_left == 0 && ct_run + 1 < ctts.len() {
                ct_run += 1;
                ct_left = ctts[ct_run].0;
            }
            let ct_offset = if ct_left == 0 {
                0
            } else {
                ctts.get(ct_run).map_or(0, |c| c.1)
            };
            ct_left = ct_left.saturating_sub(1);

            let keyframe = sync
                .as_ref()
                .is_none_or(|s| s.binary_search(&(sample as u32 + 1)).is_ok());

            out.push(AvSample {
                // Track timescale to milliseconds, which is the graph's unit.
                ts_ticks: i64::try_from(ticks * 1000 / u64::from(timescale)).unwrap_or(i64::MAX),
                composition_offset_ticks: ct_offset * 1000 / i64::from(timescale),
                keyframe,
                data: data.to_vec(),
            });
            ticks += delta;
            at += size as u64;
            sample += 1;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{demux, Mp4Error};
    use crate::matroska::{AvGraph, AvSample, AvTrack, TrackKind};
    use openconvert_core::codec::{AudioCodec, VideoCodec};

    /// A graph the muxer will accept, with distinguishable sample payloads.
    fn graph(samples_per_track: usize) -> AvGraph {
        let mk = |n: u64, kind: TrackKind| AvTrack {
            number: n,
            kind,
            video: (kind == TrackKind::Video).then_some(VideoCodec::H264),
            audio: (kind == TrackKind::Audio).then_some(AudioCodec::Aac),
            codec_id: if kind == TrackKind::Video {
                b"V_MPEG4/ISO/AVC".to_vec()
            } else {
                b"A_AAC".to_vec()
            },
            // Non-empty, because it has to survive the round trip too.
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
            samples: (0..samples_per_track)
                .map(|i| AvSample {
                    ts_ticks: (i as i64) * 20,
                    // NOT ZERO for the video track, so the `ctts` round trip is
                    // exercised. A fixture whose every offset is zero would
                    // pass with the table written by nothing, which is how the
                    // frame reordering shipped in the first place. The pattern
                    // alternates sign, because a real muxer's do.
                    composition_offset_ticks: if kind == TrackKind::Video {
                        [0_i64, 40, -20][i % 3]
                    } else {
                        0
                    },
                    // Every sample a different LENGTH and different CONTENT,
                    // so a stsz/stsc/stco walk that is off by one shows up as
                    // wrong bytes rather than as the right bytes shuffled.
                    keyframe: i % 3 == 0,
                    data: vec![(n as u8) << 4 | (i as u8 & 0x0F); 7 + i * 3],
                })
                .collect(),
        };
        AvGraph {
            tracks: vec![mk(1, TrackKind::Video), mk(2, TrackKind::Audio)],
            timecode_scale_ns: 1_000_000,
        }
    }

    /// The five sample tables, walked together, must reproduce the samples.
    ///
    /// `mp4mux` and this demuxer were written years apart and by different
    /// hands, so a round trip through both is a real check rather than a
    /// tautology: it is the sample-table arithmetic (`stts`, `stsz`, `stsc`,
    /// `stco`, `stss`) that has to agree, and nothing shares that code.
    #[test]
    fn every_sample_survives_a_mux_demux_round_trip() {
        let before = graph(9);
        let (mp4, _) = crate::mp4mux::mux(&before).expect("mux");
        let (after, dropped) = demux(&mp4, 1 << 20).expect("demux");

        assert!(dropped.is_empty(), "nothing should be dropped: {dropped:?}");
        assert_eq!(after.tracks.len(), before.tracks.len(), "track count");
        for (a, b) in after.tracks.iter().zip(&before.tracks) {
            assert_eq!(a.kind, b.kind, "track kind");
            assert_eq!(a.samples.len(), b.samples.len(), "sample count");
            for (i, (x, y)) in a.samples.iter().zip(&b.samples).enumerate() {
                assert_eq!(x.data, y.data, "sample {i} payload");
                assert_eq!(x.keyframe, y.keyframe, "sample {i} sync flag");
                // WHEN each sample is shown, not just which bytes it holds.
                // `ctts` was written by nothing and read by nothing, so every
                // frame survived in the wrong display order; the fixture above
                // gives the video track non-zero offsets of both signs so this
                // line has something to fail on.
                assert_eq!(
                    x.composition_offset_ticks, y.composition_offset_ticks,
                    "sample {i} composition offset"
                );
            }
        }
    }

    /// The three audio sample-entry layouts, and the one that was assumed.
    ///
    /// A `.mov` writes version 1 and its children start sixteen bytes later
    /// than version 0's. Read at 36, the walk landed inside `samplesPerPacket`,
    /// found no boxes, and returned an EMPTY AAC config -- from which `mp4mux`
    /// produced a file no player would open.
    #[test]
    fn an_audio_entry_s_children_start_after_its_version_s_fields() {
        assert_eq!(super::audio_children_at(0), 36);
        assert_eq!(super::audio_children_at(1), 52);
        assert_eq!(super::audio_children_at(2), 72);
        // An unknown version reads as the base layout rather than refusing:
        // the boxes are still there and the walk below either finds them or
        // returns nothing.
        assert_eq!(super::audio_children_at(7), 36);
    }

    /// QuickTime wraps the `esds` in a `wave` atom; MP4 does not.
    #[test]
    fn the_decoder_config_is_found_inside_a_wave_atom() {
        // An esds carrying one DecoderSpecificInfo (tag 0x05) of two bytes.
        let esds = {
            let body = [
                0x00, 0x00, 0x00, 0x00, // version + flags
                0x05, 0x02, 0x12, 0x08, // tag 5, length 2, the config
            ];
            let mut b = (8u32 + body.len() as u32).to_be_bytes().to_vec();
            b.extend_from_slice(b"esds");
            b.extend_from_slice(&body);
            b
        };
        let wave = {
            let mut b = (8u32 + esds.len() as u32).to_be_bytes().to_vec();
            b.extend_from_slice(b"wave");
            b.extend_from_slice(&esds);
            b
        };

        assert_eq!(super::config_in(&esds, true), vec![0x12, 0x08], "bare esds");
        assert_eq!(
            super::config_in(&wave, true),
            vec![0x12, 0x08],
            "esds inside a wave atom"
        );
        // One level only. A `wave` inside a `wave` is not a shape any file
        // has, and descending forever through boxes we do not recognise is
        // what `boxes` exists to prevent.
        assert!(super::config_in(&wave, false).is_empty());
    }

    /// Audio geometry has to come back, because the Matroska writer emits it.
    ///
    /// The sample entry's fields sit at fixed offsets that are easy to be
    /// wrong about: reading the audio entry at 28 instead of 36 reported a
    /// 2 Hz sample rate, which no test would have caught by counting samples.
    #[test]
    fn audio_geometry_survives() {
        let (mp4, _) = crate::mp4mux::mux(&graph(4)).expect("mux");
        let (after, _) = demux(&mp4, 1 << 20).expect("demux");
        let audio = after
            .tracks
            .iter()
            .find(|t| t.kind == TrackKind::Audio)
            .expect("an audio track");
        assert!(
            (audio.sample_rate - 48_000.0).abs() < 1.0,
            "sample rate came back as {}",
            audio.sample_rate
        );
        assert_eq!(audio.channels, 2, "channel count");
        assert!(!audio.codec_private.is_empty(), "codec config was dropped");
    }

    /// The budget bounds what is retained, and refuses rather than allocating.
    #[test]
    fn the_sample_budget_refuses_rather_than_growing() {
        let (mp4, _) = crate::mp4mux::mux(&graph(40)).expect("mux");
        assert!(
            matches!(demux(&mp4, 64), Err(Mp4Error::TooLarge { .. })),
            "a 64-byte budget must refuse a file with far more sample data"
        );
    }

    /// Every prefix of a valid file must produce an error, never a panic.
    ///
    /// The sample tables are attacker-controlled and indexed against each
    /// other, so a truncated one is the shape most likely to walk off an end.
    #[test]
    fn truncation_never_panics() {
        let (mp4, _) = crate::mp4mux::mux(&graph(6)).expect("mux");
        for cut in (0..mp4.len()).step_by(17) {
            let _ = demux(&mp4[..cut], 1 << 20);
        }
        // And a few hostile shapes that are not simple truncations.
        assert!(demux(b"", 1 << 20).is_err());
        assert!(demux(b"not an mp4 at all", 1 << 20).is_err());
        let mut no_moov = mp4.clone();
        if let Some(at) = no_moov.windows(4).position(|w| w == b"moov") {
            no_moov[at..at + 4].copy_from_slice(b"xxxx");
        }
        assert!(
            demux(&no_moov, 1 << 20).is_err(),
            "a file with no moov must refuse"
        );
    }
}
