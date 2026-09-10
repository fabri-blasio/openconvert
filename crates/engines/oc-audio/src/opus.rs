//! Opus encoding through **libopus**, muxed into Ogg pages. Linked
//! dynamically.
//!
//! Opus only encodes 48 kHz, so the caller's stream is resampled here with
//! linear interpolation when its source rate differs. The caller narrows
//! symphonia full-scale i32 to i16 BEFORE calling [`encode_opus`] — narrowing
//! is `(v >> 16) as i16`, the exact inverse of symphonia's widening.
//!
//! # The container is real, not demo-grade
//!
//! The first delivery of this module emitted Ogg pages with a zero CRC, no
//! identification header, and no end-of-stream marker — a stream strict
//! demuxers reject. What ships now follows RFC 3533 (page framing + CRC-32)
//! and RFC 7845 §4.2/§4.6 (OpusHead on a BOS page, OpusTags behind it, EOS on
//! the final page, pre-skip taken from the encoder's own lookahead).

use std::ffi::c_void;

pub type OpusEncoderPtr = *mut c_void;

const OPUS_APPLICATION_AUDIO: i32 = 2049;
const OPUS_OK: i32 = 0;
const OPUS_RATE: u32 = 48000;
/// `OPUS_GET_LOOKAHEAD` — the encoder's own delay, in samples at 48 kHz,
/// written into OpusHead as the pre-skip.
const OPUS_GET_LOOKAHEAD: i32 = 4027;
const FRAME_SIZE: usize = 960; // 20ms at 48kHz
const MAX_PACKET_SIZE: usize = 4000;

// One page's segment table has room for 255 entries of ≤255 bytes each, so a
// packet of MAX_PACKET_SIZE bytes needs at most ceil(4000/255) = 16 entries —
// every packet this encoder can produce fits in a single page.
extern "C" {
    fn opus_encoder_create(
        sample_rate: i32,
        channels: i32,
        application: i32,
        error: *mut i32,
    ) -> OpusEncoderPtr;
    fn opus_encoder_destroy(enc: OpusEncoderPtr);
    fn opus_encode(
        enc: OpusEncoderPtr,
        pcm: *const i16,
        frame_size: i32,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;
    // Variadic in C (`opus_encoder_ctl(enc, int request, ...)`). Every call
    // here passes exactly one extra `*mut i32`, which is the shape
    // `OPUS_GET_LOOKAHEAD` documents.
    fn opus_encoder_ctl(enc: OpusEncoderPtr, request: i32, ...) -> i32;
}

// ---------------------------------------------------------------------------
// The Ogg container side — RFC 3533 framing, RFC 7845 headers
// ---------------------------------------------------------------------------

/// Page flag: this page starts a logical stream. Carried by the OpusHead page.
const FLAG_BOS: u8 = 0x02;
/// Page flag: this page ends a logical stream. Carried by the final page.
const FLAG_EOS: u8 = 0x04;

/// The Ogg CRC-32 table: polynomial 0x04c11db7, MSB-first, init 0, no final
/// xor. Not the reflected zlib polynomial — a page carrying a zlib CRC fails
/// every strict demuxer, which is precisely what the zero placeholder did.
const fn ogg_crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut r = (i as u32) << 24;
        let mut bit = 0;
        while bit < 8 {
            r = if r & 0x8000_0000 != 0 {
                (r << 1) ^ 0x04C1_1DB7
            } else {
                r << 1
            };
            bit += 1;
        }
        table[i] = r;
        i += 1;
    }
    table
}

/// CRC over a whole page whose CRC field was left zeroed.
fn ogg_crc(page: &[u8]) -> u32 {
    const TABLE: [u32; 256] = ogg_crc_table();
    let mut r: u32 = 0;
    for &b in page {
        r = (r << 8) ^ TABLE[(((r >> 24) as u8) ^ b) as usize];
    }
    r
}

/// One logical Ogg stream under construction: fixed serial, running page
/// sequence, bytes so far, and the extent of the most recent page written.
struct OggMux {
    out: Vec<u8>,
    serial: u32,
    seq: u32,
    /// `(flag_byte_offset, page_end)` of the last page [`OggMux::page`]
    /// appended — where an EOS flag would be patched in later.
    last_page_range: Option<(usize, usize)>,
}

