//! Drive the real `oc-audio` binary as a real subprocess.
//!
//! Same discipline as `oc-images/tests/end_to_end.rs`: the protocol is
//! exercised over raw pipes against the built program, because a closed enum
//! of programs is only meaningful if the programs run.

use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};
use std::io::Write;
use std::process::{Command, Stdio};

/// A short 16-bit stereo WAV at 8 kHz, built here so the test needs no fixture.
fn wav() -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
    // A deterministic ramp; large enough to survive FLAC framing, small
    // enough that this test stays fast.
    for i in 0..4000i32 {
        w.write_sample((i % 3000) - 1500).unwrap();
        w.write_sample((i * 7 % 3000) - 1500).unwrap();
    }
    w.finalize().unwrap();
    buf.into_inner()
}

fn worker_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current exe");
    p.pop(); // deps/
    p.pop(); // target/<profile>/
    p.join(format!("oc-audio{}", std::env::consts::EXE_SUFFIX))
}

fn talk(requests: &[(Request, Vec<u8>)]) -> Vec<(Response, Vec<u8>)> {
    let exe = worker_path();
    assert!(
        exe.exists(),
        "oc-audio is not built at {} -- this test needs the binary",
        exe.display()
    );
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn oc-audio");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for (r, content) in requests {
            let body = serde_json::to_vec(r).unwrap();
            write_frame(&mut stdin, &body).unwrap();
            write_content(&mut stdin, content).unwrap();
        }
        stdin.flush().unwrap();
    }
    let out = child.wait_with_output().expect("wait");
    assert!(
        out.status.success(),
        "oc-audio exited with {:?}",
        out.status.code()
    );

    let mut cursor = std::io::Cursor::new(out.stdout);
    let mut answers = Vec::new();
    while let Ok(frame) = read_frame(&mut cursor) {
        let response: Response = serde_json::from_slice(&frame).unwrap();
        let content = match response {
            Response::Done { output_len, .. } => {
                read_content(&mut cursor, output_len, u64::MAX).expect("content")
            }
            _ => Vec::new(),
        };
        answers.push((response, content));
    }
    answers
}

fn lim(memory_bytes: u64) -> RunLimits {
    RunLimits {
        decode_pixels: u64::MAX,
        memory_bytes,
        archive_depth: 32,
        archive_entries: 100_000,
        archive_total_bytes: 1 << 30,
        use_gpu: true,
    }
}

fn req(r: Request, content: &[u8]) -> (Request, Vec<u8>) {
    (r, content.to_vec())
}

/// Hello names the engine; probe reads rate and channels from the header.
#[test]
fn hello_and_probe_answer_from_the_header_alone() {
    let src = wav();
    let answers = talk(&[
        req(Request::Hello, &[]),
        req(
            Request::Probe {
                input_len: src.len() as u64,
            },
            &src,
        ),
    ]);
    let Response::Ready { engine, .. } = &answers[0].0 else {
        panic!("expected Ready, got {:?}", answers[0].0);
    };
    assert_eq!(engine, "oc-audio");

    let Response::Properties(openconvert_worker::Probed::Audio {
        sample_rate,
        channels,
        duration_ms,
    }) = &answers[1].0
    else {
        panic!("expected audio Properties, got {:?}", answers[1].0);
    };
    assert_eq!(*sample_rate, 8000);
    assert_eq!(*channels, 2);
    assert_eq!(*duration_ms, 0, "duration is never guessed from a header");
}

/// **The Class A claim, held end to end.** WAV in, FLAC out, decoded back:
/// every sample identical. This is the property the route table asserts when
/// it marks `Wav → Flac` Class A, and here it is measured through the real
/// binary rather than trusted from it.
#[test]
fn wav_to_flac_round_trips_bit_identically_through_the_real_worker() {
    use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
    use symphonia::core::errors::Error as SymphoniaError;

    let original = wav();

    // Collect the source's own samples first — the reference the output must
    // match exactly. Converted through SampleBuffer<i16> so BOTH sides pass
    // through the identical widening/narrowing convention: symphonia's FLAC
    // decoder emits full-scale S32 buffers, and its own converter narrows
    // them back with the exact inverse shift.
    fn decode_i16(bytes: &[u8]) -> Vec<i16> {
        let mss = symphonia::core::io::MediaSourceStream::new(
            Box::new(std::io::Cursor::new(bytes.to_vec())),
            Default::default(),
        );
        let mut reader = symphonia::default::get_probe()
            .format(
                &Default::default(),
                mss,
                &Default::default(),
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("probe of {} bytes failed: {e:?}", bytes.len()))
            .format;
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("track");
        let params = track.codec_params.clone();
        let track_id = track.id;
        let mut dec = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions { verify: false })
            .expect("decoder");
        let mut out = Vec::new();
        loop {
            match reader.next_packet() {
                Ok(p) => {
                    if p.track_id() != track_id {
                        continue;
                    }
                    let b = dec.decode(&p).expect("decode");
                    let mut sb = symphonia::core::audio::SampleBuffer::<i16>::new(
                        b.frames() as u64,
                        *b.spec(),
                    );
                    sb.copy_interleaved_ref(b);
                    out.extend_from_slice(sb.samples());
                }
                Err(SymphoniaError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break
                }
                Err(e) => panic!("decode error: {e}"),
            }
        }
        out
    }

    let expected = decode_i16(&original);

    let answers = talk(&[req(
        Request::Run {
            input_len: original.len() as u64,
            to: "flac".into(),
            limits: lim(1 << 26),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &original,
    )]);
    let (Response::Done { removed, .. }, flac_bytes) = &answers[0] else {
        panic!("expected Done, got {:?}", answers[0].0);
    };
    assert!(flac_bytes.starts_with(b"fLaC"), "output is not FLAC");
    assert!(
        removed.iter().any(|r| r.contains("metadata")),
        "tag loss must be disclosed: {removed:?}"
    );

    let got = decode_i16(flac_bytes);
    assert_eq!(
        got.len(),
        expected.len(),
        "sample count changed across the lossless pair"
    );
    assert_eq!(got, expected, "samples differ across a CLASS A conversion");
}

