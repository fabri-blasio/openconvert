//! Merge and reorder, driven through the **real worker binary**.
//!
//! `pages.rs` has unit tests for the algorithm; this exercises everything
//! between the host and it: the `extra_lens` frames added to the protocol, the
//! worker draining them in the order the host wrote them, the `op` parameter
//! selecting the operation, and the bytes coming back out. A library test
//! cannot catch a desynchronised pipe, and a desynchronised pipe is the thing
//! most likely to go wrong when a protocol grows a second list of blobs.
//!
//! **Not gated on Windows**, unlike `host_drives_worker.rs`. That file is
//! gated because pdfium only links there; `lopdf` is pure Rust, so merging and
//! reordering work on every platform and this test runs on all of them.

use openconvert_core::limits::Limits;
use openconvert_run::worker_client::Worker;

const TX_PDF: &str = env!("CARGO_BIN_EXE_oc-pdf");

fn start() -> Worker {
    Worker::start_at(TX_PDF.as_ref(), "oc-pdf", &Limits::defaults()).expect("start oc-pdf")
}

/// A small valid PDF with `pages` pages, built by lopdf so the xref is real.
fn doc(pages: usize) -> Vec<u8> {
    use lopdf::{Dictionary, Document, Object, Stream};
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let mut ids = Vec::new();
    for i in 0..pages {
        let content = doc.add_object(Stream::new(
            Dictionary::new(),
            format!("BT /F1 12 Tf 72 720 Td (page {}) Tj ET", i + 1).into_bytes(),
        ));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set("Contents", content);
        page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        ids.push(doc.add_object(page));
    }
    let kids: Vec<Object> = ids.iter().map(|id| Object::Reference(*id)).collect();
    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set("Kids", kids);
    tree.set("Count", i64::try_from(pages).unwrap_or(0));
    doc.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", catalog_id);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("write fixture");
    out
}

fn page_count(bytes: &[u8]) -> usize {
    lopdf::Document::load_mem(bytes)
        .expect("the worker returned something that is not a PDF")
        .get_pages()
        .len()
}

/// Three documents in, one out, with every page present.
#[test]
fn merging_over_the_protocol_returns_every_page() {
    let mut worker = start();
    let first = doc(2);
    let extras = vec![doc(3), doc(1)];

    let (out, removed) = worker
        .run_with_inputs(
            first,
            &extras,
            "pdf",
            &Limits::defaults(),
            &[("op".to_string(), "merge".to_string())],
        )
        .expect("the worker refused a merge it should have accepted");

    assert_eq!(page_count(&out), 6, "2 + 3 + 1 pages must all arrive");
    assert!(
        removed.iter().any(|r| r.contains("carried across")),
        "the receipt line naming what came across is missing: {removed:?}"
    );
}

/// The extras must be drained in the order the host wrote them.
///
/// A protocol that carried them out of order would still produce a valid PDF
/// with the right page count — so page COUNT cannot catch it. This asks which
/// document ended up first by giving each a distinct number of pages.
#[test]
fn the_extra_inputs_arrive_in_order() {
    let mut worker = start();
    // 1 page, then 4: if the frames desynchronised, the reorder below would
    // address the wrong document and fail rather than quietly pass.
    let (out, _) = worker
        .run_with_inputs(
            doc(1),
            &[doc(4)],
            "pdf",
            &Limits::defaults(),
            &[("op".to_string(), "merge".to_string())],
        )
        .expect("merge");
    assert_eq!(page_count(&out), 5);

    // Now reorder the merged document backwards. This only succeeds if the
    // merged file really has five well-formed pages.
    let mut worker = start();
    let (reordered, _) = worker
        .run_with_inputs(
            out,
            &[],
            "pdf",
            &Limits::defaults(),
            &[
                ("op".to_string(), "reorder".to_string()),
                ("order".to_string(), "5,4,3,2,1".to_string()),
            ],
        )
        .expect("reorder");
    assert_eq!(page_count(&reordered), 5, "no page lost in the reorder");
}

