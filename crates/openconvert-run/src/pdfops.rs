//! Merging and reordering PDFs: the host half.
//!
//! # Why these do not go through `route()`
//!
//! Every other tool reaches its worker by naming a [`Target::Operation`] and
//! letting `route()` build a plan. Two things stop that here, and both are
//! properties of the plan types rather than oversights:
//!
//! - **A merge has N inputs.** `PlanRequest` names one input format and one
//!   target. `openconvert concat` hit this first and says so in its own comment:
//!   the plan types model one operation per pair, and a join of five documents
//!   is not that shape.
//! - **A reorder carries a variable-length parameter.** `StepKind` is a small
//!   `Copy` enum in `openconvert-core`, which is a purity constraint the crate
//!   keeps deliberately. A page order is a `Vec<usize>`, and widening the plan
//!   vocabulary to hold one would cost the core its shape for one operation.
//!
//! So these follow `concat`'s precedent: drive the worker directly, write the
//! output through the one creation path, and build the receipt here. What a
//! receipt owes its reader is unchanged — what ran, in what confinement, what
//! class it was, what was dropped, and where the bytes went.
//!
//! # What does NOT change
//!
//! The bytes are still parsed inside `oc-pdf`, sandboxed, and the confinement
//! recorded on the receipt is still read back from the worker rather than
//! assumed. Skipping `route()` skips planning, not confinement.

use std::path::{Path, PathBuf};

use openconvert_core::facts::Provenance;
use openconvert_core::policy::Policy;
use openconvert_sandbox::argv::EngineBin;
use openconvert_sandbox::display::DisplayName;

use crate::handles::HandleTable;
use crate::pool::WorkerPool;
use crate::tools::{ToolError, ToolRun, ToolRunOptions};

/// Merge documents in the order given, keeping their links working.
///
/// # Errors
///
/// [`ToolError`] — fewer than two inputs, a file that is not a PDF, or a
/// worker refusal carrying the engine's own message.
pub fn merge(
    inputs: &[PathBuf],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    if inputs.len() < 2 {
        return Err(ToolError::BadInput(
            "merging needs two or more documents".to_string(),
        ));
    }
    let (first, rest) = read_all_pdfs(inputs)?;
    run_pdf_op(
        inputs,
        first,
        &rest,
        &[("op".to_string(), "merge".to_string())],
        "merged",
        "Merge",
        policy,
        options,
    )
}

/// Reorder one document's pages. `order` is one-based.
///
/// # Errors
///
/// [`ToolError`].
pub fn reorder(
    inputs: &[PathBuf],
    order: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "reordering takes exactly one document".to_string(),
        ));
    };
    if order.trim().is_empty() {
        return Err(ToolError::BadParam(
            "reordering needs the new page order".to_string(),
        ));
    }
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;
    run_pdf_op(
        inputs,
        bytes,
        &[],
        &[
            ("op".to_string(), "reorder".to_string()),
            ("order".to_string(), order.to_string()),
        ],
        "reordered",
        "Reorder",
        policy,
        options,
    )
}

/// Keep only the pages named.
///
/// # Errors
///
/// As [`reorder`]: one document, and a non-empty selection.
pub fn extract_pages(
    inputs: &[PathBuf],
    pages: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    select(
        inputs,
        pages,
        "keep",
        "pages",
        "Extract pages",
        policy,
        options,
    )
}

/// Keep everything EXCEPT the pages named.
///
/// # Errors
///
/// As [`reorder`], plus a refusal when the selection covers the whole
/// document — the engine will not produce a PDF with no pages.
pub fn remove_pages(
    inputs: &[PathBuf],
    pages: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    select(
        inputs,
        pages,
        "drop",
        "trimmed",
        "Remove pages",
        policy,
        options,
    )
}

