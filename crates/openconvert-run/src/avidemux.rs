//! Reading an AVI into the shared graph.
//!
//! # What the spike found, and what it decided
//!
//! Four AVI files were walked before this module was written — H.264 with MP3,
//! H.264 with PCM, MPEG-4 Part 2 with MP3, and Motion JPEG — and the RIFF half
//! turned out to be the easy half. `hdrl` holds one `strl` per stream, each
//! with a `strh` (rate and kind) and a `strf` (a `BITMAPINFOHEADER` or a
//! `WAVEFORMATEX`); `movi` holds the coded samples as `##dc` and `##wb` chunks
//! in playback order. That is a morning's work.
//!
//! **The bitstream is the real job.** AVI stores H.264 as *Annex-B*: NAL units
//! separated by `00 00 01` start codes, with the SPS and PPS in band. MP4 needs
//! *AVCC*: each NAL prefixed by its length, and the SPS and PPS lifted out into
//! an `avcC` configuration box in the sample entry. The `strf` extradata in
//! every file spiked was **empty**, so there is no `avcC` to copy — it has to
//! be built from the parameter sets found in the stream. [`annexb`] is that.
//!
//! # Which streams carry
//!
//! H.264 video and MP3 audio, which between them are what a `.avi` worth
//! converting holds. Everything else is **dropped and named** — Motion JPEG,
//! MPEG-4 Part 2, PCM — by exactly the rule `mp4demux` follows: an empty
//! CodecID means "not carried", and a file with nothing carryable is refused
//! rather than emptied.
//!
//! MPEG-4 Part 2 and Motion JPEG both have MP4 sample entries and could be
//! added. They are not here because `mp4mux` writes neither, and a demuxer that
//! produced tracks the muxer drops would be a route that half-works.
//!
//! # Timestamps
//!
//! AVI records none per sample. It records a stream RATE, and the rest is the
//! chunk's position: a video sample's time is its index times the frame
//! duration, and an audio sample's is the bytes before it divided by the byte
//! rate. That is what every AVI reader does, and it is why an AVI cannot carry
//! composition offsets — there is nowhere to put them, so B-frame reordering
//! information is simply absent from the container and the graph's offsets stay
//! zero.

use crate::matroska::{AvGraph, AvSample, AvTrack, TrackKind};
use openconvert_core::codec::{AudioCodec, VideoCodec};

/// Why a file could not be demuxed.
#[derive(Debug, thiserror::Error)]
pub enum AviError {
    /// The structure is not what it claims.
    #[error("this AVI is malformed: {0}")]
    Malformed(String),
    /// The sample bytes would exceed the caller's budget.
    #[error("this AVI's streams exceed the {limit}-byte budget")]
    TooLarge {
        /// The budget that was crossed.
        limit: u64,
    },
    /// Nothing in it can be carried into another container.
    #[error("this AVI has no audio or video track this build can carry")]
    NoTracks,
}

/// The most chunks walked, so a malformed `movi` cannot spin.
const MAX_CHUNKS: usize = 4_000_000;

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

/// One RIFF chunk, as **absolute offsets into the whole file**.
///
/// Offsets rather than slices, and that is not a style choice. The first
/// version of this carried `&[u8]` bodies and recovered positions with
/// `bytes.len() - body.len()`, which is only correct for a slice that runs to
/// the end of the file — so every nested walk started in the wrong place and
/// the demuxer reported "no streams" for every AVI ever written. An offset
/// cannot be wrong about where it is.
#[derive(Clone, Copy)]
struct Chunk {
    id: [u8; 4],
    /// `LIST`/`RIFF` only: the four bytes naming what kind of list it is.
    form: Option<[u8; 4]>,
    /// Where this chunk's contents begin — after the form, for a list.
    from: usize,
    /// Where they end.
    to: usize,
}

/// Walk the chunks between `at` and `end`.
///
/// A scanner, bounded and non-recursive: every advance is by a size the file
/// states, checked against the end, so a chunk claiming four gigabytes inside a
/// forty-kilobyte file stops the walk rather than indexing past it.
fn chunks(buf: &[u8], mut at: usize, end: usize) -> Vec<Chunk> {
    let end = end.min(buf.len());
    let mut out = Vec::new();
    while at + 8 <= end && out.len() < MAX_CHUNKS {
        let Some(id) = buf.get(at..at + 4).and_then(|s| s.try_into().ok()) else {
            break;
        };
        let Some(size) = le32(buf, at + 4).map(|s| s as usize) else {
            break;
        };
        let body_at = at + 8;
        let body_end = body_at.saturating_add(size).min(end);
        let is_list = &id == b"LIST" || &id == b"RIFF";
        let form = if is_list {
            buf.get(body_at..body_at + 4)
                .and_then(|s| s.try_into().ok())
        } else {
            None
        };
        // A list's contents start after its form. Every chunk is padded to an
        // even length, which is the one thing about RIFF that bites.
        let from = if is_list {
            (body_at + 4).min(body_end)
        } else {
            body_at
        };
        out.push(Chunk {
            id,
            form,
            from,
            to: body_end,
        });
        let next = body_at.saturating_add(size) + (size & 1);
        if next <= at {
            break; // a zero-size chunk would spin forever
        }
        at = next;
    }
    out
}

