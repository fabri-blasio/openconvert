//! `oc-pdf` — the PDF rendering worker.
//!
//! Runs confined, speaks the host protocol on stdin/stdout, and knows nothing
//! about paths. It is handed bytes and asked for bytes: one PDF in, one
//! rasterised page out (`png` / `jpeg`).
//!
//! # What it links
//!
//! pdfium, dynamically, through the raw declarations in [`pdfium`] — no `-sys`
//! crate, the same pattern as `heif.rs` in oc-images. All FFI `unsafe` lives
//! in that module; this file only converts pixels between formats, which is
//! the separation the module boundary enforces: **the FFI module does C work,
//! this file does pixel-format work.**
//!
//! # Platform scope
//!
//! pdfium is linked on Windows builds only today; `build.rs` has no POSIX
//! link story yet. Rather than fail to LINK elsewhere, the engine gates its
//! FFI behind `cfg(windows)` and answers every request on other platforms
//! with a structured refusal naming the rebuild that would fix it — the same
//! honesty rule `oc-images` applies when libheif is absent.

mod docx;
mod dxf;
mod epub;
mod html;
mod htmlread;
mod ipynb;
mod lock;
mod markdown;
mod mdread;
mod odt;
mod office;
// Pure Rust, so they build everywhere. pdfium is the Windows-only part, and an
// earlier edit here left the gate on the wrong module -- which would have made
// merge and reorder unavailable on Linux while trying to compile the FFI that
// genuinely cannot build there.
mod pages;
#[cfg(windows)]
mod pdfium;
mod searchable;
mod sheets;
mod slides;
mod stamp;
// The layout reconstruction. Pure and testable on every platform precisely
// BECAUSE it is separate from the FFI that feeds it -- every rule in it is a
// judgement call, and a judgement call reachable only through pdfium is a
// judgement call with no tests.
mod text;
mod typeset;
mod vector;

use openconvert_worker::{run_real, Converted, Engine, Probed, RunLimits};

/// The PDF engine.
struct Pdf;