/// The shared half of extract and remove.
///
/// One function because they are one operation with the selection inverted,
/// and two copies would be two places to keep the guards in step.
fn select(
    inputs: &[PathBuf],
    pages: &str,
    mode: &str,
    suffix: &str,
    label: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(format!(
            "{label} takes exactly one document"
        )));
    };
    if pages.trim().is_empty() {
        return Err(ToolError::BadParam(
            "name the pages, like 1-3,7 or 2-".to_string(),
        ));
    }
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;
    run_pdf_op(
        inputs,
        bytes,
        &[],
        &[
            ("op".to_string(), "select".to_string()),
            ("pages".to_string(), pages.to_string()),
            ("mode".to_string(), mode.to_string()),
        ],
        suffix,
        label,
        policy,
        options,
    )
}

/// Turn pages by a quarter, half or three-quarter circle.
///
/// `pages` may be empty, which means the whole document — the right default
/// for a sideways scan, which is what this is for.
///
/// # Errors
///
/// As [`reorder`], plus a turn that is not a multiple of 90.
pub fn rotate(
    inputs: &[PathBuf],
    pages: &str,
    turn: i64,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "rotating takes exactly one document".to_string(),
        ));
    };
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;
    run_pdf_op(
        inputs,
        bytes,
        &[],
        &[
            ("op".to_string(), "rotate".to_string()),
            ("pages".to_string(), pages.to_string()),
            ("turn".to_string(), turn.to_string()),
        ],
        "rotated",
        "Rotate",
        policy,
        options,
    )
}

/// Crop pages inwards by a margin on each side, in points.
///
/// # Errors
///
/// As [`reorder`], plus a crop that leaves nothing of a page.
pub fn crop(
    inputs: &[PathBuf],
    pages: &str,
    insets: [f32; 4],
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "cropping takes exactly one document".to_string(),
        ));
    };
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;
    run_pdf_op(
        inputs,
        bytes,
        &[],
        &[
            ("op".to_string(), "crop".to_string()),
            ("pages".to_string(), pages.to_string()),
            ("left".to_string(), insets[0].to_string()),
            ("bottom".to_string(), insets[1].to_string()),
            ("right".to_string(), insets[2].to_string()),
            ("top".to_string(), insets[3].to_string()),
        ],
        "cropped",
        "Crop",
        policy,
        options,
    )
}

/// Take a password off a document the caller can open.
///
/// # Errors
///
/// As [`reorder`], plus a wrong or missing password.
pub fn unlock(
    inputs: &[PathBuf],
    password: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    lock_op(
        inputs, "unlock", password, "", "unlocked", "Unlock", policy, options,
    )
}

/// Put an AES-256 password on a document.
///
/// # Errors
///
/// As [`reorder`], plus a missing password or an already-encrypted document.
pub fn protect(
    inputs: &[PathBuf],
    password: &str,
    owner: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    lock_op(
        inputs,
        "protect",
        password,
        owner,
        "protected",
        "Protect",
        policy,
        options,
    )
}

/// The shared half of unlock and protect.
///
/// # Why the password is not validated here
///
/// Only the engine can tell an empty password from a wrong one, because only
/// the engine opens the document. A host-side length check would duplicate the
/// refusal and could disagree with it.
#[allow(clippy::too_many_arguments)]
fn lock_op(
    inputs: &[PathBuf],
    op: &str,
    password: &str,
    owner: &str,
    suffix: &str,
    label: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(format!(
            "{label} takes exactly one document"
        )));
    };
    if password.is_empty() {
        return Err(ToolError::BadParam(
            "this needs the document's password".to_string(),
        ));
    }

    // READ WITHOUT PARSING. `read_all_pdfs` checks the `%PDF` header and
    // nothing else, so an encrypted document passes it -- which is required,
    // since unlocking one is the entire point.
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;

    let mut params = vec![
        ("op".to_string(), op.to_string()),
        ("password".to_string(), password.to_string()),
    ];
    if !owner.is_empty() {
        params.push(("owner_password".to_string(), owner.to_string()));
    }

    run_pdf_op(inputs, bytes, &[], &params, suffix, label, policy, options)
}