impl OggMux {
    /// Append one page carrying `packet`.
    ///
    /// Lacing follows RFC 3533: one entry per 255-byte segment plus the
    /// remainder, so packets larger than 255 bytes declare their true length.
    fn page(&mut self, granule: u64, flags: u8, packet: &[u8]) {
        let segments = packet.len() / 255;
        let remainder = packet.len() % 255;
        let lacing_entries = segments + usize::from(remainder > 0);

        let mut header = Vec::with_capacity(27 + lacing_entries);
        header.extend_from_slice(b"OggS");
        header.push(0); // structure version
        header.push(flags);
        header.extend_from_slice(&granule.to_le_bytes());
        header.extend_from_slice(&self.serial.to_le_bytes());
        header.extend_from_slice(&self.seq.to_le_bytes());
        header.extend_from_slice(&[0; 4]); // CRC placeholder, patched below
        debug_assert!(lacing_entries <= 255, "one packet per page, always");
        header.push(lacing_entries as u8);
        let mut lacing = vec![255u8; segments];
        if remainder > 0 {
            lacing.push(remainder as u8);
        }
        header.extend_from_slice(&lacing);

        let start = self.out.len();
        self.out.extend_from_slice(&header);
        self.out.extend_from_slice(packet);

        // The CRC covers the WHOLE page with its own field zeroed — which it
        // still is right now — so hashing the bytes just appended is correct.
        let crc = ogg_crc(&self.out[start..]);
        self.out[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());

        self.last_page_range = Some((start, self.out.len()));
        self.seq += 1;
    }
}

/// The OpusHead identification header (RFC 7845 §4.2).
fn opus_head(channels: usize, source_rate: u32, pre_skip: u16) -> Vec<u8> {
    let mut p = Vec::with_capacity(19);
    p.extend_from_slice(b"OpusHead");
    p.push(1); // version
    p.push(channels as u8);
    p.extend_from_slice(&pre_skip.to_le_bytes());
    // The ORIGINAL rate, as the field is defined; decoders handle the fact
    // that the coded stream runs at 48 kHz themselves.
    p.extend_from_slice(&source_rate.to_le_bytes());
    p.extend_from_slice(&0i16.to_le_bytes()); // output gain
    p.push(0); // channel mapping family 0
    p
}

/// The OpusTags comment header (RFC 7845 §4.6): vendor string, no comments.
fn opus_tags() -> Vec<u8> {
    const VENDOR: &str = "openconvert oc-audio";
    let mut p = Vec::with_capacity(8 + 4 + VENDOR.len() + 4);
    p.extend_from_slice(b"OpusTags");
    p.extend_from_slice(&(VENDOR.len() as u32).to_le_bytes());
    p.extend_from_slice(VENDOR.as_bytes());
    p.extend_from_slice(&0u32.to_le_bytes()); // user comment list length
    p
}