impl Engine for Pdf {
    fn name(&self) -> &'static str {
        "oc-pdf"
    }

    fn version(&self) -> String {
        // The receipt records what ran. pdfium has no version query in the
        // fifteen functions this worker binds, so the receipt names the
        // engine family; the DLL beside the binary IS the version.
        format!("oc-pdf {} (pdfium)", env!("CARGO_PKG_VERSION"))
    }

    #[cfg(windows)]
    fn probe(&self, bytes: &[u8]) -> Result<Probed, String> {
        pdfium::init_library();
        let (_doc, pages) =
            pdfium::open_document(bytes).map_err(|e| format!("could not open PDF: {e:?}"))?;
        // Page count comes from the container's own count; reaching here at
        // all means the document opened without a password, because this
        // worker passes none and pdfium refuses encrypted files it cannot
        // unlock.
        Ok(Probed::Document {
            pages: pages.max(0) as u32,
            encrypted: false,
        })
    }

    #[cfg(not(windows))]
    fn probe(&self, _bytes: &[u8]) -> Result<Probed, String> {
        Err(
            "this oc-pdf binary was built without pdfium, which this platform \
             does not link yet; rebuild on Windows to read PDFs"
                .into(),
        )
    }

    #[cfg(windows)]
    fn run(&self, bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
        self.run_params(bytes, to, &[], limits)
    }

    /// Page surgery: merging documents, and reordering one document's pages.
    ///
    /// **Not behind `cfg(windows)`, unlike everything else here.** The rest of
    /// this engine needs pdfium, which only Windows links today; `lopdf` is
    /// pure Rust, so merge and reorder work on every platform this builds for.
    /// Splitting them out is what stops a Linux build refusing an operation it
    /// is perfectly able to perform.
    ///
    /// Selected by an `op` parameter rather than by the target format, because
    /// both of these are PDF-to-PDF and the format alone cannot say which was
    /// meant.
    fn run_with_inputs(
        &self,
        bytes: &[u8],
        extras: &[Vec<u8>],
        models: &[Vec<u8>],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        let op = params
            .iter()
            .find(|(k, _)| k == "op")
            .map(|(_, v)| v.as_str());
        // Accumulated across a `compose`, which runs several stages and owes
        // the receipt what every one of them changed.
        #[allow(unused_mut)]
        let mut removed: Vec<String> = Vec::new();
        match op {
            Some("merge") => {
                if extras.is_empty() {
                    return Err("merging needs two or more documents".to_string());
                }
                let mut docs = Vec::with_capacity(extras.len() + 1);
                docs.push(bytes.to_vec());
                docs.extend(extras.iter().cloned());
                let (out, report) = pages::merge(&docs).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            // How many pages, without rendering any of them.
            //
            // The host has no other way to ask: `probe()` has no document
            // branch, so `preview_file` assumed one page for every PDF. The
            // answer comes back as text because that is what the protocol
            // carries; the caller parses one integer.
            Some("pages") => {
                let n = pages::count(bytes).map_err(|e| e.to_string())?;
                Ok(Converted {
                    bytes: n.to_string().into_bytes(),
                    removed: Vec::new(),
                })
            }
            Some("geometry") => {
                let page = number_param(params, "page", 1)?;
                let geometry = pages::geometry(bytes, page).map_err(|e| e.to_string())?;
                Ok(Converted {
                    bytes: serde_json::to_vec(&geometry).map_err(|e| e.to_string())?,
                    removed: Vec::new(),
                })
            }
            // Smaller, without changing a page.
            Some("compress") => {
                let quality = match params
                    .iter()
                    .find(|(k, _)| k == "quality")
                    .map(|(_, v)| v.as_str())
                {
                    None | Some("lossless") => None,
                    Some("85") => Some(85),
                    Some("45") => Some(45),
                    _ => return Err("unknown PDF compression level".into()),
                };
                let (out, report) =
                    pages::compress_with_quality(bytes, quality, limits.decode_pixels)
                        .map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            Some("reorder") => {
                let order = params
                    .iter()
                    .find(|(k, _)| k == "order")
                    .map(|(_, v)| v.as_str())
                    .ok_or_else(|| "reordering needs an `order` parameter".to_string())?;
                let parsed = parse_order(order)?;
                let (out, report) = pages::reorder(bytes, &parsed).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            // EXTRACT AND REMOVE ARE ONE OP, and the engine already had it.
            //
            // `pages::reorder` permits a subset — it says so in its own
            // refusal, "N of the document's M pages were not kept" — so
            // keeping pages 1-3 is a reorder to [1,2,3] and removing them is a
            // reorder to the complement. Two tool names, one operation, no new
            // page-tree rebuilder: `rebuild_page_tree` is the one that
            // repoints `/Parent` correctly and there must not be a second.
            //
            // The spec resolves HERE rather than in the host because it needs
            // the page count, and the document is already open in this
            // process. Asking the host to fetch a count first would be a round
            // trip whose only product is a number.
            Some("select") => {
                let spec = params
                    .iter()
                    .find(|(k, _)| k == "pages")
                    .map(|(_, v)| v.as_str())
                    .ok_or_else(|| "selecting pages needs a `pages` parameter".to_string())?;
                let mode = params
                    .iter()
                    .find(|(k, _)| k == "mode")
                    .map_or("keep", |(_, v)| v.as_str());

                let have = pages::count(bytes).map_err(|e| e.to_string())?;
                let named = pages::resolve_spec(spec, have).map_err(|e| e.to_string())?;
                let order = match mode {
                    "keep" => named,
                    "drop" => pages::complement_of(&named, have).map_err(|e| e.to_string())?,
                    other => return Err(format!("mode must be `keep` or `drop`, got {other:?}")),
                };

                let (out, report) = pages::reorder(bytes, &order).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            // The page count on its own, for a host that has to plan work
            // across the document without opening it. Splitting is what needs
            // this; nothing else should.
            Some("count") => {
                let have = pages::count(bytes).map_err(|e| e.to_string())?;
                Ok(Converted {
                    bytes: have.to_string().into_bytes(),
                    removed: Vec::new(),
                })
            }
            // ROTATE AND CROP TAKE THE SAME SELECTION AS SELECT, and default
            // to every page. A tool that silently rotated only page one would
            // be the wrong default for a sideways scan, which is the case
            // these exist for.
            Some("rotate") => {
                let have = pages::count(bytes).map_err(|e| e.to_string())?;
                let chosen = selection(params, have)?;
                // SIGNED, because a quarter turn anticlockwise is `-90` and
                // is a thing people want. `pages::rotate` normalises whatever
                // arrives into {0, 90, 180, 270}, so a negative turn is not a
                // special case anywhere below this line.
                let turn: i64 = match params.iter().find(|(k, _)| k == "turn") {
                    None => 90,
                    Some((_, v)) => v.trim().parse().map_err(|_| {
                        format!("turn must be a whole number of degrees, got {v:?}")
                    })?,
                };
                let (out, report) =
                    pages::rotate(bytes, &chosen, turn).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            Some("crop") => {
                let have = pages::count(bytes).map_err(|e| e.to_string())?;
                let chosen = selection(params, have)?;
                let insets = [
                    float_param(params, "left", 0.0)?,
                    float_param(params, "bottom", 0.0)?,
                    float_param(params, "right", 0.0)?,
                    float_param(params, "top", 0.0)?,
                ];
                let (out, report) =
                    pages::crop(bytes, &chosen, insets).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            // PASSWORDS ARRIVE AS PIPE FRAMES, NOT ARGV. That is a property
            // of the worker protocol rather than of this arm, and it is what
            // keeps a password out of a process listing. Nothing here may put
            // one on a command line or into the `removed` list, which is what
            // reaches the receipt.
            Some("unlock") => {
                let password = params
                    .iter()
                    .find(|(k, _)| k == "password")
                    .map_or("", |(_, v)| v.as_str());
                let out = lock::unlock(bytes, password).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: vec!["the password: this document is no longer encrypted".to_string()],
                })
            }
            Some("protect") => {
                let password = params
                    .iter()
                    .find(|(k, _)| k == "password")
                    .map_or("", |(_, v)| v.as_str());
                let owner = params
                    .iter()
                    .find(|(k, _)| k == "owner_password")
                    .map_or("", |(_, v)| v.as_str());
                let out = lock::protect(bytes, password, owner).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: vec![concat!(
                        "nothing: the content is unchanged and now encrypted with ",
                        "AES-256. Every permission is granted -- PDF permissions are ",
                        "a request to the reader, not something the file enforces"
                    )
                    .to_string()],
                })
            }
            Some("searchable") => {
                let [layout] = extras else {
                    return Err("A searchable PDF needs the OCR layout".into());
                };
                let out = searchable::write(bytes, layout, limits)?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: vec![
                        "source image metadata; recognized text can contain OCR errors".into(),
                    ],
                })
            }
            Some("stamp") => {
                let [image] = extras else {
                    return Err(format!(
                        "stamping needs exactly one image alongside the document, got {}",
                        extras.len()
                    ));
                };
                let at = stamp::Placement {
                    page: number_param(params, "page", 1)?,
                    x: float_param(params, "x", 72.0)?,
                    y: float_param(params, "y", 72.0)?,
                    width: float_param(params, "width", 144.0)?,
                };
                let (out, report) = stamp::stamp(bytes, image, at).map_err(|e| e.to_string())?;
                enforce_output_cap(out.len(), limits)?;
                Ok(Converted {
                    bytes: out,
                    removed: describe(&report),
                })
            }
            // ONE OPERATION THAT DOES ALL THREE, because the alternative is
            // three files.
            //
            // Merging, reordering and signing are separate tools and separate
            // worker calls, and a user who wants all three wants ONE document
            // at the end of it. Running them in sequence from the host would
            // write an output and a receipt at each step: two files nobody
            // asked for, and two receipts describing documents that no longer
            // exist by the time the third finishes.
            //
            // So the sequence happens here, on the bytes, and one output comes
            // back. SR-11 is satisfied by one receipt for one output, and the
            // report is the union of what each stage removed.
            //
            // THE ORDER OF THE THREE IS FIXED AND IT MATTERS. Merge first,
            // because page numbers in `order` refer to the combined document.
            // Reorder second, because the page a signature goes on is a page
            // of the FINAL document. Sign last, for the same reason. Any other
            // sequence makes `page` mean something different depending on
            // which stages the caller happened to ask for.
            Some("compose") => {
                // The image, when there is one, is the LAST extra: everything
                // before it is another document to merge. Documents and images
                // arrive on the same wire, so the boundary has to be stated
                // rather than sniffed -- and the host is the one that knows.
                let stamp_specs = params
                    .iter()
                    .find(|(k, _)| k == "stamps")
                    .map(|(_, v)| serde_json::from_str::<serde_json::Value>(v))
                    .transpose()
                    .map_err(|_| "invalid signature placements")?;
                let image_count = number_param(params, "image_count", 0)?;
                if image_count > extras.len() || image_count > 32 {
                    return Err("invalid signature image count".into());
                }
                let (doc_inputs, images) = extras.split_at(extras.len() - image_count);
                let has_image = params.iter().any(|(k, v)| k == "image" && v == "1");
                let (docs_extra, image) = if has_image {
                    let Some((last, rest)) = doc_inputs.split_last() else {
                        return Err("composing with an image needs the image".to_string());
                    };
                    (rest, Some(last))
                } else {
                    (doc_inputs, None)
                };

                let mut current = if docs_extra.is_empty() {
                    bytes.to_vec()
                } else {
                    let mut docs = Vec::with_capacity(docs_extra.len() + 1);
                    docs.push(bytes.to_vec());
                    docs.extend(docs_extra.iter().cloned());
                    let (out, report) = pages::merge(&docs).map_err(|e| e.to_string())?;
                    removed.extend(describe(&report));
                    out
                };

                if let Some((_, order)) = params.iter().find(|(k, _)| k == "order") {
                    if !order.trim().is_empty() {
                        let parsed = parse_order(order)?;
                        let (out, report) =
                            pages::reorder(&current, &parsed).map_err(|e| e.to_string())?;
                        removed.extend(describe(&report));
                        current = out;
                    }
                }

                // TURNS AND CROPS, PER PAGE, after the order is settled.
                //
                // Same reasoning as the signature below: a page number here
                // means a page of the FINAL document, so these run after the
                // merge and the reorder rather than before. Rotating "page 3"
                // and then moving page 3 elsewhere would otherwise rotate
                // whatever landed in that slot.
                //
                // Grouped by value rather than applied one page at a time:
                // `pages::rotate` takes a SET of pages and one turn, so four
                // pages turned the same way are one call and one report line
                // instead of four.
                for (turn, pages) in group_by_value(parse_page_values(
                    params
                        .iter()
                        .find(|(k, _)| k == "turns")
                        .map_or("", |(_, v)| v.as_str()),
                    1,
                )?) {
                    let (out, report) = pages::rotate(&current, &pages, turn[0] as i64)
                        .map_err(|e| e.to_string())?;
                    removed.extend(describe(&report));
                    current = out;
                }

                for (insets, pages) in group_by_value(parse_page_values(
                    params
                        .iter()
                        .find(|(k, _)| k == "crops")
                        .map_or("", |(_, v)| v.as_str()),
                    4,
                )?) {
                    let four = [insets[0], insets[1], insets[2], insets[3]];
                    let (out, report) =
                        pages::crop(&current, &pages, four).map_err(|e| e.to_string())?;
                    removed.extend(describe(&report));
                    current = out;
                }

                if let Some(image) = image {
                    let at = stamp::Placement {
                        page: number_param(params, "page", 1)?,
                        x: float_param(params, "x", 72.0)?,
                        y: float_param(params, "y", 72.0)?,
                        width: float_param(params, "width", 144.0)?,
                    };
                    let (out, report) =
                        stamp::stamp(&current, image, at).map_err(|e| e.to_string())?;
                    removed.extend(describe(&report));
                    current = out;
                }

                if let Some(specs) = stamp_specs {
                    let items = specs
                        .as_array()
                        .ok_or("signature placements must be a list")?;
                    if items.len() != images.len() {
                        return Err("signature count does not match images".into());
                    }
                    for (item, image) in items.iter().zip(images) {
                        let number = |key: &str| -> Result<f32, String> {
                            item[key]
                                .as_str()
                                .ok_or_else(|| format!("missing {key}"))?
                                .parse()
                                .map_err(|_| format!("invalid {key}"))
                        };
                        let page = item["page"]
                            .as_str()
                            .ok_or("missing page")?
                            .parse::<usize>()
                            .map_err(|_| "invalid page")?;
                        let at = stamp::Placement {
                            page,
                            x: number("x")?,
                            y: number("y")?,
                            width: number("width")?,
                        };
                        let (out, report) =
                            stamp::stamp(&current, image, at).map_err(|e| e.to_string())?;
                        removed.extend(describe(&report));
                        current = out;
                    }
                }
                enforce_output_cap(current.len(), limits)?;
                Ok(Converted {
                    bytes: current,
                    removed,
                })
            }
            Some(other) => Err(format!(
                "oc-pdf does not understand the operation {other:?}"
            )),
            // No `op`: the ordinary render path, which is where every
            // pre-existing caller lands.
            None => {
                if !extras.is_empty() {
                    return Err(format!(
                        "oc-pdf takes one input for this operation, was handed {} more",
                        extras.len()
                    ));
                }
                self.run_model(bytes, models, to, params, limits)
            }
        }
    }

    /// Convert with parameters. The only parameter this worker understands is
    /// `page`, selecting which page renders; with none, page 0 renders —
    /// exactly what this worker always did.
    #[cfg(windows)]
    fn run_params(
        &self,
        bytes: &[u8],
        to: &str,
        params: &[(String, String)],
        limits: &RunLimits,
    ) -> Result<Converted, String> {
        // TEXT OUT IS A DIFFERENT JOB, and it is dispatched on CONTENT.
        //
        // A `.docx` and a `.odt` are both ZIPs, so the destination alone does
        // not say which member to read — and neither does the extension, which
        // this worker never sees. The archive's own contents decide.
        // A PDF's own text, out to plain text or into a Word document. Both
        // read the same characters and infer the same paragraphs; only the
        // last step differs, so they share one branch and cannot drift.
        const ZIP: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];
        /// Everything reachable from a PDF's own text.
        // KEPT IN STEP WITH THE ROUTE TABLE, and it was not.
        //
        // `Pdf -> Html` was declared in `route.rs`, passed routing, spawned this
        // worker and was refused here with "oc-pdf does not write html" —
        // because this list did not carry it. A route the table offers and the
        // engine rejects is worse than a missing one: it survives planning, so
        // the user sees it as available and finds out at the last step.
        //
        // `every_declared_document_route_runs` (crates/openconvert/tests) now
        // drives a fixture through every declared document route, using the
        // real CLI and real workers, which is what catches this class.
        const FROM_PDF_TEXT: [&str; 5] = ["txt", "docx", "markdown", "html", "odt"];
        /// Everything reachable from a Word or OpenDocument file.
        const FROM_OFFICE: [&str; 6] = ["txt", "markdown", "pdf", "odt", "docx", "html"];

        // A DXF IS TEXT, so it is checked before the text path — which would
        // otherwise parse a drawing as CommonMark and emit its group codes as
        // prose. The opening pair is what says it is one.
        const FROM_DXF: [&str; 2] = ["pdf", "svg"];
        if FROM_DXF.contains(&to) && is_dxf(bytes) {
            return run_dxf(bytes, to, limits);
        }

        // A NOTEBOOK IS JSON, so it is checked before the text path: `{` is
        // the first byte of both, and `mdread` would happily parse a notebook
        // as prose and emit its JSON as a paragraph.
        const FROM_NOTEBOOK: [&str; 6] = ["pdf", "docx", "odt", "html", "markdown", "txt"];
        if FROM_NOTEBOOK.contains(&to) && is_notebook(bytes) {
            return run_notebook(bytes, to, limits);
        }

        // TEXT IN. Markdown, plain text and HTML all arrive here, and all
        // three are parsed as CommonMark -- see `mdread` for why that is the
        // whole design rather than a shortcut. A PDF is not text and is
        // excluded by its own signature.
        const FROM_TEXT: [&str; 6] = ["pdf", "docx", "odt", "html", "markdown", "txt"];
        if FROM_TEXT.contains(&to) && !bytes.starts_with(&ZIP) && !bytes.starts_with(b"%PDF") {
            return run_text(bytes, to, limits);
        }
        if FROM_PDF_TEXT.contains(&to) && !bytes.starts_with(&ZIP) {
            return run_pdf_text(bytes, to, params, limits);
        }
        // THE OTHER ZIPS, and every one of them is decided by what is
        // INSIDE. A pptx, an odp, an epub, a xlsx and an ods all open `PK`
        // exactly as a docx does, and this worker never sees a filename -- so
        // the members decide, the same rule `run_office` already applied to
        // separate Word from OpenDocument.
        //
        // A workbook is checked before the deck and the book because it is the
        // only one of the three whose targets are not documents, and mixing
        // that ordering would send a `.xlsx -> csv` into the deck reader.
        if bytes.starts_with(&ZIP) {
            const FROM_SHEET: [&str; 3] = ["csv", "json", "xlsx"];
            if FROM_SHEET.contains(&to) {
                return run_sheet(bytes, to, limits);
            }
            const FROM_DECK: [&str; 6] = ["pdf", "docx", "odt", "html", "markdown", "txt"];
            if FROM_DECK.contains(&to) {
                if let Some(deck) = deck_kind(bytes) {
                    return run_deck(bytes, deck, to, limits);
                }
                if is_epub(bytes) {
                    return run_epub(bytes, to, limits);
                }
            }
        }
        if FROM_OFFICE.contains(&to) && bytes.starts_with(&ZIP) {
            return run_office(bytes, to, limits);
        }

        // Refuse an unwritable target BEFORE opening anything: the answer is
        // the same regardless of what the bytes contain.
        let format = match to {
            "png" => image::ImageFormat::Png,
            "jpeg" => image::ImageFormat::Jpeg,
            other => return Err(format!("oc-pdf does not write {other}")),
        };

        // The one parameter, parsed before anything opens. An unrecognised
        // parameter is an ERROR rather than a silent ignore: silently dropping
        // it would render something other than what was asked, under a clean
        // receipt.
        let mut page: u32 = 0;
        for (key, value) in params {
            match key.as_str() {
                "max_px" => {
                    value
                        .parse::<i32>()
                        .map_err(|_| "max_px must be an integer".to_string())?;
                }
                "page" => {
                    page = value
                        .parse()
                        .map_err(|_| format!("page must be a whole number, got {value:?}"))?;
                }
                other => {
                    return Err(format!(
                        "oc-pdf does not understand the parameter {other:?}"
                    ));
                }
            }
        }

        pdfium::init_library();
        let (doc, pages) = pdfium::open_document(bytes).map_err(|e| {
            format!("could not open PDF: {e:?}. The file may be corrupt or password-protected.")
        })?;

        // The index is checked against the count the document itself reported,
        // before anything loads. A 3-page document asked for page 9 is a
        // refusal naming both numbers, not a pdfium null-handle error.
        if page >= pages.max(0) as u32 {
            return Err(format!(
                "this document has {pages} page(s); asked for page {page} (zero-based)"
            ));
        }

        // Measure BEFORE painting: the entire point of a limit is refusing
        // before paying for what it refuses, and rendering allocates the
        // bitmap it paints into. The dimensions come from the same capped
        // helper the render uses, so the budgeted size and the painted size
        // cannot disagree. Loading the page twice — once to measure, once to
        // paint — costs microseconds next to the render itself.
        //
        // The message contains "limit" on purpose — the runtime reads that
        // word to decide the failure is retryable rather than fatal.
        let (width, height) = pdfium::page_dimensions(&doc, page)
            .map_err(|e| format!("could not render page: {e:?}"))?;
        let pixels = u64::from(width) * u64::from(height);
        if pixels > limits.decode_pixels {
            return Err(format!(
                "page {page} renders to {width}x{height} = {pixels} pixels, over the limit of {}",
                limits.decode_pixels
            ));
        }

        let requested = params
            .iter()
            .find(|(key, _)| key == "max_px")
            .map(|(_, value)| {
                value
                    .parse::<u32>()
                    .map_err(|_| "invalid preview resolution")
            })
            .transpose()?
            .map(|n| n.clamp(64, 4096));
        if requested.is_some_and(|n| u64::from(n) * u64::from(n) > limits.decode_pixels) {
            return Err("preview resolution exceeds the pixel limit".into());
        }
        let rendered = pdfium::render_page_sized(&doc, page, requested)
            .map_err(|e| format!("could not render page: {e:?}"))?;

        // BGRA (pdfium's layout) → RGBA (the image crate's): swap the blue
        // and red bytes of every pixel in place. Alpha arrives un-premultiplied
        // because FPDFBitmap_BGRA was requested, not the premultiplied form.
        let mut rgba = rendered.bgra;
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let (width, height) = (rendered.width, rendered.height);

        let img = image::RgbaImage::from_raw(width, height, rgba)
            .ok_or("rendered pixel buffer does not match its declared dimensions")?;
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, format)
            .map_err(|e| format!("could not encode to {to}: {e}"))?;

        Ok(Converted {
            bytes: out.into_inner(),
            // Rendering to pixels keeps nothing from the source container:
            // metadata, attachments and annotation objects are all dropped
            // with the document. Annotations are drawn INTO the raster, so
            // their ink survives while everything else about them does not.
            removed: vec![format!(
                "all PDF metadata and annotations (rendered page {page})"
            )],
        })
    }

    #[cfg(not(windows))]
    fn run(&self, _bytes: &[u8], _to: &str, _limits: &RunLimits) -> Result<Converted, String> {
        Err(
            "this oc-pdf binary was built without pdfium, which this platform \
             does not link yet; rebuild on Windows to convert PDFs"
                .into(),
        )
    }
}