/// The reverse pair is also Class A: FLAC back to WAV, samples intact.
#[test]
fn flac_to_wav_preserves_the_samples() {
    // Build the FLAC via the worker once, then convert it back.
    let original = wav();
    let answers = talk(&[req(
        Request::Run {
            input_len: original.len() as u64,
            to: "flac".into(),
            limits: lim(1 << 26),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &original,
    )]);
    let (_, flac) = &answers[0];
    assert!(flac.starts_with(b"fLaC"));

    let answers = talk(&[req(
        Request::Run {
            input_len: flac.len() as u64,
            to: "wav".into(),
            limits: lim(1 << 26),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        flac,
    )]);
    let (Response::Done { .. }, wav_bytes) = &answers[0] else {
        panic!("expected Done, got {:?}", answers[0].0);
    };
    assert!(wav_bytes.starts_with(b"RIFF"), "output is not WAV");

    // Both ends of the round trip must agree bit for bit.
    fn pcm(bytes: &[u8]) -> Vec<u8> {
        // Data chunk payload: find "data" and take the declared length.
        let pos = bytes
            .windows(4)
            .position(|w| w == b"data")
            .expect("data chunk");
        let len = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]) as usize;
        bytes[pos + 8..pos + 8 + len].to_vec()
    }
    assert_eq!(
        pcm(wav_bytes),
        pcm(&original),
        "FLAC → WAV did not preserve the PCM stream"
    );
}

/// MP3 output answers through the REAL encoder chain, and each answer pins
/// its own build. Where LAME was linked at build time, the output is genuine
/// MPEG audio (sync word, version/layer bits); where its import library was
/// missing, the refusal names what is missing and is not retryable — an
/// encoder we cannot ship is still a route the table must not advertise.
#[test]
fn mp3_output_is_valid_mpeg_or_refused_by_name() {
    let src = wav();
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "mp3".into(),
            limits: lim(1 << 24),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    match &answers[0].0 {
        Response::Done { output_len, .. } => {
            assert!(
                *output_len > 0 && answers[0].1.len() == *output_len as usize,
                "content length must match the announcement"
            );
            let mp3 = &answers[0].1;
            assert!(
                mp3.first() == Some(&0xFF) && (mp3[1] & 0xE0) == 0xE0,
                "output does not start with an MPEG audio sync word: {:02X?}",
                &mp3[..4.min(mp3.len())]
            );
        }
        Response::Failed {
            message, retryable, ..
        } => {
            assert!(
                message.contains("LAME"),
                "the refusal should name what is missing: {message}"
            );
            assert!(!*retryable, "retrying will not link LAME");
        }
        other => panic!("expected Done or Failed, got {other:?}"),
    }
}

/// Float sources refuse FLAC rather than inventing quantisation decisions.
#[test]
fn float_pcm_refuses_flac_rather_than_requantising() {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 4000,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut buf = Cursor::new(Vec::new());
    let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
    for i in 0..256i32 {
        w.write_sample(i as f32 / 256.0).unwrap();
    }
    w.finalize().unwrap();

    let answers = talk(&[req(
        Request::Run {
            input_len: buf.get_ref().len() as u64,
            to: "flac".into(),
            limits: lim(1 << 22),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        buf.get_ref(),
    )]);
    let Response::Failed { message, .. } = &answers[0].0 else {
        panic!("expected Failed, got {:?}", answers[0].0);
    };
    assert!(message.contains("float"), "{message}");
}

/// Garbage produces a structured failure, never a crash.
#[test]
fn malformed_audio_produces_a_failure_not_a_crash() {
    let junk = b"RIFFxxxxWAVEjunkjunkjunk";
    let answers = talk(&[req(
        Request::Run {
            input_len: junk.len() as u64,
            to: "flac".into(),
            limits: lim(1 << 20),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        junk,
    )]);
    assert!(
        matches!(answers[0].0, Response::Failed { .. }),
        "expected a structured failure, got {:?}",
        answers[0].0
    );
}

// `hound` and `symphonia` are main dependencies of this package, so the
// fixtures above can use them directly; serde_json comes from dev-deps like
// everywhere else.
use std::io::Cursor;