/// How a document is cut into parts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitSpec {
    /// Every `n` pages: 1-3, 4-6, and a short final part if it does not divide.
    EveryN(usize),
    /// One part per named range.
    Ranges(Vec<(usize, usize)>),
    /// Cut AFTER each of these pages.
    ///
    /// `2,5,10` on a twelve-page document is four parts: 1-2, 3-5, 6-10,
    /// 11-12. This is the form a person actually holds in their head -- "split
    /// it after the summary and after the appendix" -- and the one the
    /// interface offers. It is a separate variant rather than ranges computed
    /// in the interface because the LAST part runs to the end of the document,
    /// and the interface does not know where that is; `plan_parts` does.
    Boundaries(Vec<usize>),
}

/// Cut one document into several.
///
/// # The only tool that produces more than one file
///
/// Everything else in this module returns a single [`ToolRun`], and the worker
/// protocol returns one blob per request. Widening that protocol was
/// considered and rejected: every engine is written against one-blob-out, and
/// one tool does not justify changing the shape all of them assume.
///
/// So the fan-out is HERE, in the host. The page count is fetched once, the
/// parts are planned, and the engine's `select` op runs once per part — each
/// producing an ordinary single-output conversion with its own receipt.
///
/// # One receipt per file, deliberately
///
/// A single receipt covering six outputs would describe a document that does
/// not exist. Each part gets its own, describing only itself.
///
/// # Worker reuse
///
/// The parts share one [`WorkerPool`], and that is a decision rather than a
/// default. SR-20 governs *distinct untrusted files* sharing a worker; this is
/// one file read N times, which is not that — the same bytes, already admitted
/// once, cannot learn anything new about themselves on the second pass.
///
/// # Errors
///
/// As [`reorder`], plus a spec that names no parts.
pub fn split(
    inputs: &[PathBuf],
    spec: &SplitSpec,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<Vec<ToolRun>, ToolError> {
    let [input] = inputs else {
        return Err(ToolError::BadInput(
            "splitting takes exactly one document".to_string(),
        ));
    };
    let (bytes, _) = read_all_pdfs(std::slice::from_ref(input))?;

    // The page count comes from the engine, because only the engine opens the
    // document. Exposing `count` was the smaller change than moving the whole
    // split plan into the worker, and it is independently useful.
    let have = page_count(&bytes, policy)?;
    let parts = plan_parts(spec, have)?;

    let mut out = Vec::with_capacity(parts.len());
    for (first, last) in parts {
        let range = if first == last {
            first.to_string()
        } else {
            format!("{first}-{last}")
        };
        // NAMED BY THE PAGES IT HOLDS, not by an index. `report-1.pdf`
        // collides with the next split of the same document and says nothing;
        // `report-pages-1-3.pdf` survives a re-split and describes itself.
        let suffix = format!("pages-{range}");
        out.push(run_pdf_op(
            inputs,
            bytes.clone(),
            &[],
            &[
                ("op".to_string(), "select".to_string()),
                ("pages".to_string(), range),
                ("mode".to_string(), "keep".to_string()),
            ],
            &suffix,
            "Split",
            policy,
            options,
        )?);
    }
    Ok(out)
}

/// Ask the engine how many pages the document has.
///
/// A worker round trip for one number, which is the smaller of the two options
/// the plan for this weighed: the alternative was moving the whole split plan
/// into the worker, which would put output naming and the conflict policy — a
/// host concern — behind the sandbox boundary.
fn page_count(bytes: &[u8], policy: &Policy) -> Result<usize, ToolError> {
    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());
    let (out, _removed, _confinement) = pool
        .run_with_inputs(
            EngineBin::Pdf,
            Provenance::Untrusted,
            &limits,
            bytes.to_vec(),
            &[],
            "pdf",
            &[("op".to_string(), "count".to_string())],
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;

    String::from_utf8_lossy(&out)
        .trim()
        .parse()
        .map_err(|_| ToolError::BadInput("the engine did not report a page count".to_string()))
}

/// Turn a spec into inclusive 1-based page ranges.
fn plan_parts(spec: &SplitSpec, have: usize) -> Result<Vec<(usize, usize)>, ToolError> {
    if have == 0 {
        return Err(ToolError::BadInput(
            "this document has no pages".to_string(),
        ));
    }
    let parts: Vec<(usize, usize)> = match spec {
        SplitSpec::EveryN(n) => {
            if *n == 0 {
                return Err(ToolError::BadParam(
                    "split every how many pages? zero is not a size".to_string(),
                ));
            }
            (1..=have)
                .step_by(*n)
                .map(|first| (first, (first + n - 1).min(have)))
                .collect()
        }
        SplitSpec::Ranges(ranges) => {
            for &(first, last) in ranges {
                if first == 0 || last < first || last > have {
                    return Err(ToolError::BadParam(format!(
                        "{first}-{last} is not a range this {have}-page document has"
                    )));
                }
            }
            ranges.clone()
        }
        SplitSpec::Boundaries(cuts) => {
            let mut sorted: Vec<usize> = cuts.clone();
            sorted.sort_unstable();
            sorted.dedup();
            for &at in &sorted {
                if at == 0 || at >= have {
                    // `>= have` and not `> have`: cutting after the last page
                    // produces an empty final part, which is a request that
                    // cannot be honoured rather than one that produces
                    // nothing quietly.
                    return Err(ToolError::BadParam(format!(
                        "this document has {have} pages; there is nothing to cut after page {at}"
                    )));
                }
            }
            let mut parts = Vec::with_capacity(sorted.len() + 1);
            let mut first = 1;
            for &at in &sorted {
                parts.push((first, at));
                first = at + 1;
            }
            // The tail, which is why this is a variant and not a list of
            // ranges built by the caller.
            parts.push((first, have));
            parts
        }
    };
    if parts.is_empty() {
        return Err(ToolError::BadParam("that splits into nothing".to_string()));
    }
    Ok(parts)
}

/// What the page board asks for, as three strings the worker parses.
///
/// **One struct because they are one request.** They arrived as three more
/// arguments to `compose` and clippy was right to object at nine: they always
/// travel together, they are always empty together when nothing was
/// rearranged, and a caller passing two of the three has made a mistake the
/// type can prevent.
///
/// The formats are the worker's, and the host does not parse them -- see
/// `parse_page_values` in `oc-pdf`. Every page number is a position in the
/// FINAL document, after the merge and the reorder, which is also what the
/// signature's own `page` has always meant.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageEdits<'a> {
    /// The output order, `1,3,2`. Empty leaves the document as it is.
    pub order: &'a str,
    /// Quarter turns per page, `3:90;5:180`. Empty turns nothing.
    pub turns: &'a str,
    /// Margins per page in points, `3:0,0,10,5`. Empty crops nothing.
    pub crops: &'a str,
}

