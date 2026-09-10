//! Drive the real `oc-pdf` binary as a real subprocess.
//!
//! Same discipline as `oc-audio/tests/end_to_end.rs`: the protocol is
//! exercised over raw pipes against the built program, because a closed enum
//! of programs is only meaningful if the programs run.
//!
//! Platform gate: pdfium is linked on Windows builds only today; the stub
//! engine elsewhere would turn every assertion here into a false failure.

#![cfg(windows)]

use openconvert_core::format::FormatId;
use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};
use std::io::Write;
use std::process::{Command, Stdio};

/// A minimal but fully valid one-page PDF, built here so the test needs no
/// fixture.
///
/// The xref table is computed from real offsets rather than hand-written:
/// pdfium will *repair* a broken one silently, and then this fixture would
/// prove nothing about honest files — a document only pdfium can open is not
/// a control for the happy path.
fn minimal_pdf() -> Vec<u8> {
    one_page_pdf("0 0 144 72")
}

/// The same document with an arbitrary MediaBox, for tests that need pages
/// bigger than the dimension cap.
fn one_page_pdf(mediabox: &str) -> Vec<u8> {
    let mut out = Vec::from(b"%PDF-1.4\n");
    let mut offsets = [0u32; 4];
    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [{mediabox}] >>"),
    ];
    for (i, body) in bodies.iter().enumerate() {
        let num = i + 1;
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

fn worker_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current exe");
    p.pop(); // deps/
    p.pop(); // target/<profile>/
    p.join(format!("oc-pdf{}", std::env::consts::EXE_SUFFIX))
}