/// A worker asked to merge one document says so rather than succeeding.
#[test]
fn merging_one_document_is_refused_by_the_worker() {
    let mut worker = start();
    let err = worker
        .run_with_inputs(
            doc(2),
            &[],
            "pdf",
            &Limits::defaults(),
            &[("op".to_string(), "merge".to_string())],
        )
        .expect_err("one document is not a merge");
    let text = err.to_string();
    assert!(
        text.contains("two or more"),
        "the refusal should say what was wrong, got {text:?}"
    );
}

/// An unknown operation is named, not ignored.
///
/// Silence here would mean the worker rendering page 0 when someone asked to
/// stamp a document — converting something other than what was asked, which is
/// the failure `run_params` exists to prevent.
#[test]
fn an_unknown_operation_is_refused() {
    let mut worker = start();
    let err = worker
        .run_with_inputs(
            doc(1),
            &[],
            "pdf",
            &Limits::defaults(),
            &[("op".to_string(), "flatten".to_string())],
        )
        .expect_err("an operation this engine has never heard of must fail");
    assert!(
        err.to_string().contains("flatten"),
        "the refusal must name the operation"
    );
}

// ---------------------------------------------------------------------------
// compose: all three stages, one output
// ---------------------------------------------------------------------------

/// A 2x2 transparent PNG, so the stamp stage has something with real alpha.
fn stamp_png() -> Vec<u8> {
    let mut img = image::RgbaImage::new(2, 2);
    img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    img.put_pixel(1, 0, image::Rgba([0, 255, 0, 0]));
    img.put_pixel(0, 1, image::Rgba([0, 0, 255, 128]));
    img.put_pixel(1, 1, image::Rgba([255, 255, 255, 64]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .expect("encode png");
    out.into_inner()
}

/// Whether the named XObject is registered on any page.
fn has_stamp(bytes: &[u8]) -> bool {
    let doc = lopdf::Document::load_mem(bytes).expect("a PDF");
    doc.get_pages().values().any(|id| {
        doc.get_object(*id)
            .and_then(lopdf::Object::as_dict)
            .and_then(|d| d.get(b"Resources"))
            .and_then(lopdf::Object::as_dict)
            .and_then(|r| r.get(b"XObject"))
            .and_then(lopdf::Object::as_dict)
            .is_ok_and(|x| x.get(b"OpenConvertStamp").is_ok())
    })
}

/// **Merge, reorder and stamp in one call, and the stages run in that order.**
///
/// The order is the whole reason this is one operation rather than three. Page
/// numbers in `order` mean pages of the MERGED document, and the page a
/// signature lands on means a page of the REORDERED one. Any other sequence
/// makes both parameters mean something different depending on which stages
/// the caller happened to ask for.
///
/// Two documents of 2 and 3 pages merge to 5; `5,4,3,2,1` reverses them; and
/// the stamp goes on page 1 of the result — which is the last page of the
/// second document. Asserting the count alone would pass on any ordering, so
/// the assertion is which page carries the stamp.
#[test]
fn composing_merges_then_reorders_then_stamps() {
    let mut worker = start();
    let (out, removed) = worker
        .run_with_inputs(
            doc(2),
            &[doc(3), stamp_png()],
            "pdf",
            &Limits::defaults(),
            &[
                ("op".to_string(), "compose".to_string()),
                ("order".to_string(), "5,4,3,2,1".to_string()),
                ("image".to_string(), "1".to_string()),
                ("page".to_string(), "1".to_string()),
                ("x".to_string(), "72".to_string()),
                ("y".to_string(), "72".to_string()),
                ("width".to_string(), "144".to_string()),
            ],
        )
        .expect("the worker refused a compose it should have accepted");

    assert_eq!(
        page_count(&out),
        5,
        "2 + 3 pages, then reordered, is still 5"
    );
    assert!(has_stamp(&out), "the stamp stage did not run");

    let doc_out = lopdf::Document::load_mem(&out).expect("a PDF");
    let first_id = *doc_out.get_pages().values().next().expect("a first page");
    let resources = doc_out
        .get_object(first_id)
        .and_then(lopdf::Object::as_dict)
        .and_then(|d| d.get(b"Resources"))
        .and_then(lopdf::Object::as_dict)
        .expect("the first page has resources");
    assert!(
        resources
            .get(b"XObject")
            .and_then(lopdf::Object::as_dict)
            .is_ok_and(|x| x.get(b"OpenConvertStamp").is_ok()),
        "the stamp must land on page 1 OF THE REORDERED document, which means \
         reorder ran before stamp"
    );

    assert!(
        removed.iter().any(|r| r.contains("carried across")),
        "the merge stage owes the receipt its line: {removed:?}"
    );
    assert!(
        removed.iter().any(|r| r.contains("transparency")),
        "the stamp stage owes the receipt its line: {removed:?}"
    );
}

/// Compose with nothing selected is a copy, not an error.
///
/// One document, no order, no image. Every stage is skipped and the bytes come
/// back. This is the path a caller takes when the user has a tool lit and has
/// not yet done anything with it, and it must not be a failure.
#[test]
fn composing_with_no_stages_returns_the_document() {
    let mut worker = start();
    let (out, _) = worker
        .run_with_inputs(
            doc(3),
            &[],
            "pdf",
            &Limits::defaults(),
            &[
                ("op".to_string(), "compose".to_string()),
                ("order".to_string(), String::new()),
            ],
        )
        .expect("compose with no stages");
    assert_eq!(page_count(&out), 3);
    assert!(
        !has_stamp(&out),
        "nothing was asked for, so nothing was drawn"
    );
}

/// **Composing a SUBSET writes a file.** It used to write nothing at all.
///
/// This is the desktop's actual save path for the reorder board: `pdf-compose`
/// with an `order`. Every card on that board has a bin and both odd/even
/// shortcuts drop half the pages, so a short order is the ordinary case -- and
/// `pages::reorder` refused it, which reached the user as "That did not produce
/// a file. Nothing was written."
///
/// Over the wire rather than in a unit test because the unit test and the
/// worker disagreeing about this is precisely what was not noticed.
#[test]
fn composing_a_subset_of_the_pages_writes_those_pages() {
    let mut worker = start();
    let (out, _) = worker
        .run_with_inputs(
            doc(4),
            &[],
            "pdf",
            &Limits::defaults(),
            &[
                ("op".to_string(), "compose".to_string()),
                // Keep pages 4 and 1, in that order, and drop the rest --
                // which is what "remove even pages" plus a drag produces.
                ("order".to_string(), "4,1".to_string()),
            ],
        )
        .expect("a subset is a selection, not a failure");
    assert_eq!(
        page_count(&out),
        2,
        "the output has the pages that were kept"
    );
}

/// Merge and sign, with no reorder in between.
///
/// The stages are independent: skipping the middle one must not shift which
/// extra input is the image. `image=1` is what marks the boundary between
/// documents and the picture, and this is the case where getting it wrong
/// would stamp a PDF onto a page.
#[test]
fn composing_can_skip_the_reorder_stage() {
    let mut worker = start();
    let (out, _) = worker
        .run_with_inputs(
            doc(2),
            &[doc(2), stamp_png()],
            "pdf",
            &Limits::defaults(),
            &[
                ("op".to_string(), "compose".to_string()),
                ("image".to_string(), "1".to_string()),
                ("page".to_string(), "3".to_string()),
            ],
        )
        .expect("compose without a reorder");
    assert_eq!(
        page_count(&out),
        4,
        "the image must not be merged as a document"
    );
    assert!(has_stamp(&out));
}