/// One stream's header pair, as `hdrl` gives it.
struct Stream {
    kind: TrackKind,
    /// `dwScale` and `dwRate` from `strh`: the stream's clock, in units per
    /// second.
    scale: u32,
    rate: u32,
    /// `dwSampleSize`, which says what the clock COUNTS.
    ///
    /// Zero means one chunk is one unit — the case for all video and for
    /// variable-bitrate audio. Non-zero means the unit is that many bytes, so
    /// a chunk holding several of them advances the clock by several.
    ///
    /// Reading this wrong is not subtle: a two-second file with a
    /// byte-counted clock applied to a chunk-counted stream came out claiming
    /// **420 seconds**.
    sample_size: u32,
    /// The `strf` payload — a `BITMAPINFOHEADER` or a `WAVEFORMATEX`.
    format: Vec<u8>,
}

/// Demux an AVI into the shared graph, and say what did not come with it.
///
/// `budget` bounds the total sample bytes retained, exactly as the Matroska and
/// MP4 demuxers' do.
///
/// # Errors
///
/// [`AviError`] — a malformed file, a budget overrun, or nothing carryable.
pub fn demux(bytes: &[u8], budget: u64) -> Result<(AvGraph, Vec<String>), AviError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"AVI " {
        return Err(AviError::Malformed("not a RIFF AVI".into()));
    }

    // EVERY TOP-LEVEL RIFF, not just the first. A file over two gigabytes is
    // OpenDML and continues in `RIFF AVIX` segments, each with its own `movi`.
    // Reading only the first would silently truncate a long recording at the
    // two-gigabyte mark, which is the kind of loss that looks like success.
    let mut streams: Vec<Stream> = Vec::new();
    let mut movi: Vec<(usize, usize)> = Vec::new();
    for riff in chunks(bytes, 0, bytes.len()) {
        if &riff.id != b"RIFF" {
            continue;
        }
        for top in chunks(bytes, riff.from, riff.to) {
            match (&top.id, top.form.as_ref()) {
                (b"LIST", Some(b"hdrl")) => streams.extend(read_hdrl(bytes, top)),
                (b"LIST", Some(b"movi")) => movi.push((top.from, top.to)),
                _ => {}
            }
        }
    }
    if streams.is_empty() {
        return Err(AviError::Malformed("this AVI declares no streams".into()));
    }

    // Which of them can be carried, and what each becomes.
    let mut tracks: Vec<Option<AvTrack>> = Vec::with_capacity(streams.len());
    let mut dropped = Vec::new();
    let mut number = 1_u64;
    for s in &streams {
        match carry(s, number) {
            Ok(t) => {
                number += 1;
                tracks.push(Some(t));
            }
            Err(why) => {
                dropped.push(why);
                tracks.push(None);
            }
        }
    }

    // The samples, in the order the file plays them.
    let mut retained: u64 = 0;
    let mut counts = vec![0_u64; streams.len()];
    let mut bytes_seen = vec![0_u64; streams.len()];
    let mut params: Vec<annexb::ParameterSets> = streams
        .iter()
        .map(|_| annexb::ParameterSets::default())
        .collect();
    for (list_from, list_to) in movi {
        for c in chunks(bytes, list_from, list_to) {
            // `##dc`, `##db`, `##wb`, `##tx`: the first two characters are the
            // stream number in ASCII decimal.
            let Some(index) = stream_of(&c.id) else {
                continue;
            };
            let Some(slot) = tracks.get_mut(index) else {
                continue;
            };
            let stream = &streams[index];
            // The clock advances for a DROPPED stream too: an audio track we
            // cannot carry still consumed time, and a later chunk of a track we
            // can must land where it belongs.
            let payload = bytes.get(c.from..c.to).unwrap_or(&[]);
            let ts = timestamp(stream, counts[index], bytes_seen[index]);
            counts[index] += 1;
            bytes_seen[index] += payload.len() as u64;

            let Some(track) = slot.as_mut() else { continue };
            if payload.is_empty() {
                continue;
            }
            retained = retained.saturating_add(payload.len() as u64);
            if retained > budget {
                return Err(AviError::TooLarge { limit: budget });
            }
            let (data, keyframe) = if track.video == Some(VideoCodec::H264) {
                // THE PARAMETER SETS ARE HARVESTED HERE, because here is where
                // the original Annex-B still exists: `to_avcc` strips the SPS
                // and PPS out of the sample, and they are the only copy of the
                // configuration an AVI has.
                annexb::to_avcc(payload, &mut params[index])
            } else {
                // Audio rides through as it is, and every audio frame is a
                // sync point.
                (payload.to_vec(), true)
            };
            if data.is_empty() {
                continue;
            }
            track.samples.push(AvSample {
                ts_ticks: ts,
                // AVI states no composition offsets, so there are none to
                // carry. See this module's header.
                composition_offset_ticks: 0,
                keyframe,
                data,
            });
        }
    }

    // The parameter sets are in band, so the `avcC` can only be built after the
    // stream has been walked.
    let mut out: Vec<AvTrack> = Vec::new();
    for (index, track) in tracks.into_iter().enumerate() {
        let Some(mut track) = track else { continue };
        if track.video == Some(VideoCodec::H264) {
            match params[index].avc_config() {
                Some(cfg) => track.codec_private = cfg,
                None => {
                    dropped.push(
                        "the H.264 video track, which carries no SPS or PPS: without them \
                         there is no configuration to write, and no player could decode \
                         the result"
                            .to_string(),
                    );
                    continue;
                }
            }
        }
        if track.samples.is_empty() {
            continue;
        }
        out.push(track);
    }
    if out.is_empty() {
        return Err(AviError::NoTracks);
    }

    Ok((
        AvGraph {
            tracks: out,
            // Milliseconds, matching what the other two demuxers produce, so a
            // graph from any source is the same shape.
            timecode_scale_ns: 1_000_000,
        },
        dropped,
    ))
}