/// Encode full-scale i16 interleaved PCM to Opus-in-Ogg at 48 kHz.
pub fn encode_opus(
    samples_i16: &[i16],
    channels: usize,
    source_rate: u32,
) -> Result<Vec<u8>, String> {
    if channels != 1 && channels != 2 {
        return Err(format!("Opus supports 1 or 2 channels, got {channels}"));
    }
    // A zero source rate would divide by zero below and try to allocate an
    // infinite resample buffer; no real stream reports one, so refuse early.
    if source_rate == 0 {
        return Err("the stream never reported a usable sample rate".into());
    }

    // A fixed serial keeps the output byte-identical run to run, which the
    // determinism gate requires; uniqueness matters only for interleaved
    // physical streams, which nothing here produces.
    const SERIAL: u32 = 0x5452_4E53; // "TRNS"
    let mut mux = OggMux {
        out: Vec::new(),
        serial: SERIAL,
        seq: 0,
        last_page_range: None,
    };

    if samples_i16.is_empty() {
        // No samples to encode is still a well-formed STREAM: identification,
        // tags, and an end marker. Strict demuxers accept this; they reject a
        // bare header with no EOS, which is what the old code produced.
        mux.page(0, FLAG_BOS, &opus_head(channels, source_rate, 0));
        mux.page(0, FLAG_EOS, &opus_tags());
        return Ok(mux.out);
    }

    // Resample to 48 kHz by linear interpolation: each output position lands
    // fractionally between two neighbouring source samples, mixed by weight.
    // Positions never reach past the final sample (the last output clamps its
    // `next` neighbour onto the last real sample), so nothing beyond the
    // stream's end is invented. Interleaving is preserved because positions
    // advance uniformly across the whole interleaved buffer.
    let ratio = OPUS_RATE as f64 / source_rate as f64;
    let new_len = (samples_i16.len() as f64 * ratio) as usize;
    let last = samples_i16.len() - 1;
    let resampled: Vec<i16> = (0..new_len)
        .map(|i| {
            let pos = i as f64 / ratio;
            let idx = (pos as usize).min(last);
            let frac = (pos - idx as f64) as f32;
            let here = f32::from(samples_i16[idx]);
            let next = samples_i16.get(idx + 1).map_or(here, |&v| f32::from(v));
            (here * (1.0 - frac) + next * frac) as i16
        })
        .collect();

    let mut error: i32 = 0;
    // SAFETY: allocates encoder; freed by Guard.
    let enc = unsafe {
        opus_encoder_create(
            OPUS_RATE as i32,
            channels as i32,
            OPUS_APPLICATION_AUDIO,
            &mut error,
        )
    };
    if error != OPUS_OK || enc.is_null() {
        return Err(format!("opus_encoder_create failed: {error}"));
    }
    struct Guard(OpusEncoderPtr);
    impl Drop for Guard {
        fn drop(&mut self) {
            unsafe {
                opus_encoder_destroy(self.0);
            }
        }
    }
    let _guard = Guard(enc);

    // The encoder's own delay, written into OpusHead as the pre-skip so a
    // decoder skips the priming samples instead of playing them. A failed
    // query leaves 0 — legal, and at worst plays a click's worth of silence.
    // SAFETY: live encoder; variadic ctl takes exactly one `*mut i32` here.
    let mut lookahead: i32 = 0;
    // SAFETY: live encoder held by `_guard`; documented ctl shape above.
    unsafe { opus_encoder_ctl(enc, OPUS_GET_LOOKAHEAD, &mut lookahead) };
    let pre_skip = u16::try_from(lookahead.clamp(0, i32::from(u16::MAX))).unwrap_or(0);

    // FLUSH THE ENCODER'S DELAY, or the pre-skip eats the end of the file.
    //
    // Declaring a pre-skip obliges the decoder to DISCARD that many samples,
    // and libopus holds exactly that many of ours inside itself when the
    // input runs out. Feeding N samples and declaring a pre-skip of P
    // therefore yields N - P samples back: this encoder wrote a one-second
    // tone and a correct decoder returned 0.9935 seconds of it, with the last
    // 312 samples still stuck in the encoder. RFC 7845 §4 also defines the
    // granule position as counting the pre-skip, so the duration claim was
    // over by P on top of the audio being short by P.
    //
    // Appending P frames of silence pushes the real tail out and makes the
    // final granule N + P, which is what the RFC asks for and what makes a
    // wav -> opus -> wav round trip come back the length it went in.
    //
    // This was invisible until this module learned to DECODE. Nothing could
    // read Ogg Opus, so nothing could measure what came back.
    let mut resampled = resampled;
    resampled.resize(resampled.len() + pre_skip as usize * channels, 0);
    let resampled = resampled;

    // Headers first: OpusHead on its BOS page, OpusTags behind it. Both carry
    // granule 0 — they contain no samples.
    mux.page(0, FLAG_BOS, &opus_head(channels, source_rate, pre_skip));
    mux.page(0, 0, &opus_tags());

    let frame_stride = FRAME_SIZE * channels;
    let mut granule: u64 = 0;

    for start in (0..resampled.len()).step_by(frame_stride) {
        let end = (start + frame_stride).min(resampled.len());
        let real_frames = (end - start) / channels;

        // An Ogg page's granule states where the data INSIDE that page ends,
        // so this window's frames are credited BEFORE the header is written —
        // crediting after left every page reporting the previous page's end.
        granule += u64::try_from(real_frames).unwrap_or(u64::MAX);

        // libopus reads exactly FRAME_SIZE*channels values from the pointer,
        // so a short FINAL window is zero-padded to a full frame instead of
        // letting the encoder walk off the end of the input. Silence padding
        // is standard encoder behaviour; only the REAL frames advance the
        // granule below, so duration stays honest. A stray trailing sample
        // that cannot complete a channel group (fewer than `channels` left)
        // is dropped — sub-frame truncation of at most one interleaved value.
        let mut padded: Vec<i16>;
        let pcm: &[i16] = if end - start == frame_stride {
            &resampled[start..end]
        } else {
            padded = vec![0i16; frame_stride];
            padded[..end - start].copy_from_slice(&resampled[start..end]);
            &padded
        };

        let mut packet = vec![0u8; MAX_PACKET_SIZE];
        // SAFETY: pcm holds FRAME_SIZE*channels valid i16 values (real data
        // plus, on the final frame, zero padding); packet is MAX_PACKET_SIZE.
        let written = unsafe {
            opus_encode(
                enc,
                pcm.as_ptr(),
                FRAME_SIZE as i32,
                packet.as_mut_ptr(),
                MAX_PACKET_SIZE as i32,
            )
        };
        if written < 0 {
            return Err(format!("opus_encode failed: {written}"));
        }
        if written == 0 {
            continue;
        }

        mux.page(granule, 0, &packet[..written as usize]);
    }

    // End-of-stream lives on the FINAL page. Setting the flag invalidates
    // that page's CRC — which must be hashed over the whole page with its own
    // field ZEROED, and the old value is sitting there right now, so the
    // field is cleared before the recompute.
    if let Some((page_start, page_end)) = mux.last_page_range {
        mux.out[page_start + 5] |= FLAG_EOS;
        mux.out[page_start + 22..page_start + 26].fill(0);
        let crc = ogg_crc(&mux.out[page_start..page_end]);
        mux.out[page_start + 22..page_start + 26].copy_from_slice(&crc.to_le_bytes());
    }

    Ok(mux.out)
}

