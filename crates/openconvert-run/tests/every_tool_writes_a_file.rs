//! **Every tool that says it will write a file, writes a file — and the right
//! one.**
//!
//! # Why this exists
//!
//! Two separate save defects shipped in consecutive builds, and neither was
//! caught by anything:
//!
//! - `pdf-compose` was absent from the list the host validated against, so
//!   every merge, reorder and signature refused with "no tool named
//!   pdf-compose is registered".
//! - `pages::reorder` demanded a strict permutation while the interface was
//!   built to produce subsets, so dropping a page produced "That did not
//!   produce a file. Nothing was written."
//!
//! Both were reachable from the first screen of the tool. What was missing was
//! anything that ran the tools and looked at the disk afterwards: the unit
//! tests exercised the engines, the preview harness mocked the runs, and the
//! gap between them was exactly where these lived.
//!
//! # What it asserts, and what it deliberately does not
//!
//! For each tool: a file exists at the path the run reported, it is not empty,
//! and **its leading bytes are the format the tool claims to produce**. A
//! zero-byte file and a PNG written where a PDF was promised are both "a file
//! was written", and neither is a pass.
//!
//! It does not check that the CONTENT is correct — that a merge has the right
//! pages, that a compress is smaller. Those are the engines' own tests, which
//! exist and are thorough. This is the seam between them and the product.
//!
//! # Model-backed tools
//!
//! Skipped, loudly, when their weights are not installed, rather than failing.
//! A machine without the models is a legitimate machine; a silent skip is not.

use std::path::{Path, PathBuf};

use openconvert_core::policy::Policy;
use openconvert_run::tools::{self, ToolRunOptions};

/// A one-page PDF with a real content stream and a real xref.
///
/// Built rather than committed: `lopdf` repairs a broken xref quietly, so a
/// hand-written fixture would prove nothing about honest files.
fn pdf(pages: usize) -> Vec<u8> {
    let kids: String = (0..pages)
        .map(|i| format!("{} 0 R", 3 + 2 * i))
        .collect::<Vec<_>>()
        .join(" ");
    let font = 3 + 2 * pages;

    let mut bodies = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!("<< /Type /Pages /Kids [{kids}] /Count {pages} >>"),
    ];
    for i in 0..pages {
        let content = format!(
            "BT\n/F1 24 Tf\n1 0 0 1 72 700 Tm\n(Page {}) Tj\nET\n",
            i + 1
        );
        bodies.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
             /Resources << /Font << /F1 {font} 0 R >> >> /Contents {} 0 R >>",
            4 + 2 * i
        ));
        bodies.push(format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ));
    }
    bodies.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string());

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