/// The streams declared in an `hdrl` list.
fn read_hdrl(bytes: &[u8], hdrl: Chunk) -> Vec<Stream> {
    let mut out = Vec::new();
    for c in chunks(bytes, hdrl.from, hdrl.to) {
        if &c.id != b"LIST" || c.form.as_ref() != Some(b"strl") {
            continue;
        }
        let (mut strh, mut strf) = (None, None);
        for f in chunks(bytes, c.from, c.to) {
            match &f.id {
                b"strh" => strh = bytes.get(f.from..f.to),
                b"strf" => strf = bytes.get(f.from..f.to),
                _ => {}
            }
        }
        let (Some(strh), Some(strf)) = (strh, strf) else {
            continue;
        };
        let kind = match strh.get(..4) {
            Some(b"vids") => TrackKind::Video,
            Some(b"auds") => TrackKind::Audio,
            // Subtitle and timecode streams. Kept in the list so the stream
            // NUMBERING stays right — a chunk says `02wb`, and dropping stream
            // 1 from the vector would make that index the wrong stream.
            _ => TrackKind::Other,
        };
        out.push(Stream {
            kind,
            // AVISTREAMHEADER: dwScale at 20, dwRate at 24, dwSampleSize at 44,
            // all counted from the start of the `strh` payload.
            scale: le32(strh, 20).unwrap_or(1).max(1),
            rate: le32(strh, 24).unwrap_or(1).max(1),
            sample_size: le32(strh, 44).unwrap_or(0),
            format: strf.to_vec(),
        });
    }
    out
}