fn main() -> std::process::ExitCode {
    // `run_real`, never `serve`: this binary applies its own confinement
    // before the first request arrives (Linux), and reports what the host's
    // spawn actually engaged (Windows).
    match run_real(&Pdf) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            // stderr, never stdout: stdout is the protocol, and a stray line
            // on it desynchronises the host's framing for the rest of the
            // session.
            eprintln!("oc-pdf: {e}");
            std::process::ExitCode::from(1)
        }
    }
}

/// A one-based page order written as `3,1,2`.
///
/// Parsed here rather than on the host so the worker refuses a malformed order
/// itself: a parameter it does not understand must be an error, and "silently
/// reordered to something else" is the failure this avoids.
fn parse_order(text: &str) -> Result<Vec<usize>, String> {
    text.split(',')
        .map(|part| {
            part.trim()
                .parse::<usize>()
                .map_err(|_| format!("{part:?} is not a page number"))
        })
        .collect()
}

/// Per-page values, as `page:v[,v...];page:v[,v...]`.
///
/// One page's values are comma-separated and pages are separated by
/// semicolons, so a crop reads `3:0,0,10,5;4:6,0,0,0` and a turn reads
/// `3:90;5:180`. `arity` is how many numbers each page must carry, and a page
/// carrying a different count is an error rather than a padded guess.
///
/// **Parsed here and not on the host**, for the same reason `parse_order` is:
/// a worker that does not understand a parameter must refuse it. Silently
/// dropping a malformed crop would apply some of what was asked for and none
/// of the rest, with a receipt claiming the whole thing ran.
///
/// An empty string is no work, not an error: "nothing was rotated" is a
/// perfectly ordinary compose.
fn parse_page_values(text: &str, arity: usize) -> Result<Vec<(usize, Vec<f32>)>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in text.split(';').filter(|e| !e.trim().is_empty()) {
        let (page, rest) = entry
            .split_once(':')
            .ok_or_else(|| format!("{entry:?} is not `page:value`"))?;
        let page: usize = page
            .trim()
            .parse()
            .map_err(|_| format!("{page:?} is not a page number"))?;
        if page == 0 {
            return Err("pages are numbered from 1".to_string());
        }
        let values: Vec<f32> = rest
            .split(',')
            .map(|v| {
                v.trim()
                    .parse::<f32>()
                    .map_err(|_| format!("{v:?} is not a number"))
            })
            .collect::<Result<_, _>>()?;
        if values.len() != arity {
            return Err(format!(
                "page {page} carries {} values and this operation takes {arity}",
                values.len()
            ));
        }
        if values.iter().any(|v| !v.is_finite()) {
            return Err(format!("page {page} carries a value that is not a number"));
        }
        out.push((page, values));
    }
    Ok(out)
}