/// Where a stamp lands, as the UI measured it.
///
/// PDF user space: origin at the BOTTOM-left, units of 1/72 inch. The webview
/// works in top-left pixels, so the flip happens there, once, next to the page
/// whose height it needs — rather than here, where the page size is unknown.
#[derive(Debug, Clone, Copy)]
pub struct At {
    /// Distance from the left edge, in points.
    pub x: f32,
    /// Distance from the BOTTOM edge, in points.
    pub y: f32,
    /// Width on the page, in points; height follows the aspect ratio.
    pub width: f32,
}

impl Default for At {
    fn default() -> Self {
        // One inch in, one inch up, two inches wide -- the same defaults the
        // worker applies, repeated here so a caller that does not care gets
        // the same answer as one that omits the parameters entirely.
        Self {
            x: 72.0,
            y: 72.0,
            width: 144.0,
        }
    }
}

/// Merge, reorder and stamp in one pass, producing ONE output.
///
/// The three PDF tools each do one thing, and a person who wants all three
/// wants one document at the end. Running them in sequence from here would
/// write an output and a receipt per stage — two intermediate files nobody
/// asked for, and two receipts describing documents that stop existing the
/// moment the next stage runs.
///
/// So the sequence happens inside the worker, on the bytes, and this sends it
/// everything it needs at once: the documents in the order they should be
/// joined, the page order for the result, and the image if one is being
/// placed. Empty stages are skipped, so this is also the plain merge, the
/// plain reorder and the plain stamp.
///
/// `image` is the LAST extra input when it is present, and the `image=1`
/// parameter is what tells the worker where the documents stop. Documents and
/// images travel the same wire; guessing between them by sniffing would be a
/// decision made in the wrong place.
///
/// # Errors
///
/// [`ToolError`].
pub fn compose(
    inputs: &[PathBuf],
    image_path: Option<&str>,
    edits: PageEdits<'_>,
    at: At,
    page: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    compose_with_stamps(inputs, image_path, edits, at, page, policy, options, None)
}

