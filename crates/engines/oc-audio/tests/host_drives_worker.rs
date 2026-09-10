//! The host half, tested where the binary is **guaranteed to exist**.
//!
//! Same reasoning as `oc-images/tests/host_drives_worker.rs`: these started
//! life beside `current_exe()` and skipped under `cargo mutants`. Here there
//! is no skip path — `CARGO_BIN_EXE_oc-audio` makes Cargo build the binary
//! before this test compiles.

use openconvert_core::limits::Limits;
use openconvert_run::worker_client::{Worker, WorkerError};

const TX_AUDIO: &str = env!("CARGO_BIN_EXE_oc-audio");

fn start() -> Worker {
    Worker::start_at(TX_AUDIO.as_ref(), "oc-audio", &Limits::defaults()).expect("start oc-audio")
}

/// A 16-bit stereo WAV fixture, built here.
fn wav() -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
        for i in 0..2000i32 {
            w.write_sample((i % 1000) - 500).unwrap();
            w.write_sample((i * 3 % 1000) - 500).unwrap();
        }
        w.finalize().unwrap();
    }
    buf.into_inner()
}

/// The loop, closed for the audio engine: host starts the confined worker,
/// speaks the protocol, gets FLAC back.
#[test]
fn the_host_converts_audio_through_the_confined_worker() {
    let mut worker = start();

    if cfg!(target_os = "linux") {
        // The Linux self-confinement claim, asserted from the handshake --
        // same as the image worker's host test. A false here means the
        // worker started unconfined, which is the failure SR-1 exists to
        // prevent on this platform.
        let engaged = |name: &str| {
            worker
                .mitigations()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, on)| *on)
        };
        assert_eq!(
            engaged("Landlock"),
            Some(true),
            "{:?}",
            worker.mitigations()
        );
        assert_eq!(engaged("Seccomp"), Some(true), "{:?}", worker.mitigations());
    }

    let (bytes, removed) = worker
        .run(wav(), "flac", &Limits::defaults(), &[])
        .expect("convert");
    assert!(bytes.starts_with(b"fLaC"));
    assert!(!removed.is_empty(), "tag loss must be disclosed");
}

/// A format this build cannot write refuses as a REASON, not a dead pipe;
/// a format it can writes real bytes. MP3 is both, depending on whether the
/// LAME import library existed at build time — each outcome pins itself.
#[test]
fn an_unwritable_format_refuses_with_a_reason() {
    let mut worker = start();
    match worker.run(wav(), "mp3", &Limits::defaults(), &[]) {
        Ok((bytes, removed)) => {
            assert!(
                bytes.len() > 4 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0,
                "output is not MPEG audio"
            );
            assert!(!removed.is_empty(), "tag loss must be disclosed");
        }
        Err(WorkerError::Engine {
            engine,
            message,
            retryable,
        }) => {
            assert_eq!(engine, "oc-audio", "the HOST names the engine");
            assert!(
                message.contains("LAME"),
                "the refusal should name what is missing: {message}"
            );
            assert!(!retryable);
        }
        Err(other) => panic!("expected a structured refusal, got {other}"),
    }
}

// ---------------------------------------------------------------------------
// peaks: the waveform, measured rather than invented
// ---------------------------------------------------------------------------

/// A WAV whose first half is silent and second half is full-scale.
///
/// A shape the assertions can be certain about: any correct bucketing puts
/// near-zero in the first half and near-maximum in the second, whatever the
/// bucket count.
fn wav_quiet_then_loud() -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 8000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = hound::WavWriter::new(&mut buf, spec).unwrap();
        for _ in 0..8000 {
            w.write_sample(0i16).unwrap();
        }
        for i in 0..8000 {
            // Alternating full-scale, so every bucket in the second half
            // contains a peak rather than only the ones a slow wave happens
            // to crest in.
            w.write_sample(if i % 2 == 0 { i16::MAX } else { i16::MIN })
                .unwrap();
        }
        w.finalize().unwrap();
    }
    buf.into_inner()
}

fn peaks_of(bytes: Vec<u8>, buckets: usize) -> Vec<u8> {
    let mut worker = start();
    let (out, _removed) = worker
        .run_with_inputs(
            bytes,
            &[],
            "wav",
            &Limits::defaults(),
            &[
                ("op".to_string(), "peaks".to_string()),
                ("buckets".to_string(), buckets.to_string()),
            ],
        )
        .expect("the worker refused a peaks request it should have accepted");
    out
}

/// **The waveform describes the audio.**
///
/// The tool workspace used to draw this shape from a seeded random number
/// generator and label it with the user's filename — a picture of a file the
/// program had never read. This is the property that makes it a measurement:
/// silence reads quiet and full scale reads loud, in the right halves.
#[test]
fn peaks_follow_the_audio() {
    let out = peaks_of(wav_quiet_then_loud(), 16);
    assert_eq!(out.len(), 16, "one byte per bucket");

    let (quiet, loud) = out.split_at(8);
    assert!(
        quiet.iter().all(|&v| v <= 2),
        "the silent half must read silent, got {quiet:?}"
    );
    assert!(
        loud.iter().all(|&v| v >= 250),
        "the full-scale half must read loud, got {loud:?}"
    );
}

/// The caller decides how many bars, within a bound.
///
/// The clamp is not decoration: `buckets` arrives from the frontend, and a
/// request for ten million bars is a denial of service with extra steps.
#[test]
fn the_bucket_count_is_honoured_and_bounded() {
    assert_eq!(peaks_of(wav(), 1).len(), 1);
    assert_eq!(peaks_of(wav(), 96).len(), 96);
    assert_eq!(peaks_of(wav(), 2048).len(), 2048);
    assert_eq!(
        peaks_of(wav(), 100_000).len(),
        2048,
        "an absurd request is clamped, not honoured and not refused"
    );
}

/// A stereo file is bucketed by FRAME, not by interleaved sample.
///
/// Bucketing the interleaved run would make the left and right channels
/// alternate down the timeline rather than share it, so the same recording
/// would draw a different shape depending on its channel count. The fixture is
/// stereo and 2000 frames long; every bucket must still see real audio.
#[test]
fn stereo_is_bucketed_by_frame() {
    let out = peaks_of(wav(), 8);
    assert_eq!(out.len(), 8);
    assert!(
        out.iter().all(|&v| v > 0),
        "every bucket of a continuous stereo signal has audio in it: {out:?}"
    );
}

/// Something that is not audio is refused, not answered with a flat line.
#[test]
fn peaks_refuse_what_they_cannot_decode() {
    let mut worker = start();
    let err = worker
        .run_with_inputs(
            b"this is not a recording".to_vec(),
            &[],
            "wav",
            &Limits::defaults(),
            &[("op".to_string(), "peaks".to_string())],
        )
        .expect_err("arbitrary bytes are not a recording");
    assert!(
        matches!(err, WorkerError::Engine { .. }),
        "a refusal has to come back as the engine's own message, got {err:?}"
    );
}