/// Gather pages that share a value, so one call covers all of them.
///
/// The order of the groups follows first appearance, which keeps the operation
/// deterministic: two runs of the same request produce the same document, and
/// a hash-ordered grouping would not.
fn group_by_value(entries: Vec<(usize, Vec<f32>)>) -> Vec<(Vec<f32>, Vec<usize>)> {
    let mut groups: Vec<(Vec<f32>, Vec<usize>)> = Vec::new();
    for (page, values) in entries {
        match groups.iter_mut().find(|(v, _)| *v == values) {
            Some((_, pages)) => pages.push(page),
            None => groups.push((values, vec![page])),
        }
    }
    groups
}

/// Refuse a result larger than the memory this job was given.
///
/// `RunLimits` carries no output ceiling -- the host applies that on the way
/// back -- so the bound used here is `memory_bytes`, which is the ceiling this
/// process is actually running under. A merge that would exceed it fails before
/// the bytes cross the pipe rather than after.
fn enforce_output_cap(len: usize, limits: &RunLimits) -> Result<(), String> {
    if len as u64 > limits.memory_bytes {
        return Err(format!(
            "the merged document is {len} bytes, over the {} this job allows",
            limits.memory_bytes
        ));
    }
    Ok(())
}

/// A PDF's own text, out as plain text or as a Word document.
///
/// # This is not OCR, and the difference is the class
///
/// A PDF stores text as text. pdfium hands back the characters the file
/// already contains, so nothing here is recognised or guessed — which is
/// why `pdf -> txt` is Class B beside OCR rows that are Class D. What is
/// lost is the layout around the words, and that loss is on the receipt.
///
/// `pdf -> docx` is Class C because of the second half: paragraphs,
/// headings and reading order are INFERRED from glyph geometry (`text.rs`)
/// and a PDF contains none of them. The words are the file's; the
/// structure is a conclusion.
///
/// # A scanned PDF is refused, not silently emptied
///
/// A page of scanned paper has no text objects at all, so extraction
/// yields nothing. Writing that out is a zero-byte `.txt` or an empty Word
/// document, and both look like success — the user gets a file, opens it
/// later, and finds their document gone. It refuses by name instead, and
/// names the tool that WOULD work: OCR reads pictures of text, and this
/// build has it.
#[cfg(windows)]
fn run_pdf_text(
    bytes: &[u8],
    to: &str,
    params: &[(String, String)],
    limits: &RunLimits,
) -> Result<Converted, String> {
    // No parameters, and an unrecognised one is refused rather than
    // ignored -- the same rule the render path follows, for the same
    // reason: silently dropping a parameter produces something other than
    // what was asked under a clean receipt.
    if let Some((key, _)) = params.iter().find(|(k, _)| k != "page") {
        return Err(format!(
            "oc-pdf does not understand the parameter {key:?} when reading text"
        ));
    }

    pdfium::init_library();
    let (doc, pages) = pdfium::open_document(bytes).map_err(|e| {
        format!("could not open PDF: {e}. The file may be corrupt or password-protected.")
    })?;

    // The running total is checked EVERY page, not once at the end. A
    // document engineered to expand into gigabytes of glyphs is refused
    // partway through rather than after the allocation it was refused for.
    let mut per_page: Vec<Vec<text::Paragraph>> = Vec::with_capacity(pages.max(0) as usize);
    let mut total = 0_usize;
    for index in 0..pages.max(0) as u32 {
        let chars = pdfium::page_chars(&doc, index)
            .map_err(|e| format!("could not read the text of page {}: {e}", index + 1))?;
        let paragraphs = text::layout(&chars);
        total += paragraphs.iter().map(|p| p.text.len()).sum::<usize>();
        // "limit" is in the message on purpose -- the runtime reads that
        // word to decide a failure is retryable rather than fatal.
        if total as u64 > limits.memory_bytes {
            return Err(format!(
                "this document's text passed {total} bytes by page {}, over the limit of {}",
                index + 1,
                limits.memory_bytes
            ));
        }
        per_page.push(paragraphs);
    }

    let empty = per_page.iter().all(Vec::is_empty);
    if empty {
        return Err(format!(
            concat!(
                "this PDF has no text in it -- all {pages} page(s) are images, ",
                "which is what a scan is. The characters would have to be ",
                "recognised from the pixels; convert it to an image and run ",
                "text recognition on that."
            ),
            pages = pages
        ));
    }

    // A page that yielded nothing inside a document that mostly did is a
    // scan spliced into a born-digital file, or a full-page figure. It is
    // reported rather than dropped silently, because the output is missing
    // a page of the user's document and nothing else would say so.
    let blank: Vec<String> = per_page
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_empty())
        .map(|(i, _)| (i + 1).to_string())
        .collect();

    let mut removed = vec![
        "all page geometry: fonts, sizes, colour, columns and margins".to_string(),
        "images, annotations, form fields, links and attachments".to_string(),
        "all PDF metadata".to_string(),
    ];
    if !blank.is_empty() {
        removed.push(format!(
            concat!(
                "page(s) {} contained no text objects and produced nothing ",
                "(scanned or image-only pages)"
            ),
            blank.join(", ")
        ));
    }

    /// The one sentence every structured target from a PDF has to carry.
    ///
    /// A PDF records glyphs at coordinates and nothing else: no paragraph, no
    /// heading, no reading order. Every one of those in the output was
    /// reconstructed by `text::layout` from where the characters sit, and a
    /// receipt that did not say so would be claiming the document had a
    /// structure it recovered rather than one it invented.
    const INFERRED: &str = concat!(
        "paragraph breaks, headings and reading order are ",
        "INFERRED from glyph positions; a PDF records none of them"
    );

    let out = match to {
        "txt" => text::to_plain(&per_page).into_bytes(),
        "markdown" => {
            removed.push(INFERRED.to_string());
            markdown::to_markdown(&per_page).into_bytes()
        }
        "html" => {
            removed.push(INFERRED.to_string());
            html::write(&flatten(&per_page), "Document").into_bytes()
        }
        "odt" => {
            removed.push(INFERRED.to_string());
            odt::write(&flatten(&per_page))?
        }
        // The only other target this function is reached for.
        _ => {
            removed.push(INFERRED.to_string());
            docx::write(&per_page)?
        }
    };
    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// Whether these bytes open like a notebook.