/// Compose pages and multiple independently placed signature images.
// This public boundary mirrors the independent controls exposed by the CLI and
// desktop app; grouping them would only move the argument list into a wrapper.
#[allow(clippy::too_many_arguments)]
pub fn compose_with_stamps(
    inputs: &[PathBuf],
    image_path: Option<&str>,
    edits: PageEdits<'_>,
    at: At,
    page: &str,
    policy: &Policy,
    options: ToolRunOptions,
    stamps: Option<&str>,
) -> Result<ToolRun, ToolError> {
    let PageEdits {
        order,
        turns,
        crops,
    } = edits;
    if inputs.is_empty() {
        return Err(ToolError::BadInput(
            "there is nothing to compose".to_string(),
        ));
    }
    let (first, rest) = read_all_pdfs(inputs)?;

    let mut extras = rest;
    let mut params = vec![
        ("op".to_string(), "compose".to_string()),
        ("order".to_string(), order.to_string()),
    ];

    // PASSED THROUGH UNPARSED. The worker refuses a malformed one itself --
    // `parse_page_values` there -- and parsing it twice would be two places
    // for the format to be understood differently.
    //
    // Absent rather than empty when there is nothing: a compose that rotates
    // nothing should not carry a `turns` key at all, so the receipt's
    // parameter list says what was actually asked for.
    if !turns.trim().is_empty() {
        params.push(("turns".to_string(), turns.to_string()));
    }
    if !crops.trim().is_empty() {
        params.push(("crops".to_string(), crops.to_string()));
    }

    if let Some(raw) = stamps {
        let value: serde_json::Value = serde_json::from_str(raw)
            .map_err(|_| ToolError::BadParam("invalid signature placements".into()))?;
        let items = value
            .as_array()
            .ok_or_else(|| ToolError::BadParam("signature placements must be a list".into()))?;
        if items.len() > 32 {
            return Err(ToolError::BadParam(
                "at most 32 signatures per document".into(),
            ));
        }
        for item in items {
            let path = item["image"]
                .as_str()
                .ok_or_else(|| ToolError::BadParam("signature image is missing".into()))?;
            let image = std::fs::read(path).map_err(|e| ToolError::BadInput(e.to_string()))?;
            if image.len() > 32 << 20 {
                return Err(ToolError::BadInput("signature is too large".into()));
            }
            extras.push(image);
        }
        params.push(("stamps".into(), raw.into()));
        params.push(("image_count".into(), items.len().to_string()));
    }
    if let Some(path) = image_path.filter(|_| stamps.is_none()) {
        let image = std::fs::read(path).map_err(|e| {
            ToolError::BadInput(format!("could not read {}: {e}", DisplayName::new(path)))
        })?;
        extras.push(image);
        params.push(("image".to_string(), "1".to_string()));
        params.push(("page".to_string(), page.to_string()));
        params.push(("x".to_string(), at.x.to_string()));
        params.push(("y".to_string(), at.y.to_string()));
        params.push(("width".to_string(), at.width.to_string()));
    }

    run_pdf_op(
        inputs, first, &extras, &params, "composed", "Compose", policy, options,
    )
}