// ---------------------------------------------------------------------------
// Decode: Ogg Opus back to PCM
// ---------------------------------------------------------------------------

/// A decoder handle. Same opaque-pointer discipline as the encoder above.
type OpusDecoderPtr = *mut core::ffi::c_void;

extern "C" {
    fn opus_decoder_create(sample_rate: i32, channels: i32, error: *mut i32) -> OpusDecoderPtr;
    fn opus_decoder_destroy(dec: OpusDecoderPtr);
    fn opus_decode(
        dec: OpusDecoderPtr,
        data: *const u8,
        len: i32,
        pcm: *mut i16,
        frame_size: i32,
        decode_fec: i32,
    ) -> i32;
}

/// Frees the decoder on every path out, including a `?`.
struct DecoderGuard(OpusDecoderPtr);

impl Drop for DecoderGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: created by opus_decoder_create, destroyed exactly once.
            unsafe { opus_decoder_destroy(self.0) };
        }
    }
}

/// The largest frame Opus defines: 120 ms at 48 kHz, per channel.
const MAX_FRAME_SAMPLES: usize = 5760;

/// Whether these bytes are an Ogg stream carrying Opus.
///
/// Cheap and structural: the Ogg capture pattern, then `OpusHead` at the start
/// of the first page's payload, which RFC 7845 §3 requires. Anything else --
/// Ogg Vorbis in particular -- answers `false` and goes to symphonia, which
/// handles it.
#[must_use]
pub fn is_ogg_opus(bytes: &[u8]) -> bool {
    let Some(pages) = OggPages::new(bytes) else {
        return false;
    };
    pages
        .take(1)
        .next()
        .is_some_and(|p| p.payload.starts_with(b"OpusHead"))
}

/// One Ogg page, already bounds-checked.
struct OggPage<'a> {
    payload: &'a [u8],
    segments: &'a [u8],
    serial: u32,
    /// Where the samples in this page END, counted from the start of the
    /// stream and INCLUDING the pre-skip (RFC 7845 §4). The last page's value
    /// is the stream's true length, and the only thing that distinguishes real
    /// audio from the encoder's final-frame zero padding.
    granule: u64,
    end: usize,
}

/// A bounded walk over an Ogg bitstream.
///
/// Every field is read only after the buffer is confirmed to hold it, and a
/// malformed page ends the iteration rather than being skipped past -- a
/// converter that resynchronises is a converter that silently drops audio.
struct OggPages<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> OggPages<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
        bytes.starts_with(b"OggS").then_some(Self { bytes, at: 0 })
    }
}

impl<'a> Iterator for OggPages<'a> {
    type Item = OggPage<'a>;