///
/// The cheap check, matching what `sniff::structural` does: two markers near
/// the start rather than a full parse of a file that may be very large. The
/// real check is in `ipynb::parse`, which refuses by name.
fn is_notebook(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    let t = text.trim_start();
    t.starts_with('{') && t.contains("\"cells\"") && t.contains("\"cell_type\"")
}

/// A Jupyter notebook, out to a document.
///
/// Reuses every writer the text and office paths use: a notebook is prose,
/// code and output, and all three are already expressible.
fn run_notebook(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    let paragraphs = ipynb::parse(bytes)?;

    let mut removed = vec![
        "figures and images: each is replaced by a line saying one was there".to_string(),
        "execution counts, cell metadata, attachments and widget state".to_string(),
        "output MIME types other than plain text (HTML, LaTeX, JavaScript)".to_string(),
    ];

    let out = match to {
        "markdown" => markdown::to_markdown(&[paragraphs]).into_bytes(),
        "txt" => text::to_plain(&[paragraphs]).into_bytes(),
        "html" => html::write(&paragraphs, "Notebook").into_bytes(),
        "odt" => odt::write(&paragraphs)?,
        "docx" => docx::write(&[paragraphs])?,
        _ => {
            let (pdf, lost) = typeset::write(&paragraphs)?;
            removed.push("the notebook had no page geometry: this is typeset on A4".to_string());
            if lost.unmappable > 0 {
                removed.push(format!(
                    "{} character(s) with no glyph in the PDF base font, replaced with '?'",
                    lost.unmappable
                ));
            }
            if lost.truncated > 0 {
                removed.push(format!(
                    "{} code line(s) wider than the page, cut rather than wrapped",
                    lost.truncated
                ));
            }
            pdf
        }
    };

    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// A text file — Markdown, plain text or HTML — out to a document.
///
/// # One reader for all three
///
/// `mdread::parse` handles every case. Plain text is CommonMark with no
/// markup, so it needs no separate path, and that is what makes routing from
/// `Txt` safe when detection cannot tell a `.md` from a `.txt`: whichever
/// name the file arrives under, the same parse runs and the same document
/// comes out.
///
/// HTML is the exception, and it needed its own reader.
///
/// This comment used to say HTML came through `mdread` with "its tags stripped
/// rather than interpreted", yielding "its prose without its structure". The
/// prose did not survive either: CommonMark passes raw HTML through as
/// `Event::Html`, `mdread` drops that rather than emitting markup as words, and
/// a whole HTML document is one raw block. Every `html -> *` route wrote an
/// empty file and reported success. [`htmlread`] is the reader that fixes it,
/// and it is a text extractor rather than an HTML parser — see its own header
/// for why that boundary is the security argument rather than a shortcut.
fn run_text(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    let source = std::str::from_utf8(bytes)
        .map_err(|e| format!("this file is not valid UTF-8 text: {e}"))?;
    let looks_like_html = htmlread::looks_like(source);

    let paragraphs = if looks_like_html {
        htmlread::parse(source)
    } else {
        mdread::parse(source)
    };

    let mut removed = vec![
        "inline emphasis, links and images; their text is kept and the markup is not".to_string(),
    ];
    if looks_like_html {
        removed.push(
            concat!(
                "HTML attributes, styling and scripts; headings, lists, code blocks ",
                "and quotations are kept, and a table flattens to a line per row"
            )
            .to_string(),
        );
    }

    let out = match to {
        "markdown" => markdown::to_markdown(&[paragraphs]).into_bytes(),
        "txt" => text::to_plain(&[paragraphs]).into_bytes(),
        "html" => html::write(&paragraphs, "Document").into_bytes(),
        "odt" => odt::write(&paragraphs)?,
        "docx" => docx::write(&[paragraphs])?,
        // The remaining target this function is reached for.
        _ => {
            let (pdf, lost) = typeset::write(&paragraphs)?;
            removed.push(
                concat!(
                    "the source had no page geometry: this is typeset on A4 in ",
                    "Helvetica, with Courier for preformatted blocks"
                )
                .to_string(),
            );
            if lost.unmappable > 0 {
                removed.push(format!(
                    "{} character(s) with no glyph in the PDF base font, replaced with '?'",
                    lost.unmappable
                ));
            }
            if lost.truncated > 0 {
                removed.push(format!(
                    "{} preformatted line(s) wider than the page, cut rather than wrapped",
                    lost.truncated
                ));
            }
            pdf
        }
    };

    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// A Word or OpenDocument file, out to whatever it was asked for.
///
/// # One reader, four writers
///
/// Both inputs are ZIPs of XML and neither carries an extension this worker
/// can see, so the archive's own contents decide which it is. From there every
/// target reads the same paragraphs; only the last step differs. Keeping that
/// in one function is what stops `-> pdf` and `-> odt` drifting apart about
/// what a heading is.
///
/// `-> txt` deliberately still uses [`office::extract`] rather than the
/// structured reader: plain text has nowhere to put a heading, its output is
/// pinned by tests, and routing it through a second scanner would put every
/// change made for the other three formats at risk of changing it.
/// Whether these bytes open the way a DXF does.
///
/// The same four-line opening `sniff::structural` uses to detect one. Checked
/// here as well because this worker dispatches on CONTENT and never sees a
/// name — the host's verdict is not an input to it.
fn is_dxf(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(&bytes[..bytes.len().min(256)]) else {
        return false;
    };
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next() == Some("0") && lines.next() == Some("SECTION")
}

/// A CAD drawing, out to a page or to SVG.
fn run_dxf(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    let drawing = dxf::read(bytes)?;

    let mut removed = vec![
        "layers, colours, line types and line weights".to_string(),
        "text, dimensions, hatches and block insertions".to_string(),
        "the curves themselves: arcs, bulges and splines are evaluated and drawn as \
         polylines, finely enough that the error is under a thousandth of any radius"
            .to_string(),
    ];
    // NAMED, NOT JUST COUNTED. A drawing that lost its dimensions should not
    // come back looking complete, and "3 DIMENSION" is something a reader can
    // act on where "some entities" is not.
    if !drawing.skipped.is_empty() {
        removed.push(format!(
            "entities this build does not draw: {}",
            drawing
                .skipped
                .iter()
                .map(|(name, n)| format!("{n} {name}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let out = if to == "svg" {
        vector::to_svg(&drawing).into_bytes()
    } else {
        let (pdf, place) = vector::write(&drawing)?;
        // THE SCALE IS THE ONE NUMBER SOMEBODY NEEDS BACK. A DXF states no
        // page and no units this build trusts, so the drawing is fitted to A4
        // — and a cutting file whose measurements silently changed is the
        // failure this line exists to prevent.
        removed.push(format!(
            "the drawing's own scale: it is fitted to A4 at {:.4} points per drawing unit, \
             so measure from the source rather than from this page",
            place.scale
        ));
        pdf
    };
    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// Which deck format this ZIP is, by its members. `None` means it is not one.
fn deck_kind(bytes: &[u8]) -> Option<slides::Deck> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    if z.index_for_name("ppt/presentation.xml").is_some()
        || z.file_names().any(|n| n.starts_with("ppt/slides/"))
    {
        return Some(slides::Deck::Pptx);
    }
    // ODP and ODT are both a ZIP with a `content.xml`, so the member cannot
    // separate them -- the `mimetype` member can, and both formats require it
    // stored first for exactly this purpose.
    let mime = z
        .by_name("mimetype")
        .ok()
        .and_then(|mut m| {
            use std::io::Read as _;
            let mut s = String::new();
            m.by_ref().take(128).read_to_string(&mut s).ok()?;
            Some(s)
        })
        .unwrap_or_default();
    mime.contains("opendocument.presentation")
        .then_some(slides::Deck::Odp)
}

/// Whether this ZIP is an EPUB, by the parts the specification requires.
fn is_epub(bytes: &[u8]) -> bool {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .ok()
        .is_some_and(|z| z.index_for_name("META-INF/container.xml").is_some())
}

/// A slide deck, out to whatever document was asked for.
///
/// **One slide is one page.** `docx::write` and `typeset::write` both take
/// pages, so a deck printed to PDF gets a page per slide, which is what
/// `pptx -> pdf` is for. The flat writers take `flatten`, and the slide
/// boundary is genuinely lost there -- said so in the receipt rather than left
/// to be noticed.
fn run_deck(
    bytes: &[u8],
    deck: slides::Deck,
    to: &str,
    limits: &RunLimits,
) -> Result<Converted, String> {
    let per_slide = slides::read(bytes, deck)?;

    let mut removed = vec![
        "speaker notes: they are a separate part of the file and are not the deck".to_string(),
        "every image, shape, chart, animation and transition".to_string(),
        "the layout, the theme, and where anything sat on the slide".to_string(),
    ];

    let out = match to {
        "txt" => text::to_plain(&per_slide).into_bytes(),
        "markdown" => {
            removed.push(FLAT.to_string());
            markdown::to_markdown(&per_slide).into_bytes()
        }
        "html" => {
            removed.push(FLAT.to_string());
            html::write(&flatten(&per_slide), "Presentation").into_bytes()
        }
        "odt" => {
            removed.push(FLAT.to_string());
            odt::write(&flatten(&per_slide))?
        }
        "docx" => docx::write(&per_slide)?,
        // The remaining target: a page per slide, typeset.
        // The remaining target, and the one that flows. See the header.
        _ => {
            let (pdf, lost) = typeset::write(&flatten(&per_slide))?;
            removed.push(
                "the slide geometry: this is the deck's text re-typeset on A4 in Helvetica, \
                 flowing continuously rather than a page per slide, and not a picture of \
                 anything"
                    .to_string(),
            );
            if lost.unmappable > 0 {
                removed.push(format!(
                    "{} character(s) with no glyph in the PDF base font, replaced with '?'",
                    lost.unmappable
                ));
            }
            pdf
        }
    };
    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// What a flat target loses when a paged source reaches it.
const FLAT: &str = "the slide and chapter boundaries: this format has no page break to put them in";

/// An EPUB, out to whatever document was asked for.
///
/// One spine item is one page in DOCX, and the same split as `run_deck`
/// applies everywhere else: the typesetter flows a book continuously, which is
/// what a book is, and Markdown, HTML and ODT have no page break to put a
/// chapter boundary in.
fn run_epub(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    let per_chapter = epub::read(bytes)?;

    let mut removed = vec![
        "images, including the cover".to_string(),
        "the stylesheet, and every piece of styling in it".to_string(),
        "the navigation document, the page list and the metadata".to_string(),
    ];

    let out = match to {
        "txt" => text::to_plain(&per_chapter).into_bytes(),
        "markdown" => {
            removed.push(FLAT.to_string());
            markdown::to_markdown(&per_chapter).into_bytes()
        }
        "html" => {
            removed.push(FLAT.to_string());
            html::write(&flatten(&per_chapter), "Book").into_bytes()
        }
        "odt" => {
            removed.push(FLAT.to_string());
            odt::write(&flatten(&per_chapter))?
        }
        "docx" => docx::write(&per_chapter)?,
        _ => {
            let (pdf, lost) = typeset::write(&flatten(&per_chapter))?;
            removed.push(
                "the book's own page geometry: this is re-typeset on A4 in Helvetica".to_string(),
            );
            if lost.unmappable > 0 {
                removed.push(format!(
                    "{} character(s) with no glyph in the PDF base font, replaced with '?'",
                    lost.unmappable
                ));
            }
            pdf
        }
    };
    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// A workbook, out to one sheet of CSV or JSON.
fn run_sheet(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    // A WORKBOOK DESTINATION KEEPS EVERY SHEET, and the text ones cannot.
    // That is the whole difference between these two arms: CSV and JSON hold
    // one grid, so one sheet is taken and the rest are named in the receipt;
    // XLSX holds as many as the source did, so nothing is chosen and nothing
    // is lost to the choice.
    if to == "xlsx" {
        let book = sheets::read_all(bytes)?;
        let out = sheets::to_xlsx(&book)?;
        enforce_output_cap(out.len(), limits)?;
        return Ok(Converted {
            removed: sheets::removed_workbook(&book),
            bytes: out,
        });
    }

    let sheet = sheets::read(bytes)?;
    let out = match to {
        "json" => sheets::to_json(&sheet.rows).into_bytes(),
        // The only other target this is reached for.
        _ => sheets::to_csv(&sheet.rows).into_bytes(),
    };
    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed: sheets::removed(&sheet),
    })
}

fn run_office(bytes: &[u8], to: &str, limits: &RunLimits) -> Result<Converted, String> {
    let kind = {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes))
            .map_err(|e| format!("this file is not a readable ZIP container: {e}"))?;
        if z.by_name("word/document.xml").is_ok() {
            office::Office::Docx
        } else if z.by_name("content.xml").is_ok() {
            office::Office::Odt
        } else {
            return Err("this ZIP is neither a Word nor an OpenDocument text document".into());
        }
    };

    let mut removed = vec![
        "tables, images, headers, footers, footnotes and comments".to_string(),
        "tracked changes; only the current text is kept".to_string(),
    ];

    let out = if to == "txt" {
        removed.insert(
            0,
            "all formatting: styles, fonts, colour, sizes and alignment".to_string(),
        );
        office::extract(bytes, kind)?.into_bytes()
    } else {
        let paragraphs = office::structured(bytes, kind)?;
        removed.insert(
            0,
            "all formatting except the heading levels the document named".to_string(),
        );
        match to {
            "markdown" => markdown::to_markdown(&[paragraphs]).into_bytes(),
            "html" => html::write(&paragraphs, "Document").into_bytes(),
            "odt" => odt::write(&paragraphs)?,
            "docx" => docx::write(&[paragraphs])?,
            // The remaining target this function is reached for.
            _ => {
                let (pdf, lost) = typeset::write(&paragraphs)?;
                removed.push(
                    concat!(
                        "the original page geometry: this is re-typeset on A4 in ",
                        "Helvetica, not a picture of the source document"
                    )
                    .to_string(),
                );
                if lost.unmappable > 0 {
                    removed.push(format!(
                        "{} character(s) with no glyph in the PDF base font, replaced with '?'",
                        lost.unmappable
                    ));
                }
                pdf
            }
        }
    };

    enforce_output_cap(out.len(), limits)?;
    Ok(Converted {
        bytes: out,
        removed,
    })
}

/// The `pages` selection for an op that defaults to the whole document.
///
/// Absent means every page, which is the right default for rotate and crop:
/// somebody straightening a scan means the scan, not its first page.
fn selection(params: &[(String, String)], have: usize) -> Result<Vec<usize>, String> {
    match params.iter().find(|(k, _)| k == "pages") {
        Some((_, spec)) if !spec.trim().is_empty() => {
            pages::resolve_spec(spec, have).map_err(|e| e.to_string())
        }
        _ => Ok((1..=have).collect()),
    }
}

/// Every page's paragraphs as one sequence.
///
/// `docx::write` takes pages because a DOCX can carry a page break; HTML and
/// ODT have no such concept in this pipeline, so they take the flat list. A
/// page boundary in a PDF is a printing artefact — `text::to_plain` says the
/// same thing where it refuses to mark one — so nothing is lost by dropping it
/// here.
fn flatten(pages: &[Vec<text::Paragraph>]) -> Vec<text::Paragraph> {
    pages.iter().flatten().cloned().collect()
}

/// Turn a page report into the receipt's "removed" lines.
fn describe(report: &pages::Report) -> Vec<String> {
    let mut out = report.removed.clone();
    out.push(format!(
        "{} pages, {} link annotations carried across",
        report.pages, report.links_kept
    ));
    out
}

/// A whole-number parameter, or its default when absent.
fn number_param(params: &[(String, String)], key: &str, fallback: usize) -> Result<usize, String> {
    match params.iter().find(|(k, _)| k == key) {
        None => Ok(fallback),
        Some((_, v)) => v
            .parse()
            .map_err(|_| format!("{key} must be a whole number, got {v:?}")),
    }
}

/// A measurement in points, or its default when absent.
///
/// Refused rather than clamped when it is not a number: a stamp placed
/// somewhere other than where it was asked to go is worse than a refusal.
fn float_param(params: &[(String, String)], key: &str, fallback: f32) -> Result<f32, String> {
    match params.iter().find(|(k, _)| k == key) {
        None => Ok(fallback),
        Some((_, v)) => {
            let n: f32 = v
                .parse()
                .map_err(|_| format!("{key} must be a number of points, got {v:?}"))?;
            if !n.is_finite() {
                return Err(format!("{key} must be a finite number, got {v:?}"));
            }
            Ok(n)
        }
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn compose_keeps_two_signature_placements() {
        let (pdf, _) = typeset::write(&[text::Paragraph {
            text: "Original document".into(),
            style: text::Style::Body,
        }])
        .unwrap();
        let mut image = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            4,
            image::Rgba([20, 40, 80, 255]),
        ))
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
        let specs = serde_json::json!([{"page":"1","x":"40","y":"50","width":"100"},{"page":"1","x":"240","y":"250","width":"80"}]);
        let limits = RunLimits {
            decode_pixels: 1_000_000,
            memory_bytes: 128 << 20,
            archive_depth: 0,
            archive_entries: 0,
            archive_total_bytes: 0,
            use_gpu: false,
        };
        let out = Pdf
            .run_with_inputs(
                &pdf,
                &[image.get_ref().clone(), image.into_inner()],
                &[],
                "pdf",
                &[
                    ("op".into(), "compose".into()),
                    ("image_count".into(), "2".into()),
                    ("stamps".into(), specs.to_string()),
                ],
                &limits,
            )
            .unwrap();
        let doc = lopdf::Document::load_mem(&out.bytes).unwrap();
        let page = doc.get_pages()[&1];
        let content = String::from_utf8_lossy(&doc.get_page_content(page)).into_owned();
        assert!(content.contains("40.00 50.00 cm"), "{content}");
        assert!(content.contains("240.00 250.00 cm"), "{content}");
        assert_eq!(content.matches(" Do").count(), 2);
    }
}