/// Draw an image onto one page of a document.
///
/// The image travels as an EXTRA INPUT rather than as a path in a parameter.
/// A worker has no filesystem — Landlock denies all of it on Linux and the
/// AppContainer denies it on Windows — so handing it a path would mean opening
/// a hole in exactly the confinement this design earns its receipts from. The
/// host reads the file and sends the bytes, the same way it sends the document.
///
/// # Errors
///
/// [`ToolError`].
pub fn stamp(
    inputs: &[PathBuf],
    image_path: &str,
    page: &str,
    at: At,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let [document] = inputs else {
        return Err(ToolError::BadInput(
            "stamping takes exactly one document".to_string(),
        ));
    };
    let (pdf, _) = read_all_pdfs(std::slice::from_ref(document))?;

    let image = std::fs::read(image_path).map_err(|e| {
        ToolError::BadInput(format!(
            "could not read {}: {e}",
            DisplayName::new(image_path)
        ))
    })?;

    run_pdf_op(
        inputs,
        pdf,
        &[image],
        &[
            ("op".to_string(), "stamp".to_string()),
            ("page".to_string(), page.to_string()),
            ("x".to_string(), at.x.to_string()),
            ("y".to_string(), at.y.to_string()),
            ("width".to_string(), at.width.to_string()),
        ],
        "stamped",
        "Stamp",
        policy,
        options,
    )
}

/// Read every input, refusing anything that is not a PDF by CONTENT.
///
/// The extension is not consulted: a file named `.pdf` that is something else
/// would otherwise be handed to the PDF parser on the strength of its name,
/// which is the whole thing SR-4 exists to prevent.
fn read_all_pdfs(inputs: &[PathBuf]) -> Result<(Vec<u8>, Vec<Vec<u8>>), ToolError> {
    let mut all = Vec::with_capacity(inputs.len());
    for path in inputs {
        let mut table = HandleTable::new();
        let facts = crate::detect::detect(path, &mut table)?;
        let sniff = facts.sniff();
        if sniff.detected != openconvert_core::format::FormatId::Pdf {
            return Err(ToolError::BadInput(format!(
                "{} is a {} by its contents, not a PDF",
                DisplayName::new(&path.to_string_lossy()),
                sniff.detected
            )));
        }
        if sniff.polyglot {
            return Err(ToolError::BadInput(
                "this file is valid as more than one format; it is quarantined rather than merged"
                    .to_string(),
            ));
        }
        all.push(crate::detect::bytes_of(&facts, &mut table)?);
    }
    let mut iter = all.into_iter();
    let first = iter.next().unwrap_or_default();
    Ok((first, iter.collect()))
}

