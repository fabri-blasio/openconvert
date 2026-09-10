//! The host half, tested where the binary is **guaranteed to exist**.
//!
//! # Why these live here and not in `openconvert-run`
//!
//! They started there, resolved `oc-images` beside `current_exe()`, and printed
//! a loud skip when it was not found. Under `cargo test` at the workspace root
//! that skip almost never fired, so the tests looked like coverage.
//!
//! Then mutation testing ran. In the `mutants.out` build tree the binary is not
//! beside the harness, every one of them skipped, and **33 mutants survived in
//! the code they were written to cover** — including `Worker::run` replaced by
//! `Ok((vec![], vec![]))`, which is a host that reports success having converted
//! nothing.
//!
//! `CARGO_BIN_EXE_oc-images` is only defined for tests **inside this package**,
//! and Cargo builds the binary before running them. There is no skip path here
//! because there is nothing to skip over: if the binary is missing, the test
//! does not compile.
//!
//! `oc-images/tests/end_to_end.rs` drives the same binary over raw pipes. This
//! drives it through the real host — `spawn_piped`, the Job Object, the
//! handshake, `DisplayName` on the way back.

use openconvert_core::limits::Limits;
use openconvert_run::worker_client::{Worker, WorkerError};

/// The binary, resolved by Cargo rather than guessed.
const TX_IMAGES: &str = env!("CARGO_BIN_EXE_oc-images");

fn start() -> Worker {
    Worker::start_at(TX_IMAGES.as_ref(), "oc-images", &Limits::defaults()).expect("start oc-images")
}

/// A real PNG, built here so the test needs no fixture file.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode");
    out.into_inner()
}

/// **The loop, closed.** `-run` asks `-os` for a confined process, speaks the
/// protocol to it, and gets bytes back.
///
/// Until this path existed, `execute()` answered every sandboxed step with
/// `NeedsSandbox`.
#[test]
fn the_host_starts_a_confined_worker_and_converts_through_it() {
    let mut worker = start();

    if cfg!(windows) {
        // Read back in the CHILD, which is the only place the answer means
        // anything (03 §9.5).
        assert_eq!(
            worker.mitigations().len(),
            3,
            "ACG, CIG and AppContainer should all be reported: {:?}",
            worker.mitigations()
        );

        // **The claim that carries SR-1, confirmed by the child's own token.**
        //
        // The host knows it passed PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES.
        // That is not the same as the kernel having honoured it, and this
        // project has twice measured that gap running in both directions
        // (S17b, S20b). So the worker asks `TokenIsAppContainer` of itself and
        // the host asserts the answer.
        let engaged = |name: &str| {
            worker
                .mitigations()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, on)| *on)
        };
        assert_eq!(
            engaged("AppContainer"),
            Some(true),
            "the worker is NOT in an AppContainer -- it can reach the filesystem \
             and the network: {:?}",
            worker.mitigations()
        );
        assert_eq!(
            engaged("ACG"),
            Some(true),
            "ACG engages inside a container whether or not it is requested (S17b)"
        );
        assert!(
            worker.container_name().starts_with("openconvert."),
            "the receipt needs a checkable container name, got {:?}",
            worker.container_name()
        );
    }

    if cfg!(target_os = "linux") {
        // **The host's Linux half of the same claim.**
        //
        // The host applies the address-space cap; the worker applies Landlock
        // and seccomp itself before reading anything. Both verdicts arrive in
        // the handshake, and both must be earned -- a `false` here means the
        // worker started unconfined, which on this platform is the failure
        // SR-1 exists to prevent.
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
            "the worker did not confine its filesystem: {:?}",
            worker.mitigations()
        );
        assert_eq!(
            engaged("Seccomp"),
            Some(true),
            "the worker did not deny its network: {:?}",
            worker.mitigations()
        );
    }

    let (bytes, removed) = worker
        .run(png(24, 24), "jpeg", &Limits::defaults(), &[])
        .expect("convert");
    assert!(
        bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "the worker did not return a JPEG ({} bytes)",
        bytes.len()
    );
    assert!(
        bytes.len() > 100,
        "a 24x24 JPEG is not {} bytes",
        bytes.len()
    );
    assert!(
        !removed.is_empty(),
        "a re-encode drops metadata and must say so"
    );
}