/// What this stream becomes, or why it cannot.
fn carry(s: &Stream, number: u64) -> Result<AvTrack, String> {
    let mut track = AvTrack {
        number,
        kind: s.kind,
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

    match s.kind {
        TrackKind::Video => {
            // BITMAPINFOHEADER: width and height at 4 and 8, the codec fourcc
            // at 16.
            let fourcc: [u8; 4] = s
                .format
                .get(16..20)
                .and_then(|f| f.try_into().ok())
                .ok_or("a video stream with no format header")?;
            // Case-insensitively, because `H264`, `h264` and `X264` are the
            // same codec written by three encoders.
            let mut upper = fourcc;
            upper.make_ascii_uppercase();
            if !matches!(&upper, b"H264" | b"X264" | b"AVC1" | b"DAVC" | b"VSSH") {
                return Err(format!(
                    "the video track coded as `{}`, which this build cannot carry into \
                     another container",
                    String::from_utf8_lossy(&fourcc)
                ));
            }
            track.video = Some(VideoCodec::H264);
            track.codec_id = b"V_MPEG4/ISO/AVC".to_vec();
            track.pixel_width = le32(&s.format, 4).unwrap_or(0);
            // Height is SIGNED, and negative means a bottom-up bitmap. That
            // says which way the rows run and nothing about the size, so the
            // magnitude is what a track's geometry wants.
            #[allow(clippy::cast_possible_wrap)]
            {
                track.pixel_height = (le32(&s.format, 8).unwrap_or(0) as i32).unsigned_abs();
            }
        }
        TrackKind::Audio => {
            // WAVEFORMATEX: format tag, channels, samples-per-second.
            let tag = le16(&s.format, 0).ok_or("an audio stream with no format header")?;
            if tag != 0x0055 {
                return Err(format!(
                    "the audio track in WAVE format 0x{tag:04x}, which this build cannot \
                     carry into another container"
                ));
            }
            track.audio = Some(AudioCodec::Mp3);
            track.codec_id = b"A_MPEG/L3".to_vec();
            track.channels = u64::from(le16(&s.format, 2).unwrap_or(0));
            track.sample_rate = f64::from(le32(&s.format, 4).unwrap_or(0));
            track.bit_depth = u64::from(le16(&s.format, 14).unwrap_or(0));
        }
        TrackKind::Other => {
            return Err("a stream that is neither audio nor video".to_string());
        }
    }
    Ok(track)
}

/// Which stream a `movi` chunk belongs to, from its two ASCII digits.
fn stream_of(id: &[u8; 4]) -> Option<usize> {
    if !matches!(&id[2..4], b"dc" | b"db" | b"wb" | b"AC") {
        return None;
    }
    let tens = (id[0] as char).to_digit(10)? as usize;
    let ones = (id[1] as char).to_digit(10)? as usize;
    Some(tens * 10 + ones)
}

/// When this sample is decoded, in milliseconds.
///
/// # `dwSampleSize` decides what is being counted
///
/// AVI carries no per-sample time. It carries a clock — `dwScale / dwRate`
/// units per second — and `dwSampleSize` says what a unit IS:
///
/// * **Zero**: one chunk is one unit. Every video stream, and every
///   variable-bitrate audio stream, including the MP3 in each file spiked.
/// * **Non-zero**: a unit is that many bytes, so a chunk holding four of them
///   advances the clock four times.
///
/// The first version of this assumed audio was always byte-counted, which
/// turned a two-second file into one claiming **420 seconds** — the audio's
/// clock was 32/1225 of a second per FRAME, and it was being multiplied by the
/// byte count instead.
///
/// This is a DECODE time. AVI stores frames in decode order and records
/// nothing about display order, so there is no composition offset to derive —
/// see the module header, and note that ffmpeg's own AVI-to-MP4 stream copy
/// writes no `ctts` either.
fn timestamp(s: &Stream, index: u64, bytes_before: u64) -> i64 {
    let units = if s.sample_size == 0 {
        index
    } else {
        bytes_before / u64::from(s.sample_size)
    };
    let ticks = units
        .saturating_mul(u64::from(s.scale))
        .saturating_mul(1000)
        / u64::from(s.rate).max(1);
    i64::try_from(ticks).unwrap_or(i64::MAX)
}

/// Annex-B to AVCC, and the configuration that goes with it.
///
/// # Two bitstream formats for one codec
///
/// H.264 is defined as a sequence of NAL units, and there are two ways to say
/// where each one ends. **Annex-B**, which AVI and MPEG-TS use, separates them
/// with `00 00 01` start codes and carries the parameter sets in band.
/// **AVCC**, which MP4 and Matroska use, prefixes each NAL with its length and
/// lifts the parameter sets into an `avcC` box in the sample entry.
///
/// The coded pictures are byte-identical between the two. Only the framing
/// differs, which is what keeps `avi -> mp4` a Class A route: no macroblock is
/// touched.
mod annexb {
    /// NAL unit types this cares about.
    const SPS: u8 = 7;
    const PPS: u8 = 8;
    const IDR: u8 = 5;
    /// Access unit delimiters carry no picture data and MP4 does not want them.
    const AUD: u8 = 9;

    /// Every NAL in an Annex-B buffer, as slices without their start codes.
    fn nals(buf: &[u8]) -> Vec<&[u8]> {
        let mut out = Vec::new();
        let mut starts = Vec::new();
        let mut i = 0;
        while i + 3 <= buf.len() {
            if buf[i] == 0 && buf[i + 1] == 0 {
                if buf.get(i + 2) == Some(&1) {
                    starts.push((i, 3));
                    i += 3;
                    continue;
                }
                if buf.get(i + 2) == Some(&0) && buf.get(i + 3) == Some(&1) {
                    starts.push((i, 4));
                    i += 4;
                    continue;
                }
            }
            i += 1;
        }
        for (n, &(at, len)) in starts.iter().enumerate() {
            let from = at + len;
            let to = starts.get(n + 1).map_or(buf.len(), |&(next, _)| next);
            if from < to {
                out.push(&buf[from..to]);
            }
        }
        out
    }

    fn nal_type(nal: &[u8]) -> u8 {
        nal.first().map_or(0, |b| b & 0x1F)
    }

    /// The parameter sets seen in one stream, in the order they first appeared.
    ///
    /// Collected rather than copied from the container: every AVI spiked
    /// carried EMPTY `strf` extradata, so the configuration exists only as
    /// these in-band NALs — which `to_avcc` removes from the samples, because
    /// MP4 wants them in the sample entry and not in the pictures.
    #[derive(Default)]
    pub struct ParameterSets {
        sps: Vec<Vec<u8>>,
        pps: Vec<Vec<u8>>,
    }

    impl ParameterSets {
        /// Remember one, if it is new.
        ///
        /// A stream may repeat its parameter sets before every keyframe — most
        /// do — and may legitimately carry more than one of each. Duplicates
        /// are dropped and genuinely different ones are kept, because an
        /// `avcC` listing the same SPS forty times is one some players refuse.
        fn note(&mut self, nal: &[u8]) {
            let list = match nal_type(nal) {
                SPS => &mut self.sps,
                PPS => &mut self.pps,
                _ => return,
            };
            if !list.iter().any(|k| k == nal) && list.len() < 32 {
                list.push(nal.to_vec());
            }
        }

        /// The `avcC` payload.
        ///
        /// `None` when the stream carried no SPS or no PPS, which is a file no
        /// player could decode either — so the caller drops the track and says
        /// why rather than writing a configuration it invented.
        pub fn avc_config(&self) -> Option<Vec<u8>> {
            let sps = self.sps.first()?;
            if self.pps.is_empty() || sps.len() < 4 {
                return None;
            }
            let mut out = Vec::with_capacity(16 + sps.len());
            out.push(1); // configurationVersion
                         // Profile, constraint flags and level come from the SPS itself:
                         // bytes 1..4, immediately after the NAL header. Writing anything
                         // else here would be a claim about the stream that the stream
                         // contradicts.
            out.extend_from_slice(&sps[1..4]);
            // Six reserved bits set, then lengthSizeMinusOne = 3, which is the
            // four-byte prefix `to_avcc` writes. These two must agree or every
            // frame is read at the wrong offset.
            out.push(0xFF);
            // Three reserved bits set, then the SPS count.
            out.push(0xE0 | u8::try_from(self.sps.len()).unwrap_or(1).min(31));
            for s in &self.sps {
                out.extend_from_slice(&u16::try_from(s.len()).unwrap_or(0).to_be_bytes());
                out.extend_from_slice(s);
            }
            // No `.min(255)`: the count is a u8 already, so the conversion
            // is the clamp.
            out.push(u8::try_from(self.pps.len()).unwrap_or(1));
            for p in &self.pps {
                out.extend_from_slice(&u16::try_from(p.len()).unwrap_or(0).to_be_bytes());
                out.extend_from_slice(p);
            }
            Some(out)
        }
    }

    /// One sample's NALs, length-prefixed, with the parameter sets and access
    /// unit delimiters removed — and the parameter sets remembered on the way
    /// past.
    ///
    /// Returns the payload and whether this sample is a sync point, which is
    /// read from the BITSTREAM (an IDR picture) rather than from the container:
    /// AVI's keyframe flags live in an `idx1` index that need not be present,
    /// and an MP4 whose every frame claims to be a sync point cannot be seeked.
    pub fn to_avcc(buf: &[u8], params: &mut ParameterSets) -> (Vec<u8>, bool) {
        let mut out = Vec::with_capacity(buf.len());
        let mut keyframe = false;
        for nal in nals(buf) {
            match nal_type(nal) {
                SPS | PPS => {
                    params.note(nal);
                    continue;
                }
                AUD => continue,
                IDR => keyframe = true,
                _ => {}
            }
            let Ok(len) = u32::try_from(nal.len()) else {
                continue;
            };
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(nal);
        }
        (out, keyframe)
    }
}