/// Drive `oc-pdf`, write the output, and optionally write its receipt beside it.
#[allow(clippy::too_many_arguments)]
fn run_pdf_op(
    inputs: &[PathBuf],
    first: Vec<u8>,
    extras: &[Vec<u8>],
    params: &[(String, String)],
    suffix: &str,
    step_kind: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<ToolRun, ToolError> {
    let limits = policy.base_limits();
    let mut pool = WorkerPool::new(policy.worker_reuse());

    // UNTRUSTED, always. These are documents a stranger may have written, and
    // a merge reads every one of them. Treating them as trusted would let one
    // file's worker be reused for the next, which is the sharing rule SR-20
    // exists to constrain.
    let (out_bytes, removed, confinement) = pool
        .run_with_inputs(
            EngineBin::Pdf,
            Provenance::Untrusted,
            &limits,
            first,
            extras,
            "pdf",
            params,
        )
        .map_err(|e| ToolError::BadInput(e.to_string()))?;

    let anchor = inputs.first().ok_or_else(|| {
        ToolError::BadInput("this operation needs at least one document".to_string())
    })?;
    let stem = anchor.file_stem().map_or_else(
        || "document".to_string(),
        |s| s.to_string_lossy().into_owned(),
    );
    let name = format!("{stem}-{suffix}.pdf");
    let dest = anchor.parent().unwrap_or(Path::new(".")).to_path_buf();

    // The one creation path, so nothing already on disk is ever at risk.
    let (placed, file) = crate::write::create_output(&dest, &name, policy.on_conflict())
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    let output = match &placed {
        crate::write::Placed::Created(p) | crate::write::Placed::Skipped(p) => p.clone(),
    };
    let Some(mut f) = file else {
        return Err(ToolError::BadInput(format!(
            "{} already exists and the conflict policy is to skip",
            DisplayName::new(&output.to_string_lossy())
        )));
    };
    use std::io::Write as _;
    f.write_all(&out_bytes)
        .map_err(|e| ToolError::BadInput(e.to_string()))?;
    drop(f);

    let (receipt, receipt_body) = make_receipt(
        &name,
        inputs,
        &out_bytes,
        &removed,
        &confinement,
        step_kind,
        "oc-pdf",
        "pdf",
        // The page tree, the content streams and the link annotations all come
        // across untouched; what is dropped is named in `removed` rather than
        // hidden behind the class.
        "A (lossless)",
        policy,
        options,
    )?;
    Ok(ToolRun {
        output,
        receipt,
        receipt_body,
    })
}

/// The receipt, built here because there is no `Plan` to build it from.
///
/// Same reasoning as `openconvert concat`: an N-input operation is not the shape
/// the plan types model, and an output without a receipt is worse than no
/// output. Shared with `image-compose`, which is the same shape for the same
/// reason — several worker calls, one file, one record. Everything a reader is owed is still present — identity hash, what
/// ran, the confinement the worker read back about itself, the class, and what
/// was dropped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_receipt(
    name: &str,
    inputs: &[PathBuf],
    out_bytes: &[u8],
    removed: &[String],
    confinement: &str,
    step_kind: &str,
    // THE ENGINE THAT ACTUALLY RAN, not the one this function was written for.
    //
    // `"oc-pdf"` was a literal here, which was true while `pdfops` was the only
    // caller. `image-compose` shares this builder and runs `oc-images` and
    // `oc-ai`, so the literal turned every composed image's receipt into a
    // false statement about which parser had touched the bytes -- in the one
    // document whose entire purpose is to be checkable.
    engine: &str,
    // WHAT THE INPUT WAS, and WHAT THE CONVERSION COST -- both of which were
    // literals here for the same reason `engine` was, and both of which became
    // false the moment anything but `pdfops` called this.
    //
    // `"detected": "pdf"` on a composed IMAGE's receipt names a format the
    // file never had. `"class": "A (lossless)"` on one that upscaled with a
    // model and cut out a background claims the output is bit-for-bit the
    // input -- which is the single worst thing a receipt in this product can
    // say, because Class A is the load-bearing promise and the whole file
    // exists to be checked against it.
    detected: &str,
    class: &str,
    policy: &Policy,
    options: ToolRunOptions,
) -> Result<(Option<PathBuf>, String), ToolError> {
    let mut hasher = blake3::Hasher::new();
    for path in inputs {
        if let Ok(bytes) = std::fs::read(path) {
            hasher.update(&bytes);
        }
    }
    let sources: Vec<String> = inputs
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();

    let receipt = serde_json::json!({
        "version": 1,
        "tool": format!("openconvert {}", env!("CARGO_PKG_VERSION")),
        "content_id": hasher.finalize().to_hex().to_string(),
        "detected": detected,
        "sources": sources,
        "class": class,
        "steps": [{
            "kind": step_kind,
            "engine": engine,
            "isolation": confinement,
            "removed": removed,
        }],
        "output": name,
        "output_bytes": out_bytes.len(),
    });

    let body =
        serde_json::to_string_pretty(&receipt).map_err(|e| ToolError::BadInput(e.to_string()))?;
    let path = if options.write_sidecar {
        let dest = inputs
            .first()
            .and_then(|path| path.parent())
            .unwrap_or(Path::new("."));
        let receipt_name = format!("{name}.receipt.json");
        let (placed, file) = crate::write::create_output(dest, &receipt_name, policy.on_conflict())
            .map_err(|e| ToolError::BadInput(e.to_string()))?;
        let path = match &placed {
            crate::write::Placed::Created(path) | crate::write::Placed::Skipped(path) => {
                path.clone()
            }
        };
        if let Some(mut file) = file {
            use std::io::Write as _;
            file.write_all(body.as_bytes())
                .map_err(|e| ToolError::BadInput(e.to_string()))?;
        }
        Some(path)
    } else {
        None
    };
    Ok((path, body))
}

