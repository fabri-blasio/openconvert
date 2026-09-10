//! `oc-audio` â€” the audio conversion worker.
//!
//! Decode is [`symphonia`] (pure Rust); encode is [`hound`] for WAV and
//! [`flacenc`] for FLAC. All three are pure Rust with no `links` key, which is
//! why `engines.toml` lists no native audio engine yet: LAME and libopus arrive
//! when MP3/Opus *output* does, each with its linkage row â€” until then an
//! encoder we cannot ship is a route the table must not advertise.
//!
//! # Why this worker exists when every dependency is safe Rust
//!
//! The same question was asked of `oc-archive`, and the answer is the same:
//! `memory_safe` answers "could this run in-process"; the FORMAT table's parser
//! flag answers "should it". Attacker-supplied media parses uniformly behind
//! the boundary, so no decoder ever shares an address space with the GUI,
//! whatever a future version links.
//!
//! # The fidelity rule this file enforces
//!
//! **Integer sources stay integer at their own width.** Symphonia widens every
//! integer sample to full-scale i32 (`i16::into_sample::<i32>` is a left
//! shift); both encoders here want native-width values, so each writer shifts
//! back down by exactly `32 âˆ’ bits`. An arithmetic shift is the exact inverse
//! of the widening shift, which is what makes `FLAC â‡„ WAV` genuinely Class A â€”
//! the samples come out identical, not approximately the same.
//!
//! Float sources take the float path untouched, and refuse FLAC outright
//! rather than making a re-quantisation decision nobody asked for.

use openconvert_worker::{run_real, Converted, Engine, Probed, RunLimits};
use std::io::Cursor;

// The native encoders link only when their vcpkg import libraries were found
// at build time (`build.rs` sets these cfgs; missing libraries warn and leave
// the formats refusing by name rather than breaking the build).
#[cfg(has_lame)]
mod lame;
#[cfg(has_opus)]
mod opus;

fn main() -> std::process::ExitCode {
    // `run_real`, never `serve`: this binary confines itself before reading
    // anything (Linux), and reports what the host's spawn engaged (Windows).
    match run_real(&Audio) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // stderr, never stdout: stdout is the protocol.
            eprintln!("oc-audio: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

struct Audio;

impl Engine for Audio {
    fn name(&self) -> &'static str {
        "oc-audio"
    }

    fn version(&self) -> String {
        // Linked C encoders add their versions here when they land -- those
        // are the numbers CVE advisories are written against.
        format!(
            "oc-audio {} (symphonia Â· hound Â· flacenc)",
            env!("CARGO_PKG_VERSION")
        )
    }

    /// Container header only: sample rate and channel count, nothing decoded.
    ///
    /// Duration stays 0 ("not determined"). `n_frames` means different things
    /// per codec â€” total samples in FLAC, packet counts in MP3 â€” and reporting
    /// a number we know to be ambiguous is worse than reporting none, which is
    /// the convention every probe in this project uses.
    fn probe(&self, bytes: &[u8]) -> Result<Probed, String> {
        let reader = open_reader(bytes)?;
        let params = reader
            .tracks()
            .iter()
            .map(|t| &t.codec_params)
            .find(|c| c.sample_rate.is_some() || c.channels.is_some())
            .ok_or("no audio track found in the header")?;
        Ok(Probed::Audio {
            sample_rate: params.sample_rate.unwrap_or(0),
            channels: params.channels.map_or(0, |c| c.count() as u8),
            duration_ms: 0,
        })
    }

    fn run_params(
        &self,
        bytes: &[u8],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        // The host sends `lossy_route` when the row being executed is already
        // Class B. It is the only parameter this engine takes, and an
        // unrecognised one is an error rather than silence — see the trait.
        let mut lossy = false;
        let mut buckets: Option<usize> = None;
        for (k, v) in params {
            match k.as_str() {
                "lossy_route" => lossy = v == "1",
                "op" if v == "peaks" => {}
                "buckets" => {
                    buckets = Some(
                        v.parse::<usize>()
                            .map_err(|_| format!("buckets must be a number, not {v:?}"))?,
                    );
                }
                other => return Err(format!("oc-audio does not take the {other:?} parameter")),
            }
        }
        if params.iter().any(|(k, v)| k == "op" && v == "peaks") {
            return self.peaks(bytes, limits, buckets.unwrap_or(DEFAULT_BUCKETS));
        }
        self.run_inner(bytes, to, limits, lossy)
    }

    fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
        self.run_inner(bytes, to, limits, false)
    }
}

