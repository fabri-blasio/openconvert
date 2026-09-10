//! The host half, tested where the binary is **guaranteed to exist**.
//!
//! Same reasoning as `oc-audio/tests/host_drives_worker.rs`: these started
//! life beside `current_exe()` and skipped under `cargo mutants`. Here there
//! is no skip path — `CARGO_BIN_EXE_oc-pdf` makes Cargo build the binary
//! before this test compiles.
//!
//! Platform gate: pdfium is linked on Windows builds only today; the stub
//! engine elsewhere would turn every assertion here into a false failure.

#![cfg(windows)]

use openconvert_core::format::FormatId;
use openconvert_core::limits::Limits;
use openconvert_run::worker_client::{Worker, WorkerError};

const TX_PDF: &str = env!("CARGO_BIN_EXE_oc-pdf");

fn start() -> Worker {
    Worker::start_at(TX_PDF.as_ref(), "oc-pdf", &Limits::defaults()).expect("start oc-pdf")
}

/// The same minimal one-page PDF the pipe-level tests build: real xref
/// offsets, so pdfium opens it honestly rather than repairing it.
fn pdf() -> Vec<u8> {
    let mut out = Vec::from(b"%PDF-1.4\n");
    let mut offsets = [0u32; 4];
    let objects = [
        (1, "<< /Type /Catalog /Pages 2 0 R >>"),
        (2, "<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 144 72] >>"),
    ];
    for (num, body) in objects {
        offsets[num] = out.len() as u32;
        out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(b"xref\n0 4\n0000000000 65535 f \n");
    for slot in &offsets[1..] {
        out.extend_from_slice(format!("{slot:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(b"trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n");
    out.extend_from_slice(format!("{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// The loop, closed for the PDF engine: host starts the confined worker,
/// speaks the protocol, gets PNG back.
#[test]
fn the_host_converts_a_pdf_through_the_confined_worker() {
    let mut worker = start();

    if cfg!(target_os = "linux") {
        // The Linux self-confinement claim, asserted from the handshake --
        // same as the audio worker's host test. A false here means the
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

    if cfg!(windows) {
        // The Windows twin of that canary. The spawn places the worker in an
        // AppContainer and asks for ACG at process creation; the handshake
        // must READ BACK what actually engaged. Asserting it turns this test
        // into the tripwire for a silent confinement regression: conversions
        // would otherwise keep succeeding green -- just unconfined, which is
        // the one outcome worse than failing. (CIG is not requested by the
        // spawn and reads back false; asserting it true would be the exact
        // claim-without-evidence the read-back exists to prevent.)
        let engaged = |name: &str| {
            worker
                .mitigations()
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, on)| *on)
        };
        assert_eq!(engaged("ACG"), Some(true), "{:?}", worker.mitigations());
        assert_eq!(
            engaged("AppContainer"),
            Some(true),
            "{:?}",
            worker.mitigations()
        );
    }

    let (bytes, removed) = worker
        .run(pdf(), FormatId::Png.name(), &Limits::defaults(), &[])
        .expect("convert");
    assert!(bytes.starts_with(b"\x89PNG"), "output is not PNG");
    assert!(
        !removed.is_empty(),
        "PDF metadata and annotation loss must be disclosed"
    );
}

/// A probe through the real host reports the document's page count in the
/// core's own shape, which phase-2 detection records.
#[test]
fn the_host_probes_page_counts_through_the_confined_worker() {
    let mut worker = start();
    let properties = worker
        .probe(pdf(), &Limits::defaults())
        .expect("probe the one-page fixture");
    match properties {
        openconvert_core::facts::Properties::Document { pages, encrypted } => {
            assert_eq!(pages, 1);
            assert!(!encrypted);
        }
        other => panic!("expected Document properties, got {other:?}"),
    }
}

/// A refusal arrives as a reason, not a dead pipe.
///
/// The example was `html`, which this engine now writes — `Pdf -> Html` was a
/// declared route the worker refused, and the fix was to implement it. `webp`
/// is a format oc-pdf genuinely does not produce, so this still exercises what
/// the test is about: the shape a refusal travels in.
#[test]
fn an_unwritable_format_refuses_with_a_reason() {
    let mut worker = start();
    let err = worker
        .run(pdf(), "webp", &Limits::defaults(), &[])
        .expect_err("oc-pdf writes no WebP");
    match err {
        WorkerError::Engine {
            engine,
            message,
            retryable,
        } => {
            assert_eq!(engine, "oc-pdf", "the HOST names the engine");
            assert!(message.contains("webp"), "{message}");
            assert!(!retryable);
        }
        other => panic!("expected a structured refusal, got {other}"),
    }
}