#[cfg(test)]
mod split_tests {
    use super::{plan_parts, SplitSpec};

    /// **`2,5,10` on twelve pages is four files.**
    ///
    /// The example from the request that introduced boundaries, asserted
    /// literally: three cuts make four parts, and the last one runs to the end
    /// of the document. That tail is the whole reason this is a variant
    /// resolved here rather than a list of ranges the interface could build --
    /// the interface does not know how many pages there are.
    #[test]
    fn boundaries_cut_after_each_named_page() {
        let parts = plan_parts(&SplitSpec::Boundaries(vec![2, 5, 10]), 12).expect("plan");
        assert_eq!(parts, vec![(1, 2), (3, 5), (6, 10), (11, 12)]);
    }

    /// One cut is two parts; the shape does not need a special case.
    #[test]
    fn one_boundary_is_two_parts() {
        let parts = plan_parts(&SplitSpec::Boundaries(vec![1]), 3).expect("plan");
        assert_eq!(parts, vec![(1, 1), (2, 3)]);
    }

    /// Out of order and repeated is a request, not a failure.
    ///
    /// Somebody typing `10,2,5,5` means the same thing as `2,5,10`, and
    /// refusing it would be pedantry about the order of a list whose order
    /// carries no information.
    #[test]
    fn boundaries_are_sorted_and_deduplicated() {
        let parts = plan_parts(&SplitSpec::Boundaries(vec![10, 2, 5, 5]), 12).expect("plan");
        assert_eq!(parts, vec![(1, 2), (3, 5), (6, 10), (11, 12)]);
    }

    /// **Cutting after the last page is refused, not silently dropped.**
    ///
    /// It would produce an empty final part. A tool that answers a request it
    /// cannot honour by quietly doing something else is the failure mode this
    /// whole module is written against.
    #[test]
    fn a_cut_past_the_end_is_refused_by_name() {
        let err = plan_parts(&SplitSpec::Boundaries(vec![12]), 12).unwrap_err();
        let text = format!("{err:?}");
        assert!(
            text.contains("12"),
            "the message must name the page: {text}"
        );
    }

    /// Page zero is not a page.
    #[test]
    fn a_cut_before_the_first_page_is_refused() {
        assert!(plan_parts(&SplitSpec::Boundaries(vec![0]), 5).is_err());
    }

    /// The other two forms still work, because scripts still use them.
    #[test]
    fn the_older_forms_are_unchanged() {
        assert_eq!(
            plan_parts(&SplitSpec::EveryN(2), 5).expect("every"),
            vec![(1, 2), (3, 4), (5, 5)]
        );
        assert_eq!(
            plan_parts(&SplitSpec::Ranges(vec![(1, 3), (4, 5)]), 5).expect("ranges"),
            vec![(1, 3), (4, 5)]
        );
    }
}