/// Bars in a waveform when the caller does not say.
const DEFAULT_BUCKETS: usize = 96;

/// The most bars anyone can ask for. A waveform is a picture a person looks
/// at; ten thousand bars is not more information, it is a denial of service
/// with extra steps.
const MAX_BUCKETS: usize = 2048;

impl Audio {
    /// Peak amplitude per bucket, for drawing a waveform.
    ///
    /// **This exists because the waveform was a lie.** The tool workspace drew
    /// a shape from a seeded random number generator and labelled it "Waveform
    /// of <the user's file>". It was deterministic, so it looked stable, and
    /// it had nothing whatever to do with the audio.
    ///
    /// The output is one byte per bucket, 0-255, being the loudest absolute
    /// sample in that slice of the file. Bytes rather than JSON because this
    /// crosses the worker protocol as a blob, and 96 bytes is the whole
    /// answer; a peak is a magnitude and eight bits is more precision than a
    /// 2px-wide bar can show.
    ///
    /// Peak, not RMS. RMS is the better measure of loudness and the worse
    /// measure for this: it flattens transients, and the difference a denoise
    /// makes shows up between the peaks, which is exactly where RMS averages
    /// it away.
    fn peaks(&self, bytes: &[u8], limits: &RunLimits, buckets: usize) -> Result<Converted, String> {
        if bytes.len() as u64 > limits.memory_bytes {
            return Err(format!(
                "input exceeds the memory limit of {} bytes",
                limits.memory_bytes
            ));
        }
        let buckets = buckets.clamp(1, MAX_BUCKETS);
        let (samples, _) = Samples::decode(bytes, limits.memory_bytes)?;

        // Per-CHANNEL frames, not interleaved samples: a stereo file has half
        // as many moments in time as it has samples, and bucketing the
        // interleaved run would make the left and right channels alternate
        // down the timeline rather than share it.
        let (frames, channels) = match &samples {
            Samples::Int {
                interleaved,
                channels,
                ..
            } => (interleaved.len() / (*channels).max(1), *channels),
            Samples::Float {
                interleaved,
                channels,
                ..
            } => (interleaved.len() / (*channels).max(1), *channels),
        };
        if frames == 0 {
            return Err("that recording decoded to no audio".to_string());
        }

        let mut out = vec![0_u8; buckets];
        for (b, slot) in out.iter_mut().enumerate() {
            let start = b * frames / buckets;
            let end = ((b + 1) * frames / buckets).max(start + 1).min(frames);
            let mut peak = 0.0_f32;
            for frame in start..end {
                for c in 0..channels {
                    let v = match &samples {
                        // Full-scale i32, as `Samples` documents, so the
                        // divisor is i32::MAX whatever the source bit depth
                        // was -- symphonia already widened it.
                        Samples::Int { interleaved, .. } => {
                            let raw = interleaved[frame * channels + c];
                            #[allow(clippy::cast_precision_loss)]
                            {
                                (raw as f32 / i32::MAX as f32).abs()
                            }
                        }
                        Samples::Float { interleaved, .. } => {
                            interleaved[frame * channels + c].abs()
                        }
                    };
                    if v > peak {
                        peak = v;
                    }
                }
            }
            // Float sources can legitimately exceed 1.0 (headroom above full
            // scale is not an error in float PCM); clamping keeps the byte in
            // range without pretending the sample was quieter than it was.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            {
                *slot = (peak.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }

        Ok(Converted {
            bytes: out,
            removed: Vec::new(),
        })
    }

    /// The one conversion path, with the route's own class alongside it.
    fn run_inner(
        &self,
        bytes: &[u8],
        to: &str,
        limits: &RunLimits,
        lossy: bool,
    ) -> Result<Converted, String> {
        if bytes.len() as u64 > limits.memory_bytes {
            return Err(format!(
                "input exceeds the memory limit of {} bytes",
                limits.memory_bytes
            ));
        }
        let (samples, dropped) = Samples::decode(bytes, limits.memory_bytes)?;
        let mut converted = match to {
            "wav" => samples.to_wav(),
            "flac" => samples.to_flac(lossy),
            "mp3" => samples.to_mp3(),
            "ogg" | "opus" => samples.to_opus(),
            other => Err(format!("oc-audio does not write {other}")),
        }?;
        // Extraction from a video container drops everything that is not the
        // chosen audio track, and SR-11 wants that ON the receipt rather than
        // inferred from the file being smaller.
        converted.removed.extend(dropped);
        Ok(converted)
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// How the first packet classified the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Integer PCM at its native width.
    Int(u32),
    /// Float PCM. FLAC refuses this; WAV carries it verbatim.
    Float,
}

/// A whole decoded stream, kept in the shape the source had it.
///
/// Integer values are stored **full-scale**, exactly as symphonia widened
/// them; each writer narrows with one arithmetic shift. Storing anything
/// narrower would mean deciding fidelity twice.
enum Samples {
    Int {
        interleaved: Vec<i32>,
        channels: usize,
        sample_rate: u32,
        bits: u32,
    },
    Float {
        interleaved: Vec<f32>,
        channels: usize,
        sample_rate: u32,
    },
}

impl Samples {
    /// Decode the entire stream, refusing before allocation outruns the
    /// budget.
    ///
    /// The cap counts DECODED bytes, not input bytes: an MP3 is roughly 11Ã—
    /// smaller than its own PCM, so bounding the input alone bounds nothing.
    /// This is the audio analogue of checking pixel dimensions before
    /// allocating a bitmap.
    fn decode(bytes: &[u8], decoded_budget: u64) -> Result<(Self, Vec<String>), String> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
        use symphonia::core::errors::Error as SymphoniaError;

        // OGG OPUS TAKES OUR OWN DECODER, NOT SYMPHONIA'S.
        //
        // symphonia 0.5 has no Opus decoder, so this branch is the difference
        // between reading an Opus file and refusing one -- including the Opus
        // files this very worker writes. libopus was already linked for the
        // encoder; only the container half was missing.
        //
        // Ogg VORBIS falls through to symphonia, which does decode it. The
        // test is `OpusHead` at the front of the first page, not the `.ogg`
        // extension or even the Ogg magic alone.
        #[cfg(has_opus)]
        if opus::is_ogg_opus(bytes) {
            let (interleaved, channels) = opus::decode_ogg_opus(bytes, decoded_budget)?;
            return Ok((
                Self::Int {
                    // libopus decodes to 16-bit, and the rest of this worker
                    // carries integers full-scale in an i32 -- the same widening
                    // symphonia applies, so `narrow` and `to_i16_for_lossy` need
                    // no special case for this path.
                    interleaved: interleaved.iter().map(|&v| i32::from(v) << 16).collect(),
                    bits: 16,
                    channels,
                    // Opus is defined at 48 kHz; see `decode_ogg_opus`.
                    sample_rate: 48_000,
                },
                Vec::new(),
            ));
        }

        let mut reader = open_reader(bytes)?;

        // PICK A TRACK WE CAN ACTUALLY DECODE, NOT MERELY THE FIRST ONE.
        //
        // "The first track whose codec is not NULL" is right for a bare audio
        // file, where there is one track and it is the point. A Matroska file
        // is the case it was never asked about: tracks come in container
        // order, video usually first, and constructing the decoder was left to
        // fail afterwards with "could not create a decoder for this stream" --
        // a true sentence about the wrong track.
        //
        // Trying each in turn and keeping the first that BUILDS is the same
        // decision made against the real question. Everything skipped is
        // counted, because a conversion that silently drops a video track is
        // the kind of quiet loss SR-11 exists to disclose.
        let candidates: Vec<_> = reader
            .tracks()
            .iter()
            .filter(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .map(|t| (t.id, t.codec_params.clone()))
            .collect();
        let total_tracks = reader.tracks().len();

        let mut chosen = None;
        // Why each candidate was turned down, kept for the refusal. Discarding
        // these and saying "no decodable audio track found" is true and names
        // the wrong problem: a 5.1 AAC file HAS an audio track, and the reason
        // it did not convert is that this build's decoder stops at stereo.
        let mut refusals: Vec<String> = Vec::new();
        for (id, params) in candidates {
            match symphonia::default::get_codecs().make(
                &params,
                // `verify` would double-check FLAC's MD5; the receipt already
                // hashes what we read, and a converter's honesty budget is
                // better spent on refusing corrupt packets outright below.
                &DecoderOptions { verify: false },
            ) {
                Ok(d) => {
                    chosen = Some((id, params, d));
                    break;
                }
                Err(e) => refusals.push(format!("track {id}: {e}")),
            }
        }
        let (track_id, codec_params, mut decoder) = chosen.ok_or_else(|| {
            if refusals.is_empty() {
                "this file contains no audio track at all".to_string()
            } else {
                format!(
                    "this file's audio is in a form this build cannot decode ({})",
                    refusals.join("; ")
                )
            }
        })?;

        let mut dropped: Vec<String> = Vec::new();
        if total_tracks > 1 {
            dropped.push(format!(
                "{} other track(s) in the source container, including any video",
                total_tracks - 1
            ));
        }

        let mut kind: Option<Kind> = None;
        let mut ints: Vec<i32> = Vec::new();
        let mut floats: Vec<f32> = Vec::new();
        let mut decoded_bytes: u64 = 0;
        let mut channels = codec_params.channels.map_or(0, |c| c.count());
        let mut sample_rate = codec_params.sample_rate.unwrap_or(0);

        loop {
            let packet = match reader.next_packet() {
                Ok(p) => p,
                // A clean EOF is how every well-formed stream ends.
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(e) => return Err(format!("could not read the stream: {e}")),
            };
            // Containers may interleave tracks we were not asked about.
            if packet.track_id() != track_id {
                continue;
            }
            // One corrupt packet fails the conversion rather than being
            // skipped. Players skip; converters must not â€” silently dropping
            // samples under a receipt that says nothing about it is exactly
            // the dishonesty this product exists to end.
            let buf = decoder.decode(&packet).map_err(|e| {
                format!(
                    "could not decode this stream near timestamp {}: {e}",
                    packet.ts()
                )
            })?;

            let container_kind = classify(&buf)?;
            // The DECODER's container width is not the signal's width:
            // symphonia's FLAC decoder hands every stream out as full-scale
            // S32 buffers regardless of whether the file carries 16 or 24
            // significant bits. The header said how wide the signal really
            // is; trusting the variant alone turned every 16-bit source into
            // a 32-bit WAV, which is exactly the silent widening this product
            // refuses to do.
            let this_kind = match (container_kind, codec_params.bits_per_sample) {
                (Kind::Int(container), Some(signal)) if signal < container => Kind::Int(signal),
                (kind, _) => kind,
            };
            if let Some(first) = kind {
                if first != this_kind {
                    return Err(
                        "this stream changes sample type part-way through, which no real encoder \
                         produces and no converter should pretend to understand"
                            .into(),
                    );
                }
            } else {
                kind = Some(this_kind);
                channels = buf.spec().channels.count();
                sample_rate = buf.spec().rate;
            }
            let frames = buf.frames() as u64;
            let ch = buf.spec().channels.count();

            match container_kind {
                // The samples are always collected at full scale; the KIND
                // records the true width the writers will narrow to.
                Kind::Int(_) => {
                    let mut sb = SampleBuffer::<i32>::new(frames, *buf.spec());
                    sb.copy_interleaved_ref(buf);
                    ints.extend_from_slice(sb.samples());
                    decoded_bytes += frames * ch as u64 * u64::from(this_kind_width(this_kind)) / 8;
                }
                Kind::Float => {
                    let mut sb = SampleBuffer::<f32>::new(frames, *buf.spec());
                    sb.copy_interleaved_ref(buf);
                    floats.extend_from_slice(sb.samples());
                    decoded_bytes += frames * ch as u64 * 4;
                }
            }

            if decoded_bytes > decoded_budget {
                return Err(format!(
                    "decoded audio exceeds the memory limit of {decoded_budget} bytes; \
                     raise --memory to convert longer streams"
                ));
            }
        }

        if ints.is_empty() && floats.is_empty() {
            return Err("this stream contains no decodable audio".into());
        }

        match kind {
            Some(Kind::Float) => Ok((
                Samples::Float {
                    interleaved: floats,
                    channels,
                    sample_rate,
                },
                dropped,
            )),
            Some(Kind::Int(bits)) => Ok((
                Samples::Int {
                    interleaved: ints,
                    channels,
                    sample_rate,
                    bits,
                },
                dropped,
            )),
            None => unreachable!("non-empty stream always classified"),
        }
    }

    fn channels(&self) -> usize {
        match self {
            Self::Int { channels, .. } | Self::Float { channels, .. } => *channels,
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            Self::Int { sample_rate, .. } | Self::Float { sample_rate, .. } => *sample_rate,
        }
    }

    /// Narrow one full-scale i32 back to its native width.
    ///
    /// Arithmetic shift, not masking: symphonia widened by shifting left, so
    /// shifting right recovers the original value exactly â€” including
    /// sign. This line is why `FLAC â‡„ WAV` may honestly claim Class A.
    fn narrow(v: i32, bits: u32) -> i32 {
        v >> (32 - bits)
    }

    /// WAV out, through hound.
    fn to_wav(&self) -> Result<Converted, String> {
        use hound::SampleFormat;
        let spec = hound::WavSpec {
            channels: self.channels() as u16,
            sample_rate: self.sample_rate(),
            bits_per_sample: match self {
                Self::Int { bits, .. } => *bits as u16,
                Self::Float { .. } => 32,
            },
            sample_format: match self {
                Self::Int { .. } => SampleFormat::Int,
                Self::Float { .. } => SampleFormat::Float,
            },
        };
        // hound's `finalize` consumes the writer and returns nothing, so the
        // buffer is ours from the start: the writer borrows it, and the bytes
        // are read back out of OUR cursor after finalisation.
        let mut buf = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(&mut buf, spec)
                .map_err(|e| format!("could not start the WAV writer: {e}"))?;
            match self {
                Self::Int {
                    interleaved, bits, ..
                } => {
                    for &v in interleaved {
                        writer
                            .write_sample(Self::narrow(v, *bits))
                            .map_err(|e| format!("could not write samples: {e}"))?;
                    }
                }
                Self::Float { interleaved, .. } => {
                    for &v in interleaved {
                        writer
                            .write_sample(v)
                            .map_err(|e| format!("could not write samples: {e}"))?;
                    }
                }
            }
            writer
                .finalize()
                .map_err(|e| format!("could not finalise the WAV: {e}"))?;
        }
        let bytes = buf.into_inner();
        Ok(Converted {
            bytes,
            // Tags live in the source container and WAV carries none of them;
            // the SAMPLES are what this conversion promises to keep, and the
            // receipt says so.
            removed: vec!["all tags and metadata (not carried across containers)".to_string()],
        })
    }

    /// FLAC out, through flacenc. Integer-only by design.
    fn to_flac(&self, lossy_route: bool) -> Result<Converted, String> {
        // `Cow`, because one arm borrows the decoded samples and the other
        // builds new ones; cloning in the common case to keep the types tidy
        // would copy a whole track for nothing.
        let quantised = lossy_route && matches!(self, Self::Float { .. });
        let (interleaved, bits, channels, rate): (std::borrow::Cow<[i32]>, u32, usize, u32) =
            match self {
                Self::Int {
                    interleaved,
                    bits,
                    channels,
                    sample_rate,
                } => (
                    std::borrow::Cow::Borrowed(interleaved.as_slice()),
                    *bits,
                    *channels,
                    *sample_rate,
                ),
                // A CLASS B ROW MAY QUANTISE; A CLASS A ROW MAY NOT.
                //
                // `Wav -> Flac` is Class A, and quantising a float WAV on the
                // way in would file a lossy conversion under a lossless
                // receipt -- the one thing this must never do. `M4a -> Flac`
                // is Class B: the source was already lossy, the row says so,
                // and refusing there left every AAC file with no path to FLAC
                // at all.
                //
                // The worker cannot tell those apart. The host sends the
                // class, and this is where it is spent.
                Self::Float {
                    interleaved,
                    channels,
                    sample_rate,
                } if lossy_route => (
                    std::borrow::Cow::Owned(
                        interleaved
                            .iter()
                            .map(|&f| {
                                let c = if f.is_nan() { 0.0 } else { f.clamp(-1.0, 1.0) };
                                // 16-bit, carried full-scale in an i32 the way
                                // every integer source here already is.
                                ((c * 32767.0).round() as i32) << 16
                            })
                            .collect(),
                    ),
                    16,
                    *channels,
                    *sample_rate,
                ),
                Self::Float { .. } => {
                    return Err(
                        "this source is float PCM and this route is lossless, so writing FLAC would need a re-quantisation this route did not authorise. Convert to WAV instead."
                            .into(),
                    )
                }
            };
        if rate == 0 || channels == 0 || !(4..=32).contains(&bits) {
            return Err("the stream never reported a usable rate, width or channel count".into());
        }
        let narrowed: Vec<i32> = interleaved.iter().map(|&v| Self::narrow(v, bits)).collect();
        // The encoder wants a VERIFIED config: `into_verified` runs the
        // library's own consistency checks once, here, rather than letting a
        // bad parameter surface as a corrupt bitstream later.
        use flacenc::error::Verify as _;
        let verified = flacenc::config::Encoder::default()
            .into_verified()
            .map_err(|(_, e)| format!("the FLAC encoder configuration is invalid: {e}"))?;
        let src = flacenc::source::MemSource::from_samples(
            &narrowed,
            channels,
            bits as usize,
            rate as usize,
        );
        const BLOCK_SIZE: usize = 4096;
        let mut stream = flacenc::encode_with_fixed_block_size(&verified, src, BLOCK_SIZE)
            .map_err(|e| format!("could not encode FLAC: {e}"))?;

        // CANONICALISE THE STREAMINFO BLOCK SIZES.
        //
        // flacenc lowers `min_block_size` to the length of the final, partial
        // frame (4000 here), which leaves min â‰  max alongside FIXED-strategy
        // frames. That combination is spec-tolerated but symphonia â€” like the
        // reference decoder's conventions â€” treats min == max as the statement
        // "fixed blocking", refuses every frame of a file where they differ,
        // and its own STREAMINFO comment records the exception this exists for:
        // "the last block may be shorter than the minimum". libFLAC writes the
        // NOMINAL size in both fields for exactly this reason. Matching that
        // convention is not massaging a number; it is writing the one reading
        // of these fields that every mainstream decoder shares, and the
        // round-trip test below holds the pair together so neither side can
        // drift.
        stream
            .stream_info_mut()
            .set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
            .map_err(|e| format!("could not normalise FLAC block sizes: {e}"))?;
        // Serialize through the library's own bit-writer. Pre-sized from
        // `count_bits`, which the library computes exactly â€” one allocation,
        // no growth path.
        use flacenc::bitsink::ByteSink;
        use flacenc::component::BitRepr;
        let mut sink = ByteSink::with_capacity(stream.count_bits());
        stream
            .write(&mut sink)
            .map_err(|e| format!("could not serialise FLAC: {e}"))?;
        let mut removed = vec!["all tags and metadata (not carried across containers)".to_string()];
        if quantised {
            // The output is a lossless FILE holding samples that were rounded
            // on the way in. Both halves of that have to be on the receipt, or
            // "FLAC" reads as "nothing was lost".
            removed.push(
                "float samples quantised to 16-bit; the source was already lossy".to_string(),
            );
        }
        Ok(Converted {
            bytes: sink.into_inner(),
            removed,
        })
    }

    /// Interleaved i16 for the LOSSY encoders, with what that cost the file.
    ///
    /// # Why this exists, and why only for the lossy targets
    ///
    /// Both encoders used to refuse float sources outright — "a re-quantisation
    /// decision this build does not make". That reads as caution and behaved as
    /// a dead end: symphonia decodes MP3, Vorbis and AAC to **float**, so
    /// `mp3 -> mp3`, `mp3 -> ogg`, `ogg -> mp3` and `ogg -> ogg` were four
    /// route rows that could not succeed for ANY input, not merely for unusual
    /// ones. The table advertised them; nothing could ever finish them.
    ///
    /// Refusing to decide was itself a decision, and the worse one. The
    /// destination here is MP3 or Opus: a lossy encoder that will quantise the
    /// signal far more aggressively than this does, in its own domain, a
    /// moment later. Declining to convert float to 16-bit before handing it to
    /// a psychoacoustic codec does not preserve anything.
    ///
    /// **`to_flac` deliberately still refuses**, and that is not an
    /// inconsistency. FLAC is lossless and `Wav -> Flac` is a Class A row: a
    /// float WAV quantised on the way into FLAC would be a lossy conversion
    /// wearing a lossless class, which is precisely the receipt this project
    /// must never write. Lossy targets may quantise because they are already
    /// declared lossy; the lossless one may not.
    ///
    /// The rule, stated rather than implied: clamp to [-1.0, 1.0], scale by
    /// 32767, round half away from zero. No dither — dither is a mastering
    /// choice, and adding noise a user did not ask for is the other way to be
    /// dishonest here.
    ///
    /// Gated to match its callers exactly. `to_mp3` needs `has_lame` and
    /// `to_ogg` needs `has_opus`; on a machine with neither -- a plain CI
    /// container, which is what found this -- both compile to their stubs and
    /// this becomes dead code. CI runs clippy with `-D warnings`, so "dead" is
    /// "the build fails", and it failed on Linux and nowhere else.
    #[cfg(any(has_lame, has_opus))]
    fn to_i16_for_lossy(&self) -> Result<(Vec<i16>, usize, u32, Vec<String>), String> {
        let mut removed = vec!["all tags and metadata (not carried across containers)".to_string()];
        let (samples, channels, sample_rate) = match self {
            Self::Int {
                interleaved,
                channels,
                sample_rate,
                ..
            } => {
                // Full-scale i32 to i16 is always one arithmetic shift: every
                // width stores its significant bits at the top of the i32.
                let v = interleaved.iter().map(|&v| (v >> 16) as i16).collect();
                (v, *channels, *sample_rate)
            }
            Self::Float {
                interleaved,
                channels,
                sample_rate,
            } => {
                let v = interleaved
                    .iter()
                    .map(|&f| {
                        let clamped = if f.is_nan() { 0.0 } else { f.clamp(-1.0, 1.0) };
                        // `round` is half-away-from-zero, and 32767 rather
                        // than 32768 so +1.0 lands exactly on i16::MAX
                        // instead of wrapping.
                        (clamped * 32767.0).round() as i16
                    })
                    .collect();
                removed
                    .push("float samples quantised to 16-bit before the lossy encode".to_string());
                (v, *channels, *sample_rate)
            }
        };
        if sample_rate == 0 || channels == 0 {
            return Err("the stream never reported a usable rate or channel count".into());
        }
        Ok((samples, channels, sample_rate, removed))
    }

    /// MP3 out, through LAME, from [`Self::to_i16_for_lossy`] — which is
    /// where the float-source rule and its disclosure both live.
    #[cfg(has_lame)]
    fn to_mp3(&self) -> Result<Converted, String> {
        const DEFAULT_KBPS: u32 = 192;
        let (narrowed, channels, sample_rate, removed) = self.to_i16_for_lossy()?;
        let bytes = lame::encode_mp3(&narrowed, channels, sample_rate, DEFAULT_KBPS)?;
        Ok(Converted { bytes, removed })
    }

    /// MP3 out on a build whose LAME import library was missing at build
    /// time: refuse by name instead of pretending the format is writable.
    #[cfg(not(has_lame))]
    fn to_mp3(&self) -> Result<Converted, String> {
        let _ = self;
        Err(
            "MP3 output needs LAME, which was not found at build time; this build writes \
             wav and flac"
                .into(),
        )
    }

    /// Opus out (Ogg container), through libopus at its fixed 48 kHz — the
    /// module resamples from the source rate itself. Same
    /// [`Self::to_i16_for_lossy`] rule as [`Self::to_mp3`].
    #[cfg(has_opus)]
    fn to_opus(&self) -> Result<Converted, String> {
        let (narrowed, channels, sample_rate, removed) = self.to_i16_for_lossy()?;
        let bytes = opus::encode_opus(&narrowed, channels, sample_rate)?;
        Ok(Converted { bytes, removed })
    }

    /// Opus out on a build without the libopus import library.
    #[cfg(not(has_opus))]
    fn to_opus(&self) -> Result<Converted, String> {
        let _ = self;
        Err(
            "Opus output needs libopus, which was not found at build time; this build \
             writes wav and flac"
                .into(),
        )
    }
}

/// Which family a decoded buffer belongs to, from its variant alone.
///
/// Unsigned widths above 8 bits are refused rather than reinterpreted: WAV
/// carries unsigned samples only at 8 bits, so a U16+ buffer means an
/// intermediate representation no real container asked us for, and guessing
/// its centre point would be deciding fidelity by accident.
fn classify(buf: &symphonia::core::audio::AudioBufferRef<'_>) -> Result<Kind, String> {
    use symphonia::core::audio::AudioBufferRef as A;
    Ok(match buf {
        A::U8(_) | A::S8(_) => Kind::Int(8),
        A::S16(_) => Kind::Int(16),
        A::S24(_) => Kind::Int(24),
        A::S32(_) => Kind::Int(32),
        A::F32(_) | A::F64(_) => Kind::Float,
        A::U16(_) | A::U24(_) | A::U32(_) => {
            return Err(
                "this stream uses unsigned PCM above 8 bits, which no common container carries; \
                 this build does not guess its conversion"
                    .into(),
            )
        }
    })
}

/// The width a `Kind` names, for budget accounting.
const fn this_kind_width(kind: Kind) -> u32 {
    match kind {
        Kind::Int(bits) => bits,
        Kind::Float => 32,
    }
}

/// Open a format reader over the bytes. Header work only.
///
/// The hint carries no extension â€” identity comes from content alone, which
/// is SR-4 applied inside the worker too.
fn open_reader(bytes: &[u8]) -> Result<Box<dyn symphonia::core::formats::FormatReader>, String> {
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    let mss = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let hint = Hint::new();
    symphonia::default::get_probe()
        .format(&hint, mss, &Default::default(), &MetadataOptions::default())
        .map(|probed| probed.format)
        .map_err(|e| format!("could not recognise this stream: {e}"))
}
