//! `pdf -> txt` and `pdf -> docx`, over the wire, through the real worker.
//!
//! The layout reconstruction has unit tests that feed it hand-built glyph
//! boxes; those prove the RULES. This proves the other half: that pdfium hands
//! back what the rules expect from an actual PDF, that the worker answers on
//! the protocol, and that the DOCX it produces is a package a reader accepts.
//!
//! The fixture is generated rather than committed, and it draws its text with
//! a real content stream and a real font resource, because a PDF with no text
//! objects is exactly the case this feature must REFUSE — a fixture that
//! accidentally had none would make the happy-path tests pass vacuously.

#![cfg(windows)]

use std::io::Write;
use std::process::{Command, Stdio};

use openconvert_sandbox::protocol::{read_content, read_frame, write_content, write_frame};
use openconvert_worker::{Request, Response, RunLimits};

/// One page of text, drawn at the given (size, x, y, string) positions.
///
/// Offsets are computed from the bytes actually written. pdfium repairs a
/// broken xref silently, so a hand-written one would let this fixture prove
/// nothing about honest files.
fn text_pdf(lines: &[(f32, f32, f32, &str)]) -> Vec<u8> {
    let mut content = String::from("BT\n");
    for (size, x, y, s) in lines {
        // Escape the two characters a PDF literal string cannot carry raw.
        let escaped = s
            .replace('\\', r"\\")
            .replace('(', r"\(")
            .replace(')', r"\)");
        content.push_str(&format!(
            "/F1 {size} Tf\n1 0 0 1 {x} {y} Tm\n({escaped}) Tj\n"
        ));
    }
    content.push_str("ET\n");

    let bodies = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];

    let mut out = Vec::from(b"%PDF-1.4\n");
    let mut offsets = vec![0u32; bodies.len() + 1];
    for (i, body) in bodies.iter().enumerate() {
        let num = i + 1;
        offsets[num] = out.len() as u32;
        out.extend_from_slice(format!("{num} 0 obj\n{body}\nendobj\n").as_bytes());
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for slot in &offsets[1..] {
        out.extend_from_slice(format!("{slot:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out.extend_from_slice(format!("{xref_at}\n%%EOF\n").as_bytes());
    out
}

/// A page with a heading and two paragraphs, laid out the way a word processor
/// would: full-width lines inside a paragraph, a short line at the end of one.
fn article() -> Vec<u8> {
    text_pdf(&[
        (18.0, 72.0, 720.0, "Chapter One"),
        (
            10.0,
            72.0,
            680.0,
            "The quick brown fox jumps over the lazy dog and",
        ),
        (
            10.0,
            72.0,
            668.0,
            "then keeps running through the long grass until",
        ),
        (10.0, 72.0, 656.0, "it is done."),
        (
            10.0,
            72.0,
            632.0,
            "A second paragraph begins here and runs on for",
        ),
        (10.0, 72.0, 620.0, "another line before it finally stops."),
    ])
}

/// The same document with `count` identical pages, for the page-index tests.
///
/// Built separately from `text_pdf` because the page tree has to carry several
/// kids: a one-page fixture cannot show an off-by-one at either end, which is
/// exactly how one survived.
fn multi_page_pdf(count: usize) -> Vec<u8> {
    let kids: String = (0..count)
        .map(|i| format!("{} 0 R", 3 + 2 * i))
        .collect::<Vec<_>>()
        .join(" ");
    let font_num = 3 + 2 * count;

    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {count} >>"),
    ];
    for i in 0..count {
        // Each page says its own number, so a render can be told from its
        // neighbours by looking at it.
        let content = format!(
            "BT
/F1 36 Tf
1 0 0 1 72 700 Tm
(Page {}) Tj
ET
",
            i + 1
        );
        bodies.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font_num} 0 R >> >> /Contents {} 0 R >>",
            4 + 2 * i
        ));
        bodies.push(format!(
            "<< /Length {} >>
stream
{content}endstream",
            content.len()
        ));
    }
    bodies.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

    let mut out = Vec::from(
        b"%PDF-1.4
",
    );
    let mut offsets = vec![0u32; bodies.len() + 1];
    for (i, body) in bodies.iter().enumerate() {
        let num = i + 1;
        offsets[num] = out.len() as u32;
        out.extend_from_slice(
            format!(
                "{num} 0 obj
{body}
endobj
"
            )
            .as_bytes(),
        );
    }
    let xref_at = out.len();
    out.extend_from_slice(
        format!(
            "xref
0 {}
0000000000 65535 f 
",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    for slot in &offsets[1..] {
        out.extend_from_slice(
            format!(
                "{slot:010} 00000 n 
"
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!(
            "trailer
<< /Size {} /Root 1 0 R >>
startxref
",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    out.extend_from_slice(
        format!(
            "{xref_at}
%%EOF
"
        )
        .as_bytes(),
    );
    out
}

/// Render one page, returning the response.
fn render(input: &[u8], page: u32) -> Response {
    talk(&[(
        Request::Run {
            input_len: input.len() as u64,
            to: "png".to_string(),
            limits: lim(),
            params: vec![("page".to_string(), page.to_string())],
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        input.to_vec(),
    )])
    .into_iter()
    .next()
    .expect("one answer")
    .0
}

/// **`page` IS ZERO-BASED, AND BOTH ENDS PROVE IT.**
///
/// This is the contract the desktop's `render_preview` sends against, and it
/// forwarded a one-based number for the whole life of the preview path. The
/// three symptoms were: a one-page document refused outright (`asked for page 1
/// (zero-based)`, which is what the bug report quoted), every "page 1" preview
/// of a longer document silently rendering page TWO, and the last page of any
/// document always failing.
///
/// Asserting only the middle of the range would have passed throughout. The
/// ends are the whole test: index 0 must render, index `count - 1` must render,
/// and index `count` must not.
#[test]
fn the_page_parameter_is_zero_based_at_both_ends() {
    for count in [1_usize, 3] {
        let doc = multi_page_pdf(count);

        let first = render(&doc, 0);
        assert!(
            matches!(first, Response::Done { .. }),
            "{count}-page document: page 0 is the first page and must render, got {first:?}"
        );

        let last = render(&doc, count as u32 - 1);
        assert!(
            matches!(last, Response::Done { .. }),
            "{count}-page document: page {} is the last page and must render, got {last:?}",
            count - 1
        );

        // One past the end is a refusal, not a render of something else.
        let past = render(&doc, count as u32);
        let Response::Failed { message, .. } = &past else {
            panic!("{count}-page document: page {count} is past the end and must be refused");
        };
        assert!(
            message.contains(&format!("{count} page(s)")),
            "the refusal should name the real page count: {message}"
        );
    }
}

/// The control: consecutive indices render DIFFERENT pages.
///
/// Without this, an engine that ignored the parameter entirely would satisfy
/// the test above — and "every page renders page one" is a shape this project
/// has already shipped once, on the reorder board.
#[test]
fn consecutive_page_indices_render_different_pages() {
    let doc = multi_page_pdf(3);
    let pages: Vec<Vec<u8>> = (0..3)
        .map(|i| {
            let answers = talk(&[(
                Request::Run {
                    input_len: doc.len() as u64,
                    to: "png".to_string(),
                    limits: lim(),
                    params: vec![("page".to_string(), i.to_string())],
                    model_lens: Vec::new(),
                    extra_lens: Vec::new(),
                },
                doc.clone(),
            )]);
            answers.into_iter().next().expect("one answer").1
        })
        .collect();

    assert_ne!(
        pages[0], pages[1],
        "page 0 and page 1 rendered the same bytes"
    );
    assert_ne!(
        pages[1], pages[2],
        "page 1 and page 2 rendered the same bytes"
    );
}

/// A structurally valid PDF with no text objects at all — a scan, in effect.
fn scan() -> Vec<u8> {
    text_pdf(&[])
}

fn worker_path() -> std::path::PathBuf {
    let mut p = std::env::current_exe().expect("current exe");
    p.pop();
    p.pop();
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
            write_frame(&mut stdin, &serde_json::to_vec(r).unwrap()).unwrap();
            write_content(&mut stdin, content).unwrap();
        }
        stdin.flush().unwrap();
    }
    let out = child.wait_with_output().expect("wait");
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

fn lim() -> RunLimits {
    RunLimits {
        decode_pixels: u64::MAX,
        memory_bytes: 64 << 20,
        archive_depth: 32,
        archive_entries: 100_000,
        archive_total_bytes: 1 << 30,
        use_gpu: true,
    }
}

fn run(input: &[u8], to: &str) -> (Response, Vec<u8>) {
    let answers = talk(&[(
        Request::Run {
            input_len: input.len() as u64,
            to: to.to_string(),
            limits: lim(),
            params: Vec::new(),
            model_lens: Vec::new(),
            extra_lens: Vec::new(),
        },
        input.to_vec(),
    )]);
    answers.into_iter().next().expect("one answer")
}

/// **The words come out, in reading order, as paragraphs.**
///
/// Not "some text appears": the two paragraphs must be two, the wrapped lines
/// inside each must be joined, and the sentence must read the way the page
/// reads. A PDF extractor that returns the right characters in the wrong order
/// has failed at the only thing anyone wanted from it.
#[test]
fn a_pdf_yields_its_own_text_as_paragraphs() {
    let (response, out) = run(&article(), "txt");
    let Response::Done { .. } = response else {
        panic!("expected Done, got {response:?}");
    };
    let text = String::from_utf8(out).expect("UTF-8");

    assert!(
        text.contains("The quick brown fox jumps over the lazy dog and then keeps running"),
        "wrapped lines were not joined:\n{text}"
    );
    assert!(text.starts_with("Chapter One"), "heading first:\n{text}");
    assert!(
        text.contains("\n\nA second paragraph begins here"),
        "the short line did not end the paragraph:\n{text}"
    );
    // Three paragraphs: the heading and the two blocks of body text.
    assert_eq!(
        text.split("\n\n").filter(|s| !s.trim().is_empty()).count(),
        3,
        "got:\n{text}"
    );
}

/// **The Word document opens, and carries the same words and the heading.**
#[test]
fn a_pdf_becomes_a_readable_word_document() {
    let (response, out) = run(&article(), "docx");
    let Response::Done { removed, .. } = &response else {
        panic!("expected Done, got {response:?}");
    };

    // It is a ZIP that identifies as a Word document — which is what `sniff`
    // will decide about our own output the next time it sees it.
    assert_eq!(&out[..4], &[0x50, 0x4B, 0x03, 0x04]);
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&out)).expect("a readable ZIP");
    assert!(zip.by_name("word/document.xml").is_ok());

    let xml = {
        use std::io::Read as _;
        let mut part = zip.by_name("word/document.xml").expect("the document part");
        let mut s = String::new();
        part.read_to_string(&mut s).expect("UTF-8");
        s
    };
    assert!(xml.contains("Chapter One"), "{xml}");
    assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#), "{xml}");
    assert!(
        xml.contains("The quick brown fox jumps over the lazy dog and then keeps running"),
        "{xml}"
    );

    // **The inference is disclosed.** This is the Class C row in the table,
    // and the receipt has to say which part of the output was concluded rather
    // than read.
    assert!(
        removed.iter().any(|r| r.contains("INFERRED")),
        "the receipt does not disclose that the structure was inferred: {removed:?}"
    );
}

/// **A scanned PDF is refused by name, not written out empty.**
///
/// The failure this prevents is silent: a zero-byte `.txt` or an empty Word
/// document is a successful-looking conversion that has thrown the user's
/// document away. The message has to name what happened and what would work.
#[test]
fn a_pdf_with_no_text_refuses_and_says_why() {
    for to in ["txt", "docx"] {
        let (response, _) = run(&scan(), to);
        let Response::Failed { message, .. } = &response else {
            panic!("{to}: expected a refusal, got {response:?}");
        };
        assert!(
            message.contains("no text in it"),
            "{to}: the message does not name the cause: {message}"
        );
        assert!(
            message.contains("recognised") || message.contains("recognition"),
            "{to}: the message does not name what would work: {message}"
        );
    }
}

/// Text extraction reports what it dropped, the same as every other route.
#[test]
fn the_receipt_names_what_the_text_left_behind() {
    let (response, _) = run(&article(), "txt");
    let Response::Done { removed, .. } = &response else {
        panic!("expected Done, got {response:?}");
    };
    assert!(
        removed.iter().any(|r| r.contains("fonts")),
        "layout loss is not disclosed: {removed:?}"
    );
    assert!(
        removed.iter().any(|r| r.contains("metadata")),
        "metadata loss is not disclosed: {removed:?}"
    );
}