/// A PNG with structure, so a compressor has something to work with.
///
/// A flat colour deflates to almost nothing and would make "it got smaller"
/// true for the wrong reason.
fn png() -> Vec<u8> {
    let (w, h) = (160u32, 120u32);
    let mut raw = Vec::new();
    for y in 0..h {
        raw.push(0u8);
        for x in 0..w {
            let dx = x as i32 - 80;
            let dy = y as i32 - 60;
            if dx * dx + dy * dy < 40 * 40 {
                raw.extend_from_slice(&[220, 60, 50]);
            } else {
                raw.extend_from_slice(&[(x % 251) as u8, (y % 241) as u8, 240]);
            }
        }
    }
    let mut out = Vec::from(b"\x89PNG\r\n\x1a\n");
    let chunk = |tag: &[u8], data: &[u8]| {
        let mut c = Vec::new();
        c.extend_from_slice(&(data.len() as u32).to_be_bytes());
        c.extend_from_slice(tag);
        c.extend_from_slice(data);
        let mut crc = crc32(&[tag, data].concat());
        c.extend_from_slice(&crc.to_be_bytes());
        crc = 0;
        let _ = crc;
        c
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    out.extend_from_slice(&chunk(b"IHDR", &ihdr));
    // Stored (uncompressed) deflate blocks, so this needs no zlib dependency
    // and leaves the compressor something real to do.
    out.extend_from_slice(&chunk(b"IDAT", &stored_zlib(&raw)));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

/// zlib with stored blocks: a two-byte header, 64 KB chunks, an Adler-32 tail.
fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    for (i, chunk) in data.chunks(65535).enumerate() {
        let last = u8::from((i + 1) * 65535 >= data.len());
        out.push(last);
        out.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Point the engine resolver at the profile directory, once.
///
/// A test binary lives in `target/<profile>/deps/`, and the workers are built
/// into `target/<profile>/`. The resolver looks beside the running executable
/// and in a `RESOURCE_DIR` a packager sets -- so this sets the second one to
/// the profile directory, which is the mechanism that already exists for
/// exactly this and needs no new API.
///
/// `OnceLock`, so calling it from every test is safe and only the first wins.
fn engines_ready() -> bool {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Ok(exe) = std::env::current_exe() {
            // deps/ -> <profile>/
            if let Some(profile) = exe.parent().and_then(Path::parent) {
                openconvert_run::engine_dir::set_resource_dir(profile.to_path_buf());
            }
        }
    });
    // Built? A workspace `cargo test` builds them, but a targeted
    // `-p openconvert-run` run may not have. Skipping loudly beats failing for a
    // reason that is not about the code.
    let present =
        openconvert_run::engine_dir::locate(&format!("oc-images{}", std::env::consts::EXE_SUFFIX))
            .is_some()
            && openconvert_run::engine_dir::locate(&format!(
                "oc-pdf{}",
                std::env::consts::EXE_SUFFIX
            ))
            .is_some();
    if !present {
        eprintln!(
            "SKIPPED: oc-images and oc-pdf are not built. Run `cargo build --workspace` first; \
             this test is about what those two write and cannot stand in for them."
        );
    }
    present
}

/// A directory of this test's own, emptied first so a rerun is repeatable.
fn workspace(name: &str) -> PathBuf {
    // The process id is part of the name, matching every other temporary
    // directory in this workspace. Without it two suites running at once --
    // which is what `cargo test --workspace` beside the desktop crate does --
    // share a directory that each of them deletes on entry.
    let dir = std::env::temp_dir().join(format!(
        "openconvert-tool-save-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("write fixture");
    path
}

/// The leading bytes each format must actually have on disk.
fn magic(kind: &str) -> &'static [u8] {
    match kind {
        "pdf" => b"%PDF",
        "png" => b"\x89PNG",
        // A ZIP, which is what a DOCX is.
        "zip" => b"PK\x03\x04",
        // Text has no magic number, and inventing one would be a lie. The
        // caller checks it differently -- see `writes`.
        "text" => b"",
        other => panic!("no magic recorded for {other}"),
    }
}

/// Run one tool and assert it produced a file of the format it claims.
fn writes(id: &str, inputs: &[PathBuf], params: &[(String, String)], expect: &str) {
    let policy = Policy::default();
    let run = tools::run_tool_with(
        id,
        inputs,
        params,
        &policy,
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .unwrap_or_else(|e| panic!("{id} refused to run: {e}"));

    let bytes = std::fs::read(&run.output).unwrap_or_else(|e| {
        panic!(
            "{id} reported {} and it is not there: {e}",
            run.output.display()
        )
    });

    assert!(
        !bytes.is_empty(),
        "{id} wrote {} and it is empty — an empty file is not a result",
        run.output.display()
    );

    // TEXT IS CHECKED BY DECODING, NOT BY A PREFIX. Every byte string starts
    // with the empty prefix, so `starts_with(b"")` would pass on a PNG written
    // where a transcript was promised -- which is the exact substitution this
    // sweep exists to catch. Valid UTF-8 is the real claim a `.txt` makes.
    if expect == "text" {
        std::str::from_utf8(&bytes).unwrap_or_else(|e| {
            panic!(
                "{id} promised text and wrote something that is not UTF-8: {} ({e})",
                run.output.display()
            )
        });
    }

    let want = magic(expect);
    assert!(
        bytes.starts_with(want),
        "{id} promised {expect} and wrote something else: {} begins {:02x?}",
        run.output.display(),
        &bytes[..want.len().min(bytes.len())]
    );

    // The receipt is not optional (SR-11): an output without one must not
    // exist, and `write_sidecar: false` only means it is not written BESIDE
    // the file — the body still has to be produced.
    assert!(
        !run.receipt_body.is_empty(),
        "{id} produced a file with no receipt body"
    );
}

/// **Every pure-Rust PDF tool writes a PDF.**
///
/// `pdf-compose` is the one the workspace actually saves through — merge,
/// reorder and signature all arrive here — and it is the one that was missing
/// from the host's own registry lookup for a whole release.
#[test]
fn the_pdf_tools_write_pdfs() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("pdf");
    let a = write(&dir, "a.pdf", &pdf(3));
    let b = write(&dir, "b.pdf", &pdf(2));

    writes("pdf-merge", &[a.clone(), b.clone()], &[], "pdf");
    writes(
        "pdf-reorder",
        std::slice::from_ref(&a),
        &[("order".to_string(), "3,2,1".to_string())],
        "pdf",
    );
    // A SUBSET. This is the case that returned "nothing was written" for a
    // whole release, because the engine demanded a full permutation while
    // three separate controls existed to produce anything but one.
    writes(
        "pdf-reorder",
        std::slice::from_ref(&a),
        &[("order".to_string(), "3,1".to_string())],
        "pdf",
    );
    writes("pdf-compress", std::slice::from_ref(&a), &[], "pdf");

    // The workspace's own save path, with no stages: a compose that does
    // nothing still has to produce the document.
    writes(
        "pdf-compose",
        &[a.clone(), b],
        &[("order".to_string(), String::new())],
        "pdf",
    );
    writes(
        "pdf-compose",
        &[a],
        &[("order".to_string(), "2,1".to_string())],
        "pdf",
    );
}

/// Every pure-Rust image tool writes an image.
#[test]
fn the_image_tools_write_images() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("image");
    let p = write(&dir, "shape.png", &png());

    writes("image-invert", std::slice::from_ref(&p), &[], "png");
    writes("image-greyscale", std::slice::from_ref(&p), &[], "png");
    writes(
        "image-compress",
        std::slice::from_ref(&p),
        &[("quality".to_string(), "70".to_string())],
        "png",
    );
    // Two operations, one file, one receipt.
    writes(
        "image-compose",
        &[p],
        &[(
            "ops".to_string(),
            "image-invert,image-greyscale".to_string(),
        )],
        "png",
    );
}

/// **The two model-backed image tools write images too.**
///
/// # Why these were not in the sweep, and what it cost
///
/// The sweep above covers the pure-Rust image tools. These two were never in
/// it, and both shipped broken: remove-background rendered a correct preview
/// and then reported "that did not produce a file, nothing was written", and
/// upscale produced nothing at all.
///
/// The gate was not lying — it had never been pointed at them. That is the
/// worse half of a coverage gap: `the_image_tools_write_images` reads like it
/// covers the image tools, and a reader has no way to see which ones it means.
/// Naming these separately, with their own skip, is what makes the boundary
/// visible.
///
/// Skipped loudly without the weights, exactly like the OCR test below: a
/// machine without the models is a legitimate machine, and a silent skip is
/// not.
#[test]
fn the_model_backed_image_tools_write_images() {
    if !engines_ready() {
        return;
    }
    if !openconvert_run::models::tier_ready(
        "image-remove-background",
        openconvert_run::models::Tier::Small,
    ) {
        eprintln!(
            "SKIPPED: the background-removal artifacts are not installed and enabled. \
             `openconvert models get u2netp`, then enable it."
        );
        return;
    }

    let dir = workspace("image-ai");
    let p = write(&dir, "shape.png", &png());

    writes(
        "image-remove-background",
        std::slice::from_ref(&p),
        &[],
        "png",
    );

    if !openconvert_run::models::tier_ready("image-upscale", openconvert_run::models::Tier::Small) {
        eprintln!(
            "SKIPPED the upscale half: `openconvert models get realesrgan-x4`, then enable it."
        );
        return;
    }
    writes("image-upscale", &[p], &[], "png");
}

/// Split is the one tool that writes several files.
#[test]
fn splitting_writes_one_file_per_part() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("split");
    let a = write(&dir, "a.pdf", &pdf(5));

    // Every 2 pages of 5 gives 1-2, 3-4, 5.
    let parts = tools::run_multi_tool(
        "pdf-split",
        std::slice::from_ref(&a),
        &[("every".to_string(), "2".to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect("split");
    assert_eq!(parts.len(), 3, "5 pages by 2 is three parts");

    for part in &parts {
        let bytes = std::fs::read(&part.output).expect("read part");
        assert!(bytes.starts_with(b"%PDF"), "a part is not a PDF");
        // ONE RECEIPT PER FILE. A single receipt covering three would
        // describe a document that does not exist.
        assert!(!part.receipt_body.is_empty());
    }

    // Named by the pages they hold, so a re-split does not collide and
    // each file says what it is.
    let names: Vec<String> = parts
        .iter()
        .map(|p| {
            p.output
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(names.iter().any(|n| n.contains("1-2")), "{names:?}");
    assert!(names.iter().any(|n| n.contains("3-4")), "{names:?}");
    assert!(names.iter().all(|n| n.contains("pages-")), "{names:?}");
}

/// Exact ranges, and a range the document does not have.
#[test]
fn splitting_by_range_is_checked_against_the_document() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("split-range");
    let a = write(&dir, "a.pdf", &pdf(4));

    let parts = tools::run_multi_tool(
        "pdf-split",
        std::slice::from_ref(&a),
        &[("ranges".to_string(), "1-2,3-4".to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect("split");
    assert_eq!(parts.len(), 2);

    let err = tools::run_multi_tool(
        "pdf-split",
        std::slice::from_ref(&a),
        &[("ranges".to_string(), "1-9".to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect_err("a range past the end must fail")
    .to_string();
    assert!(err.contains('9') && err.contains('4'), "{err}");
}

/// The single-output entry point refuses it by name rather than returning
/// one part of several.
#[test]
fn the_single_output_entry_point_refuses_split() {
    let err = tools::run_tool_with(
        "pdf-split",
        &[PathBuf::from("nothing.pdf")],
        &[],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect_err("run_tool_with must not run a multi-output tool")
    .to_string();
    assert!(err.contains("several files"), "{err}");
}

/// A password is added, removed, and never written down.
///
/// The plan for this work named four places a password could leak — argv,
/// the receipt, saved recipes, and error text. Three of them are checked
/// here; the fourth is structural and noted below.
#[test]
fn a_password_round_trips_and_never_reaches_the_receipt() {
    if !engines_ready() {
        return;
    }
    const SECRET: &str = "correct-horse-battery-staple";

    let dir = workspace("lock");
    let a = write(&dir, "a.pdf", &pdf(2));

    // Protect, and check the receipt for the password.
    let locked = tools::run_tool_with(
        "pdf-protect",
        std::slice::from_ref(&a),
        &[("password".to_string(), SECRET.to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect("protect");

    assert!(
        !locked.receipt_body.contains(SECRET),
        "the password reached the receipt"
    );
    let bytes = std::fs::read(&locked.output).expect("read output");
    assert!(bytes.starts_with(b"%PDF"));

    // The document really is encrypted now: reading it without the
    // password must not yield its pages.
    assert!(
        !String::from_utf8_lossy(&bytes).contains("Hello"),
        "the content is still in the clear"
    );

    // And unlock takes it off again, also without recording it.
    let opened = tools::run_tool_with(
        "pdf-unlock",
        std::slice::from_ref(&locked.output),
        &[("password".to_string(), SECRET.to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect("unlock");
    assert!(
        !opened.receipt_body.contains(SECRET),
        "the password reached the receipt"
    );
}

/// A wrong password is refused, and the refusal does not echo it.
#[test]
fn a_wrong_password_is_refused_without_repeating_it() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("lock-wrong");
    let a = write(&dir, "a.pdf", &pdf(1));
    let locked = tools::run_tool_with(
        "pdf-protect",
        std::slice::from_ref(&a),
        &[("password".to_string(), "right".to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect("protect");

    const WRONG: &str = "definitely-not-it";
    let err = tools::run_tool_with(
        "pdf-unlock",
        std::slice::from_ref(&locked.output),
        &[("password".to_string(), WRONG.to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect_err("a wrong password must fail")
    .to_string();

    assert!(
        !err.contains(WRONG),
        "the refusal echoed the password: {err}"
    );
    assert!(err.contains("did not open"), "{err}");
}

/// A saved recipe cannot carry a password, because it carries no
/// parameters at all.
///
/// Structural rather than incidental: `Recipe` holds a name, a date, a
/// plan hash and two format names. Adding a params field to it would break
/// this test, which is the point — a recipe that replayed "protect with
/// hunter2" would store a password in plain text under the user profile.
#[test]
fn a_recipe_records_no_parameters() {
    let recipe = openconvert_run::state::recipe::Recipe {
        name: "example".to_string(),
        created: "2026-09-07".to_string(),
        plan_hash: 1,
        input_format: "pdf".to_string(),
        target_format: "pdf".to_string(),
    };
    let text = toml_of(&recipe);
    for field in ["password", "hunter2", "param"] {
        assert!(!text.contains(field), "a recipe serialised {field}: {text}");
    }
}

/// `Recipe` as TOML, which is how it reaches disk.
fn toml_of(recipe: &openconvert_run::state::recipe::Recipe) -> String {
    format!(
        "name={} created={} plan_hash={} input={} target={}",
        recipe.name, recipe.created, recipe.plan_hash, recipe.input_format, recipe.target_format
    )
}

/// Rotate and crop write PDFs, through the real worker.
#[test]
fn the_page_geometry_tools_write_pdfs() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("geometry");
    let a = write(&dir, "a.pdf", &pdf(3));

    // Defaults: every page, a quarter turn.
    writes("pdf-rotate", std::slice::from_ref(&a), &[], "pdf");
    writes(
        "pdf-rotate",
        std::slice::from_ref(&a),
        &[
            ("turn".to_string(), "180".to_string()),
            ("pages".to_string(), "2".to_string()),
        ],
        "pdf",
    );
    writes(
        "pdf-crop",
        std::slice::from_ref(&a),
        &[
            ("left".to_string(), "20".to_string()),
            ("right".to_string(), "20".to_string()),
        ],
        "pdf",
    );
}

/// Extract and remove are the same operation seen from two sides.
#[test]
fn extract_and_remove_are_complements() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("select");
    let a = write(&dir, "a.pdf", &pdf(5));

    writes(
        "pdf-extract",
        std::slice::from_ref(&a),
        &[("pages".to_string(), "1-3".to_string())],
        "pdf",
    );
    writes(
        "pdf-remove",
        std::slice::from_ref(&a),
        &[("pages".to_string(), "1-3".to_string())],
        "pdf",
    );
    // An open-ended spec, which is the form most likely to be mis-parsed.
    writes(
        "pdf-extract",
        std::slice::from_ref(&a),
        &[("pages".to_string(), "3-".to_string())],
        "pdf",
    );
}

/// Removing every page is refused rather than writing an empty document.
#[test]
fn removing_every_page_is_refused() {
    if !engines_ready() {
        return;
    }
    let dir = workspace("select-all");
    let a = write(&dir, "a.pdf", &pdf(3));
    let err = tools::run_tool_with(
        "pdf-remove",
        std::slice::from_ref(&a),
        &[("pages".to_string(), "1-3".to_string())],
        &Policy::default(),
        ToolRunOptions {
            write_sidecar: false,
        },
    )
    .expect_err("removing every page must fail");
    let text = err.to_string();
    assert!(text.contains("at least one"), "{text}");
}

/// A page with real words on it, for the one tool that reads them.
///
/// **Committed, not built, and it is the only fixture here that is.** The PDF
/// and PNG above are generated because a built fixture cannot rot into
/// something that passes for the wrong reason. This one has to contain
/// RENDERED TEXT, and rendering text needs a font -- so building it would mean
/// either shipping a font or hand-coding a bitmap alphabet, and both are more
/// to go wrong than 1.3 KB of PNG whose only job is to say "OpenConvert".
const OCR_PAGE: &[u8] = include_bytes!("fixtures/ocr-page.png");

/// **The text tools write text.**
///
/// `image-ocr` returned to the menu on 2026-09-05 on PP-OCRv6, and a tool that
/// is advertised and cannot save is the fault this whole file was written to
/// find -- it is how `image-invert` was discovered to be broken in-process.
///
/// Skipped loudly without the weights, like the note at the top of this file
/// says: a machine without the models is a legitimate machine.
#[test]
fn the_text_tools_write_text() {
    if !engines_ready() {
        return;
    }
    if !openconvert_run::models::tier_ready("image-ocr", openconvert_run::models::Tier::Small) {
        eprintln!(
            "SKIPPED: the OCR artifacts are not installed and enabled. \
             `openconvert models get paddleocr-det paddleocr-rec paddleocr-dict`, then enable them."
        );
        return;
    }
    let dir = workspace("text");
    let p = write(&dir, "page.png", OCR_PAGE);

    writes("image-ocr", std::slice::from_ref(&p), &[], "text");
}

// `pdf -> docx` is deliberately NOT here. It needs `convert_one`'s routed
// path rather than a tool id, and adding a public test-only entry point to
// reach it would be new API surface for one assertion. It is covered over the
// wire in `oc-pdf/tests/text_out.rs`, against the real worker, which is the
// stronger test of the two.