fn talk(requests: &[(Request, Vec<u8>)]) -> Vec<(Response, Vec<u8>)> {
    let exe = worker_path();
    assert!(
        exe.exists(),
        "oc-pdf is not built at {} -- this test needs the binary",
        exe.display()
    );
    let mut child = Command::new(&exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("spawn oc-pdf");
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
        "oc-pdf exited with {:?}",
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

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = b"\xff\xd8\xff";

/// Hello names the engine; probe reads the page count from the container;
/// a Run produces PNG bytes that decode to the page's own dimensions.
#[test]
fn hello_probe_and_run_answer_from_a_one_page_pdf() {
    let src = minimal_pdf();
    let answers = talk(&[
        req(Request::Hello, &[]),
        req(
            Request::Probe {
                input_len: src.len() as u64,
            },
            &src,
        ),
        req(
            Request::Run {
                input_len: src.len() as u64,
                to: FormatId::Png.name().to_string(),
                limits: lim(1 << 24),
                params: Vec::new(),
                model_lens: Vec::new(),
                extra_lens: Vec::new(),
            },
            &src,
        ),
    ]);
    let Response::Ready { engine, .. } = &answers[0].0 else {
        panic!("expected Ready, got {:?}", answers[0].0);
    };
    assert_eq!(engine, "oc-pdf");

    let Response::Properties(openconvert_worker::Probed::Document { pages, encrypted }) =
        &answers[1].0
    else {
        panic!("expected Document Properties, got {:?}", answers[1].0);
    };
    assert_eq!(*pages, 1);
    assert!(!*encrypted);

    let (Response::Done { removed, .. }, png_bytes) = &answers[2] else {
        panic!("expected Done, got {:?}", answers[2].0);
    };
    assert!(png_bytes.starts_with(PNG_MAGIC), "output is not PNG");
    assert!(
        removed.iter().any(|r| r.contains("metadata")),
        "PDF metadata loss must be disclosed: {removed:?}"
    );

    // The raster is not just any PNG: it decodes, and to the dimensions the
    // MediaBox asked for (144x72 points at one-to-one scale).
    let img = image::load_from_memory(png_bytes).expect("decode our own output");
    assert_eq!((img.width(), img.height()), (144, 72));
}

/// Garbage produces a structured failure, never a crash.
#[test]
fn malformed_input_produces_a_failure_not_a_crash() {
    let junk = b"%PDF-1.7 this looks like it started well and then fell apart";
    let answers = talk(&[req(
        Request::Run {
            input_len: junk.len() as u64,
            to: FormatId::Png.name().to_string(),
            limits: lim(1 << 20),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        junk,
    )]);
    let Response::Failed {
        message, retryable, ..
    } = &answers[0].0
    else {
        panic!("expected a structured failure, got {:?}", answers[0].0);
    };
    assert!(message.contains("could not open PDF"), "{message}");
    assert!(!*retryable, "retrying cannot repair the file");
}

/// An output this engine cannot produce is refused BY NAME and not retryable —
/// a route the table must not advertise, and a user who reaches the worker
/// anyway gets told exactly why.
///
/// The example used to be `html`, which this engine now writes. That is the
/// direction the fix went: `Pdf -> Html` WAS declared in the route table and
/// refused here, and the answer was to implement it rather than to withdraw
/// the row. `every_declared_document_route_runs` holds the other half — that
/// nothing the table offers lands in this refusal.
#[test]
fn unsupported_output_format_refuses_clearly_and_non_retryably() {
    let src = minimal_pdf();
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: "webp".into(),
            limits: lim(1 << 20),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    let Response::Failed {
        message, retryable, ..
    } = &answers[0].0
    else {
        panic!("expected Failed, got {:?}", answers[0].0);
    };
    assert!(
        message.contains("webp"),
        "the refusal should name the format: {message}"
    );
    assert!(!*retryable, "retrying will not teach oc-pdf WebP");
}

/// The JPEG arm encodes too, at the page's own dimensions — the route table
/// advertises `Pdf -> Jpeg`, so it is held end to end like the PNG pair.
#[test]
fn a_rendered_page_has_the_white_background_a_viewer_supplies() {
    // THE BUG THIS EXISTS FOR: `FPDFBitmap_CreateEx` hands back a ZEROED
    // buffer, and in BGRA that is transparent BLACK. A PDF page carries no
    // background of its own -- white is a viewer convention -- so without an
    // explicit fill, pdfium painted black glyphs onto black and every
    // conversion produced a blank image under a clean class-B receipt.
    //
    // It survived because the tests either side of this one assert
    // DIMENSIONS. A blank page is exactly the right size. So this asserts
    // what a reader would actually notice: that the page is mostly light,
    // and that it is not one flat colour.
    let src = minimal_pdf();
    for to in [FormatId::Png, FormatId::Jpeg] {
        let answers = talk(&[req(
            Request::Run {
                input_len: src.len() as u64,
                to: to.name().to_string(),
                limits: lim(1 << 24),
                params: Vec::new(),
                model_lens: Vec::new(),
                extra_lens: Vec::new(),
            },
            &src,
        )]);
        let (Response::Done { .. }, bytes) = &answers[0] else {
            panic!("expected Done for {}, got {:?}", to.name(), answers[0].0);
        };
        let img = image::load_from_memory(bytes)
            .unwrap_or_else(|e| panic!("decode our own {} output: {e}", to.name()))
            .to_rgba8();

        // Opaque everywhere: the fill covers the whole bitmap, so no pixel
        // keeps the transparent black it was allocated with.
        assert!(
            img.pixels().all(|p| p.0[3] == 255),
            "{}: the render left transparent pixels, so the background was never painted",
            to.name()
        );

        // And light: an empty MediaBox renders as a white page. The old
        // behaviour scored 0 here.
        let total = u64::from(img.width()) * u64::from(img.height());
        let light = img
            .pixels()
            .filter(|p| u16::from(p.0[0]) + u16::from(p.0[1]) + u16::from(p.0[2]) > 600)
            .count() as u64;
        assert!(
            light * 2 > total,
            "{}: only {light} of {total} pixels are light -- the page rendered dark,              which is what an unpainted background looks like",
            to.name()
        );
    }
}

#[test]
fn jpeg_output_encodes_the_rendered_page() {
    let src = minimal_pdf();
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: FormatId::Jpeg.name().to_string(),
            limits: lim(1 << 24),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    let (Response::Done { .. }, jpeg_bytes) = &answers[0] else {
        panic!("expected Done, got {:?}", answers[0].0);
    };
    assert!(jpeg_bytes.starts_with(JPEG_MAGIC), "output is not JPEG");
    let img = image::load_from_memory(jpeg_bytes).expect("decode our own output");
    assert_eq!((img.width(), img.height()), (144, 72));
}

/// **The dimension cap, measured.** A MediaBox of 8192x4096 points exceeds
/// the 4096-per-axis ceiling on the long side; proportional scaling must land
/// on exactly 4096x2048 BEFORE the bitmap is allocated, which is what bounds
/// a hostile MediaBox's memory ask.
#[test]
fn oversized_pages_scale_down_before_allocation() {
    let src = one_page_pdf("0 0 8192 4096");
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: FormatId::Png.name().to_string(),
            limits: lim(64 << 20),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    let (Response::Done { .. }, png_bytes) = &answers[0] else {
        panic!("expected Done, got {:?}", answers[0].0);
    };
    let img = image::load_from_memory(png_bytes).expect("decode our own output");
    assert_eq!(
        (img.width(), img.height()),
        (4096, 2048),
        "the cap must scale both axes by one factor"
    );
}

/// **A limit failure is retryable, and says so.** The worker refuses pages
/// over `decode_pixels` with a message containing "limit", which is the word
/// the runtime reads to mark the failure retryable rather than fatal.
#[test]
fn a_pixel_budget_smaller_than_the_page_refuses_retryably() {
    let src = minimal_pdf();
    let limits = RunLimits {
        decode_pixels: 1, // 144x72 = 10 368 pixels cannot fit
        memory_bytes: 1 << 24,
        archive_depth: 32,
        archive_entries: 100_000,
        archive_total_bytes: 1 << 30,
        use_gpu: true,
    };
    let answers = talk(&[req(
        Request::Run {
            input_len: src.len() as u64,
            to: FormatId::Png.name().to_string(),
            limits,
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        &src,
    )]);
    let Response::Failed {
        message, retryable, ..
    } = &answers[0].0
    else {
        panic!("expected Failed, got {:?}", answers[0].0);
    };
    assert!(message.contains("limit"), "{message}");
    assert!(message.contains("pixels"), "{message}");
    assert!(
        *retryable,
        "a limit is not a crash -- the host may raise it"
    );
}

/// Bytes without even the `%PDF` magic are refused through the same
/// structured path as truncated documents — pdfium never sees a crash out of
/// them, and nothing hangs waiting for bytes that will not come.
#[test]
fn input_without_pdf_magic_refuses_rather_than_guessing() {
    let junk = b"GIF89a\x01\x00\x01\x00\x00\xff\x00,";
    let answers = talk(&[req(
        Request::Run {
            input_len: junk.len() as u64,
            to: FormatId::Png.name().to_string(),
            limits: lim(1 << 20),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        junk,
    )]);
    let Response::Failed {
        message, retryable, ..
    } = &answers[0].0
    else {
        panic!("expected a structured failure, got {:?}", answers[0].0);
    };
    assert!(message.contains("could not open PDF"), "{message}");
    assert!(!*retryable, "retrying cannot turn a GIF into a PDF");
}