/// A worker that refuses answers with a **reason**, not a dead pipe.
///
/// `03` §13 needs `Engine` and `Crashed` distinct: one names a limit the user
/// can raise, the other names nothing at all.
#[test]
fn a_refusal_arrives_as_a_reason_not_a_crash() {
    let mut worker = start();
    let tight = Limits {
        decode_pixels: 100,
        ..Limits::defaults()
    };

    let err = worker
        .run(png(64, 64), "jpeg", &tight, &[])
        .expect_err("100 pixels is not enough for a 64x64 image");

    match err {
        WorkerError::Engine {
            engine,
            message,
            retryable,
        } => {
            assert_eq!(engine, "oc-images");
            assert!(
                message.contains("4096") && message.contains("100"),
                "the message should name both the actual and the limit: {message}"
            );
            assert!(retryable, "a limit is not a crash");
        }
        other => panic!("expected a structured refusal, got {other}"),
    }
}

/// The control: the same worker converts fine under a sane limit.
///
/// Without it, the test above is satisfied by a host that refuses everything.
#[test]
fn the_same_image_converts_under_a_sane_limit() {
    let mut worker = start();
    let (bytes, _) = worker
        .run(png(64, 64), "png", &Limits::defaults(), &[])
        .expect("convert");
    assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']));
}

/// Nothing crossing back from a worker reaches a screen unrendered.
///
/// `oc-images` echoes the requested format into its refusal, which makes that
/// string a channel from the request to the user's terminal. A worker holding a
/// malicious image could put anything there.
#[test]
fn a_workers_message_cannot_carry_terminal_control_codes() {
    let mut worker = start();
    let evil = "pdf\u{1b}]0;pwned\u{7}\u{202e}gnp";

    let err = worker
        .run(png(8, 8), evil, &Limits::defaults(), &[])
        .expect_err("oc-images cannot write that format");

    let WorkerError::Engine { message, .. } = &err else {
        panic!("expected a structured refusal, got {err}");
    };
    for c in message.chars() {
        assert!(
            !matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}' | '\u{202a}'..='\u{202e}'),
            "U+{:04X} reached the message: {message:?}",
            c as u32
        );
    }
    // Visible, not merely deleted. A silently stripped control character makes
    // two different names render identically, which is the confusion this is
    // meant to remove rather than relocate.
    assert!(
        message.contains("\\u{001b}") && message.contains("\\u{202e}"),
        "the codes should be shown as escapes: {message:?}"
    );
}

/// **A worker does not get to say who it is.**
///
/// `Failed` carries an engine name. Taking it would let a compromised
/// `oc-images` sign its errors `oc-pdf`, sending both the user filing the bug
/// and the maintainer reading it to the wrong component.
///
/// Here the honest worker agrees with the host, so the error carries the plain
/// name with no disagreement note — which is the half of the rule that a test
/// asserting only the mismatch case would leave uncovered.
#[test]
fn the_error_carries_the_hosts_name_for_the_engine() {
    let mut worker = start();
    let err = worker
        .run(png(8, 8), "heic", &Limits::defaults(), &[])
        .expect_err("oc-images cannot write HEIC");

    let WorkerError::Engine {
        engine,
        message,
        retryable,
    } = &err
    else {
        panic!("expected a structured refusal, got {err}");
    };
    assert_eq!(engine, "oc-images", "the host names the engine");
    assert!(
        !message.contains("identified itself as"),
        "an honest worker should produce no disagreement note: {message}"
    );
    assert!(!retryable, "retrying will not add HEIC support");
}

/// Two sessions in a row. The first worker is fully torn down.
///
/// `Drop` closes stdin and waits, and a `Drop` that did neither would leave a
/// process holding a pipe — visible only as an accumulating handle count, which
/// is the kind of leak nobody notices until it is thousands.
#[test]
fn each_session_ends_and_the_next_one_starts_clean() {
    for _ in 0..3 {
        let mut worker = start();
        let (bytes, _) = worker
            .run(png(16, 16), "png", &Limits::defaults(), &[])
            .expect("convert");
        assert!(!bytes.is_empty());
        drop(worker);
    }
}