    fn next(&mut self) -> Option<OggPage<'a>> {
        const HEADER: usize = 27;
        let b = self.bytes;
        let at = self.at;
        if at.checked_add(HEADER)? > b.len() || &b[at..at + 4] != b"OggS" {
            return None;
        }
        let mut g = [0_u8; 8];
        g.copy_from_slice(&b[at + 6..at + 14]);
        let granule = u64::from_le_bytes(g);
        let serial = u32::from_le_bytes([b[at + 14], b[at + 15], b[at + 16], b[at + 17]]);
        let n_segments = b[at + 26] as usize;
        let seg_table = at.checked_add(HEADER)?;
        let payload_at = seg_table.checked_add(n_segments)?;
        if payload_at > b.len() {
            return None;
        }
        let segments = &b[seg_table..payload_at];
        let payload_len: usize = segments.iter().map(|&s| s as usize).sum();
        let end = payload_at.checked_add(payload_len)?;
        if end > b.len() {
            return None;
        }
        self.at = end;
        Some(OggPage {
            payload: &b[payload_at..end],
            segments,
            serial,
            granule,
            end,
        })
    }
}

/// Decode an Ogg Opus stream to interleaved 16-bit PCM at 48 kHz.
///
/// # Why this exists
///
/// This module has written Ogg Opus since the encoder landed, and **nothing
/// could read it back** -- not even this program. symphonia 0.5 ships no Opus
/// decoder at all, so `ogg -> wav`, `ogg -> mp3` and `ogg -> ogg` all refused
/// our own output with "unsupported codec". Three route rows the table
/// advertised, against files this project itself produces.
///
/// libopus was already linked for the encoder, and the decoder lives in the
/// same DLL, so the missing half was the container: reassembling packets from
/// Ogg pages. That is what the bulk of this function is.
///
/// # Output rate
///
/// Always 48 kHz. Opus is defined at 48 kHz internally and `OpusHead` records
/// the ORIGINAL input rate only as metadata -- RFC 7845 §5.1 is explicit that
/// it "does not affect the decoding process". Decoding to anything else would
/// mean resampling, which is a fidelity decision this does not take.
///
/// # Errors
///
/// A malformed container, a header that is not `OpusHead`, a channel count
/// libopus will not accept, or a decode that would exceed `decoded_budget`.
#[cfg(has_opus)]
pub fn decode_ogg_opus(bytes: &[u8], decoded_budget: u64) -> Result<(Vec<i16>, usize), String> {
    let pages = OggPages::new(bytes).ok_or("this is not an Ogg stream")?;

    // Reassemble packets. A segment of exactly 255 continues the packet; any
    // shorter value ends it, INCLUDING zero -- that is how a zero-length
    // packet is spelled, and treating it as a terminator rather than a gap is
    // what keeps packet boundaries aligned with the encoder's.
    let mut packets: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut serial: Option<u32> = None;
    let mut last_end = 0usize;
    let mut final_granule: u64 = 0;
    for page in pages {
        // One logical stream only. A chained or multiplexed Ogg file is a
        // different shape than this promises to handle, and quietly decoding
        // the first stream of several would lose audio without saying so.
        match serial {
            None => serial = Some(page.serial),
            Some(s) if s != page.serial => {
                return Err(
                    "this Ogg file carries more than one logical stream, which this build \
                     does not join"
                        .into(),
                )
            }
            Some(_) => {}
        }
        last_end = page.end;
        final_granule = page.granule;
        let mut offset = 0usize;
        for &seg in page.segments {
            let len = seg as usize;
            current.extend_from_slice(&page.payload[offset..offset + len]);
            offset += len;
            if len < 255 {
                packets.push(core::mem::take(&mut current));
            }
        }
    }
    // Trailing bytes after the last well-formed page mean the file is
    // truncated or has something appended. Either way it is not what it
    // claims, and finishing quietly would put a clean receipt over it.
    if last_end != bytes.len() {
        return Err(format!(
            "the Ogg stream ends at byte {last_end} but the file is {} bytes; it is truncated \
             or has trailing data",
            bytes.len()
        ));
    }

    // RFC 7845 §3: the identification header is the first packet, the comment
    // header is the second, and audio follows.
    let head = packets.first().ok_or("this Ogg file contains no packets")?;
    if !head.starts_with(b"OpusHead") || head.len() < 19 {
        return Err("this Ogg stream is not Opus (no OpusHead identification header)".into());
    }
    let channels = head[9] as usize;
    if channels == 0 || channels > 2 {
        return Err(format!(
            "this Opus stream has {channels} channels; this build decodes mono and stereo"
        ));
    }
    let pre_skip = u16::from_le_bytes([head[10], head[11]]) as usize;
    if head[18] != 0 {
        return Err(
            "this Opus stream uses a channel mapping family this build does not decode".into(),
        );
    }

    let mut error: i32 = 0;
    // SAFETY: plain constructor; 48 kHz and the channel count are both
    // validated above, and `error` is written before the pointer is used.
    let decoder = DecoderGuard(unsafe { opus_decoder_create(48_000, channels as i32, &mut error) });
    if decoder.0.is_null() || error != 0 {
        return Err(format!(
            "libopus would not create a decoder (error {error})"
        ));
    }

    let mut out: Vec<i16> = Vec::new();
    let mut frame = vec![0_i16; MAX_FRAME_SAMPLES * channels];
    // Skip the two headers; everything after them is audio.
    for packet in packets.iter().skip(2) {
        let len = i32::try_from(packet.len())
            .map_err(|_| "an Opus packet is larger than libopus accepts".to_string())?;
        // SAFETY: `packet` and `frame` are live for the call; `frame` holds
        // MAX_FRAME_SAMPLES per channel, which is the largest frame Opus
        // defines, so libopus cannot write past it.
        let n = unsafe {
            opus_decode(
                decoder.0,
                packet.as_ptr(),
                len,
                frame.as_mut_ptr(),
                MAX_FRAME_SAMPLES as i32,
                0,
            )
        };
        if n < 0 {
            return Err(format!("libopus refused a packet (error {n})"));
        }
        let produced = n as usize * channels;
        // The same pre-allocation rule the rest of this worker follows: refuse
        // before the allocation, not after it.
        let would_be = (out.len() + produced) as u64 * 2;
        if would_be > decoded_budget {
            return Err(format!(
                "decoding this file would need more than the {decoded_budget}-byte limit"
            ));
        }
        out.extend_from_slice(&frame[..produced]);
    }

    // TRIM TO THE GRANULE, THEN SKIP THE PRE-SKIP. Both, in that order.
    //
    // Opus codes fixed-size frames, so the final frame is zero-padded by the
    // encoder and libopus hands those padding samples back like any others.
    // The granule position is the ONLY record of where the real audio stopped
    // -- that is what the field is for -- so a decoder that returns everything
    // it was given returns silence the file does not claim to contain. A
    // one-second tone came back as 1.0135 seconds before this trim.
    //
    // The granule counts the pre-skip (RFC 7845 §4), so it bounds the buffer
    // BEFORE the skip is removed, not after.
    let total = usize::try_from(final_granule)
        .unwrap_or(usize::MAX)
        .saturating_mul(channels);
    out.truncate(total);

    // RFC 7845 §4.2: the encoder's own latency, which the decoder is required
    // to discard.
    let skip = pre_skip.saturating_mul(channels).min(out.len());
    out.drain(..skip);
    Ok((out, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_starts_with_oggs_magic() {
        let samples: Vec<i16> = vec![0i16; 48000 * 2]; // 1 second stereo silence
        let ogg = encode_opus(&samples, 2, OPUS_RATE).unwrap();
        assert!(ogg.starts_with(b"OggS"), "missing OggS magic");
        assert!(!ogg.is_empty());
    }

    #[test]
    fn empty_input_produces_a_complete_stream() {
        let ogg = encode_opus(&[], 2, OPUS_RATE).unwrap();
        assert!(ogg.starts_with(b"OggS"), "missing OggS magic");
        let pages = walk_pages(&ogg);
        assert_eq!(pages.len(), 2, "head page plus tags page, nothing else");
        assert_eq!(pages[0].0 & FLAG_BOS, FLAG_BOS, "first page must be BOS");
        assert_eq!(
            pages[pages.len() - 1].0 & FLAG_EOS,
            FLAG_EOS,
            "the final page must carry EOS"
        );
    }

    #[test]
    fn wrong_sample_rate_gets_resampled_not_rejected() {
        let samples: Vec<i16> = vec![100i16; 44100]; // 1 second mono at 44.1 kHz
        let ogg = encode_opus(&samples, 1, 44100).unwrap();
        assert!(ogg.starts_with(b"OggS"), "missing OggS magic");
        assert!(!ogg.is_empty());
    }

    #[test]
    fn three_channels_refused() {
        assert!(encode_opus(&[], 3, OPUS_RATE).is_err());
        assert!(encode_opus(&[0i16; 960], 0, OPUS_RATE).is_err());
    }

    /// Walk every page in an Ogg physical stream, checking structure and CRC.
    ///
    /// Returns each page's `(flags, granule)`. This is the independent parser
    /// the old zero-CRC output could never have survived.
    fn walk_pages(ogg: &[u8]) -> Vec<(u8, u64)> {
        let mut pages = Vec::new();
        let mut i = 0;
        while i < ogg.len() {
            assert_eq!(&ogg[i..i + 4], b"OggS", "page magic at {i}");
            assert_eq!(ogg[i + 4], 0, "unknown structure version at {i}");
            let flags = ogg[i + 5];
            let mut granule_bytes = [0u8; 8];
            granule_bytes.copy_from_slice(&ogg[i + 6..i + 14]);
            let granule = u64::from_le_bytes(granule_bytes);
            let n_segments = ogg[i + 26] as usize;
            let payload: usize = ogg[i + 27..i + 27 + n_segments]
                .iter()
                .map(|&s| s as usize)
                .sum();
            let page_len = 27 + n_segments + payload;

            // THE check the placeholder could not pass: recompute the CRC over
            // the page with its field zeroed and require equality.
            let mut copy = ogg[i..i + page_len].to_vec();
            copy[22..26].fill(0);
            let stored = u32::from_le_bytes([ogg[i + 22], ogg[i + 23], ogg[i + 24], ogg[i + 25]]);
            assert_eq!(ogg_crc(&copy), stored, "CRC mismatch on page at {i}");

            pages.push((flags, granule));
            i += page_len;
        }
        pages
    }

    #[test]
    fn every_page_carries_a_correct_crc() {
        let samples: Vec<i16> = vec![0x1234i16; 48000]; // ~1 second, multi-page
        let ogg = encode_opus(&samples, 2, OPUS_RATE).unwrap();
        let pages = walk_pages(&ogg);
        assert!(pages.len() > 3, "expected headers plus several audio pages");
        let (first_flags, _) = pages[0];
        assert_eq!(first_flags & FLAG_BOS, FLAG_BOS, "first page must be BOS");
        let (last_flags, _) = pages[pages.len() - 1];
        assert_eq!(last_flags & FLAG_EOS, FLAG_EOS, "last page must be EOS");
    }

    #[test]
    fn identification_header_is_opushead_on_page_zero() {
        let samples: Vec<i16> = vec![0i16; 960];
        let ogg = encode_opus(&samples, 1, 44100).unwrap();
        // First packet area begins right after the single-segment header.
        assert_eq!(&ogg[28..36], b"OpusHead", "page 0 must carry OpusHead");
        assert_eq!(ogg[36], 1, "OpusHead version");
        assert_eq!(ogg[37], 1, "channel count");
        // Original sample rate recorded, not the coded 48 kHz.
        let rate = u32::from_le_bytes([ogg[40], ogg[41], ogg[42], ogg[43]]);
        assert_eq!(rate, 44100, "input sample rate must be recorded verbatim");
        // Tags follow on page 1.
        let tags_at = ogg
            .windows(8)
            .position(|w| w == b"OpusTags")
            .expect("OpusTags present");
        assert!(tags_at > 36, "tags must come after the head page");
    }

    /// The old defect, pinned shut: 1000 mono frames is NOT a multiple of the
    /// 960-frame size, so the final window used to be handed to libopus short
    /// and read past the end of the buffer. It must encode cleanly via zero
    /// padding.
    ///
    /// The granule assertion here USED to read `granule == 1000` — "count the
    /// real samples, not the padding" — and that was wrong twice over. RFC
    /// 7845 §4 defines the granule position as counting the pre-skip too, and
    /// the encoder now appends `pre_skip` frames of silence so its own delay
    /// flushes instead of eating the tail. Both changes point the same way:
    /// the final granule is real + pre-skip, and a decoder that subtracts the
    /// pre-skip lands back on the real count.
    ///
    /// The old value looked more honest and produced a file that came back
    /// SHORT. Nothing caught it because nothing could decode Ogg Opus.
    #[test]
    fn the_final_granule_counts_real_samples_plus_the_pre_skip() {
        const REAL: u64 = 1000; // 960 + 40, mono at 48 kHz
        let samples: Vec<i16> = vec![0i16; REAL as usize];
        let ogg = encode_opus(&samples, 1, OPUS_RATE).unwrap();

        let pages = walk_pages(&ogg);
        let (last_flags, granule) = pages[pages.len() - 1];

        // The pre-skip the encoder declared, read back out of OpusHead.
        let head_at = ogg
            .windows(8)
            .position(|w| w == b"OpusHead")
            .expect("OpusHead present");
        let pre_skip = u64::from(u16::from_le_bytes([ogg[head_at + 10], ogg[head_at + 11]]));

        assert_eq!(
            granule,
            REAL + pre_skip,
            "the final granule counts the real samples AND the pre-skip"
        );
        // And the final page — an audio page here — still carries EOS.
        assert_eq!(last_flags & FLAG_EOS, FLAG_EOS);
    }

    /// The property the granule and pre-skip exist to make true.
    ///
    /// Encode N frames, decode them back, get N frames. Not "about N": the
    /// granule bounds the padded final frame and the pre-skip removes the
    /// encoder's delay, so the count is exact. Every off-by-one in either
    /// field shows up here as a length that is not N — which is how the two
    /// defects above were found in the first place.
    #[test]
    fn a_round_trip_returns_exactly_as_many_frames_as_it_was_given() {
        for &frames in &[1000_usize, 960, 4800, 961] {
            for channels in [1_usize, 2] {
                let samples: Vec<i16> = (0..frames * channels)
                    .map(|i| ((i / channels) as i16).wrapping_mul(37))
                    .collect();
                let ogg = encode_opus(&samples, channels, OPUS_RATE).unwrap();
                let (back, got_channels) = decode_ogg_opus(&ogg, u64::MAX).unwrap();
                assert_eq!(got_channels, channels);
                assert_eq!(
                    back.len(),
                    frames * channels,
                    "{frames} frames x {channels}ch must round-trip to the same count"
                );
            }
        }
    }

    /// The decoder refuses what it cannot vouch for, rather than guessing.
    #[test]
    fn the_decoder_refuses_malformed_and_truncated_streams() {
        let ogg = encode_opus(&vec![0i16; 2000], 1, OPUS_RATE).unwrap();

        assert!(
            decode_ogg_opus(&ogg[..ogg.len() - 20], u64::MAX).is_err(),
            "a truncated stream must refuse, not return short audio"
        );
        assert!(
            decode_ogg_opus(b"not an ogg file at all", u64::MAX).is_err(),
            "non-Ogg input must refuse"
        );
        assert!(
            decode_ogg_opus(&ogg, 64).is_err(),
            "the decoded-byte budget must refuse before the allocation"
        );

        // A Vorbis-flavoured Ogg is not ours to decode, and `is_ogg_opus`
        // must say so rather than letting the decoder fail later.
        let mut vorbis = ogg.clone();
        let head = vorbis
            .windows(8)
            .position(|w| w == b"OpusHead")
            .expect("OpusHead present");
        vorbis[head..head + 8].copy_from_slice(b"XXXXXXXX");
        assert!(!is_ogg_opus(&vorbis));

        // Every truncation point must refuse or return, never panic.
        for cut in 0..ogg.len().min(600) {
            let _ = decode_ogg_opus(&ogg[..cut], u64::MAX);
        }
    }

    #[test]
    fn crc_matches_an_independent_bitwise_implementation() {
        // Same polynomial and direction, computed bit-by-bit without the
        // table. A reflected (zlib-style) or wrong-init implementation fails
        // this on almost any input; the two agreeing over varied bytes pins
        // polynomial, direction, init AND final-xor at once.
        fn bitwise(data: &[u8]) -> u32 {
            let mut r: u32 = 0;
            for &b in data {
                r ^= u32::from(b) << 24;
                for _ in 0..8 {
                    r = if r & 0x8000_0000 != 0 {
                        (r << 1) ^ 0x04C1_1DB7
                    } else {
                        r << 1
                    };
                }
            }
            r
        }
        let samples = [
            &[0u8][..],
            b"OggS",
            b"OpusHead",
            b"The quick brown fox jumps over the lazy dog",
            &[0xFFu8; 255],
        ];
        for s in samples {
            assert_eq!(ogg_crc(s), bitwise(s), "crc diverged on {s:?}");
        }
        assert_eq!(ogg_crc(&[]), 0, "init 0, no final xor");
    }
}
