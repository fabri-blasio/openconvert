//! Page-level PDF surgery: merge and reorder, with the links kept alive.
//!
//! # Why this is not "concatenate the page trees"
//!
//! A PDF page carries its links in `/Annots` — an array of annotation
//! dictionaries. An external link (`\href{https://…}` in LaTeX) holds a
//! `/URI` action and survives any copy, because it refers to nothing inside the
//! file. An **internal** link (`\ref`, `\cite`, a table of contents entry) does
//! not: it refers to a *destination*, either directly as `/Dest` or through a
//! `/GoTo` action, and a destination is a reference to a page object.
//!
//! Three ways a naive merge breaks them, in increasing order of nastiness:
//!
//! 1. **Annotations dropped.** Copy page content and forget `/Annots`, and
//!    every link disappears. Obvious, and the easiest to get right.
//! 2. **Object references not remapped.** Copy `/Annots` verbatim into a
//!    document where object 12 means something else, and a link points at
//!    whatever now lives at 12. `lopdf`'s renumbering handles this, which is
//!    the reason this module leans on it rather than splicing bytes.
//! 3. **Named destinations collide.** This is the one that will bite anyone
//!    merging two LaTeX documents, and it is silent. `hyperref` generates
//!    *deterministic* names — `Doc-Start`, `page.1`, `section.1`, `figure.2` —
//!    so two independently compiled papers define the same names. Merge them
//!    with one flat name table and every `\ref` in the second document
//!    resolves into the first. No error, no warning; the links simply go to
//!    the wrong place, which is worse than losing them.
//!
//! So [`merge`] gives every source document its own namespace: each name is
//! rewritten to `d<N>_<original>`, and every reference to it inside that
//! document's own annotations and outlines is rewritten to match. Documents
//! cannot see each other's names afterwards, which is exactly the property a
//! merge should have.
//!
//! # What is not preserved, said plainly
//!
//! Form fields (`/AcroForm`), embedded file attachments, and document-level
//! JavaScript are dropped. Two documents' form field trees cannot be merged
//! without renaming fields, and a half-merged form is worse than none; the
//! other two are attack surface nobody asked to carry across. The caller
//! reports these on the receipt rather than the merge pretending they survived.

use lopdf::{Bookmark, Document, Object, ObjectId};

/// Why a page operation could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum PageError {
    /// The bytes are not a PDF this parser can open.
    #[error("could not read the PDF: {0}")]
    Parse(String),
    /// A page index outside the document.
    #[error("this document has {have} pages; page {asked} does not exist")]
    NoSuchPage {
        /// One-based index that was asked for.
        asked: usize,
        /// How many pages the document actually has.
        have: usize,
    },
    /// The requested order is not a permutation of the pages.
    #[error("{0}")]
    BadOrder(String),
    /// Serialising the result failed.
    #[error("could not write the PDF: {0}")]
    Write(String),
}

/// What a page operation changed, for the receipt.
#[derive(Debug, Default)]
pub struct Report {
    /// Human-readable notes about anything not carried across.
    pub removed: Vec<String>,
    /// Pages in the result.
    pub pages: usize,
    /// Link annotations kept.
    pub links_kept: usize,
}

/// Remove document metadata: the `/Info` dictionary and the XMP packet.
///
/// **UNREFERENCING IS NOT REMOVING**, and that distinction is the whole reason
/// this function exists rather than a line clearing the trailer key.
///
/// The PDF tools do not rasterise anything — merging and reordering move
/// objects between documents, so unlike an image being decoded and re-encoded,
/// there is nothing about the operation that inherently loses an author's name.
/// `merge` builds a fresh catalog, which meant no READER showed the metadata;
/// it also copied every object of every input across, so the author's name sat
/// in the output bytes, unreferenced and perfectly readable to anyone running
/// `strings` on the file. `reorder` did not even do that much: it kept the
/// document's own trailer, `/Info` and all.
///
/// A test asserts both — the trailer key is gone AND the bytes are not in the
/// file — because a privacy claim that only means "no reader surfaces it" is
/// not the claim the interface makes.
///
/// Note this is document-level metadata. Per-image EXIF inside an embedded
/// JPEG is a different question, and one this function does not answer; see
/// `crates/openconvert-run/src/pdfops.rs` for what the receipt says.
pub(crate) fn strip_document_metadata(doc: &mut Document) -> Vec<String> {
    let mut removed = Vec::new();

    // 1. The /Info dictionary: Author, Title, Producer, CreationDate.
    if let Ok(Object::Reference(id)) = doc.trailer.get(b"Info") {
        let id = *id;
        doc.trailer.remove(b"Info");
        // DELETE the object, do not merely stop pointing at it.
        if doc.objects.remove(&id).is_some() {
            removed.push("document metadata: author, title, producer and dates".to_string());
        }
    } else if doc.trailer.get(b"Info").is_ok() {
        // An inline dictionary rather than a reference. Rarer, equally revealing.
        doc.trailer.remove(b"Info");
        removed.push("document metadata: author, title, producer and dates".to_string());
    }

    // 2. The XMP packet hanging off the catalog, which carries the same facts
    //    again in a different syntax — and often more of them.
    let catalog_id = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| o.as_reference().ok());
    if let Some(catalog_id) = catalog_id {
        let meta_id = doc
            .get_object(catalog_id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .and_then(|d| d.get(b"Metadata").ok())
            .and_then(|o| o.as_reference().ok());
        if let Some(meta_id) = meta_id {
            if let Ok(dict) = doc.get_object_mut(catalog_id).and_then(Object::as_dict_mut) {
                dict.remove(b"Metadata");
            }
            if doc.objects.remove(&meta_id).is_some() {
                removed.push("the XMP metadata packet".to_string());
            }
        }
    }

    removed
}

/// Merge documents in the order given, keeping links working.
///
/// # Errors
///
/// [`PageError`] — an unreadable input, or a failure to serialise the result.
pub fn merge(docs: &[Vec<u8>]) -> Result<(Vec<u8>, Report), PageError> {
    if docs.len() < 2 {
        return Err(PageError::BadOrder(
            "merging needs two or more documents".to_string(),
        ));
    }

    let mut out = Document::with_version("1.7");
    let mut page_ids: Vec<ObjectId> = Vec::new();
    let mut report = Report::default();
    let mut bookmarks: Vec<(Bookmark, usize)> = Vec::new();
    // Every source's destination table, flattened and carried forward.
    //
    // Objects move across on their own, but nothing POINTS at them: the merged
    // document gets a fresh catalog, and a fresh catalog has no `/Names`. The
    // first version of this function stopped there, and the result was a file
    // whose destinations all existed and none of which could be found -- every
    // internal link dead, with no error anywhere. The test caught it by asking
    // the merged file what it defines and being told "nothing".
    let mut destinations: Vec<(Vec<u8>, Object)> = Vec::new();

    for (index, bytes) in docs.iter().enumerate() {
        let mut doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;

        // NAMESPACE FIRST, while the document is still standalone and its own
        // names still mean what its own links think they mean. Doing this after
        // the objects have been folded into `out` would mean guessing which
        // half of a merged name table belonged to whom.
        namespace_destinations(&mut doc, index);

        // `renumber_objects_with` shifts every object id in this document
        // clear of the ones already placed, and rewrites every internal
        // reference to match -- which is what keeps a link annotation pointing
        // at the page it was written for.
        let offset = out.max_id + 1;
        doc.renumber_objects_with(offset);
        out.max_id = doc.max_id;

        let pages = doc.get_pages();
        report.pages += pages.len();
        for (_, id) in pages {
            page_ids.push(id);
            report.links_kept += count_links(&doc, id);
        }

        collect_bookmarks(&doc, index, &mut bookmarks);
        // AFTER renumbering, so the page references inside each destination
        // are the ones the merged document will use.
        collect_destinations(&doc, &mut destinations);

        // STRIPPED BEFORE THE COPY, not after. `out.objects.extend` moves
        // every object of every input across, so an `/Info` dictionary removed
        // afterwards would already have been duplicated into the result under
        // a different id — and the fresh catalog would leave it unreferenced
        // and still readable in the bytes. Removing it here means it is never
        // copied at all.
        for line in strip_document_metadata(&mut doc) {
            if !report.removed.contains(&line) {
                report.removed.push(line);
            }
        }

        // Everything the document owns moves across. The page TREE is rebuilt
        // below; these are the objects it points at.
        out.objects.extend(doc.objects);
    }

    rebuild_page_tree(&mut out, &page_ids)?;
    install_destinations(&mut out, destinations);
    apply_bookmarks(&mut out, &bookmarks);

    report.removed.push(
        "form fields, embedded attachments and document-level JavaScript (not carried across a merge)"
            .to_string(),
    );

    let mut bytes = Vec::new();
    out.save_to(&mut bytes)
        .map_err(|e| PageError::Write(e.to_string()))?;
    Ok((bytes, report))
}

/// Reorder one document's pages.
///
/// `order` is one-based and must be a permutation of `1..=pages`: every page
/// appears exactly once. Dropping or duplicating a page is a different
/// operation with a different class, and accepting it here silently would let
/// "reorder" delete content.
///
/// How many pages a document has.
///
/// **Nothing could answer this before.** `probe()` returns `Properties::None`
/// for every document — documents have no probe — so the desktop's
/// `preview_file` fell through to its `_ => 1` arm and reported ONE page for
/// every PDF regardless of size. The reorder board drew a single card, the eye
/// preview showed a single page, and the signing column offered a single sheet,
/// all from the same missing number.
///
/// Cheap: `lopdf` walks the page tree, which is already parsed by `load_mem`.
/// No rendering, no rasterisation, no page content read.
///
/// # Errors
///
/// [`PageError`].
pub fn count(bytes: &[u8]) -> Result<usize, PageError> {
    let doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    Ok(doc.get_pages().len())
}

/// Rebuild a document with its pages in the given order.
///
/// # A SUBSET IS ALLOWED. IT DID NOT USED TO BE.
///
/// This required `order.len() == have` -- every page exactly once, a strict
/// permutation. The interface on top of it has never been that: every card on
/// the reorder board carries a bin, both odd/even shortcuts exist to drop half
/// the document, and the numbering field's own help text says "Anything you
/// leave out is dropped from the output".
///
/// So the one thing the screen most obviously offered was the one thing this
/// refused, and the refusal surfaced as "That did not produce a file. Nothing
/// was written." -- with no mention of pages, because the failure happened
/// several layers below the sentence. Two separate bug reports came out of it:
/// the save failing, and the odd/even shortcuts "removing more than they
/// should", which is what a failed save after a shortcut looks like.
///
/// What stays refused is a DUPLICATE. Selecting a subset is a choice; listing
/// the same page twice is a mistake, and copying a page is not what this
/// operation means.
///
/// An empty order is refused too: a document with no pages is not a document,
/// and every reader treats one as corrupt.
///
/// Make a document smaller without changing what is on any page.
///
/// # What it actually does, and what it deliberately does not
///
/// Two things, both lossless:
///
/// 1. **Deflate every stream that allows it.** Producers routinely write page
///    content, fonts and metadata uncompressed -- LaTeX and many report
///    generators do -- and those streams are text.
/// 2. **Drop the document metadata**, which is the `/Info` dictionary and the
///    XMP packet. Small, but it is also the part carrying the author's name and
///    the software that made the file, so removing it is worth doing on its own
///    and is disclosed rather than treated as a rounding error.
///
/// **It does not touch the images.** Re-encoding embedded JPEGs is where the
/// bytes usually are in a scanned document, and it is lossy, resolution-
/// dependent and impossible to undo -- which makes it a different operation
/// with a different name and a quality setting, not a silent part of "make this
/// smaller". A compressor that quietly degraded scans would be the worst kind
/// of helpful.
///
/// # A result that is not smaller is not returned
///
/// An already-optimised PDF comes back the same size or larger. Returning the
/// larger one would mean "compressing" a folder twice made it grow, so the
/// original bytes are kept and the report says nothing was gained.
///
/// # Errors
///
/// [`PageError`] if the document will not parse or will not serialise.
#[cfg(test)]
pub fn compress(bytes: &[u8]) -> Result<(Vec<u8>, Report), PageError> {
    compress_with_quality(bytes, None, 40_000_000)
}

/// Effective unrotated page box and inherited clockwise rotation.
pub fn geometry(bytes: &[u8], page: usize) -> Result<(f32, f32, i64), PageError> {
    let doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    let ids = page_ids(&doc);
    let id = *ids
        .get(page.saturating_sub(1))
        .ok_or(PageError::NoSuchPage {
            asked: page,
            have: ids.len(),
        })?;
    let [x0, y0, x1, y1] = rect_of(&doc, id, b"CropBox")
        .or_else(|| rect_of(&doc, id, b"MediaBox"))
        .ok_or_else(|| PageError::Parse("page has no box".into()))?;
    let rotation = inherited(&doc, id, b"Rotate")
        .and_then(|o| doc.dereference(o).ok()?.1.as_i64().ok())
        .unwrap_or(0)
        .rem_euclid(360);
    if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) || x1 <= x0 || y1 <= y0 {
        return Err(PageError::Parse("page has invalid dimensions".into()));
    }
    Ok((x1 - x0, y1 - y0, rotation))
}

/// Optimize streams and optionally recompress ordinary RGB/gray JPEG images.
/// Other colour spaces and image masks remain untouched.
pub fn compress_with_quality(
    bytes: &[u8],
    quality: Option<u8>,
    pixel_limit: u64,
) -> Result<(Vec<u8>, Report), PageError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    let pages = doc.get_pages().len();

    let mut report = Report {
        pages,
        ..Report::default()
    };
    report.removed.extend(strip_document_metadata(&mut doc));
    if let Some(quality) = quality {
        // Keep enough image pixels for the largest page; text and vectors remain intact.
        let dpi = if quality <= 45 { 120.0 } else { 180.0 };
        let longest = page_ids(&doc)
            .into_iter()
            .filter_map(|id| {
                let r = rect_of(&doc, id, b"CropBox").or_else(|| rect_of(&doc, id, b"MediaBox"))?;
                let unit = inherited(&doc, id, b"UserUnit")
                    .and_then(|v| doc.dereference(v).ok())
                    .and_then(|(_, v)| match v {
                        Object::Integer(n) => Some(*n as f32),
                        Object::Real(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(1.0);
                Some((r[2] - r[0]).max(r[3] - r[1]) * unit.max(1.0))
            })
            .fold(0.0_f32, f32::max);
        let bound = if longest.is_finite() && longest > 0.0 {
            (longest * dpi / 72.0).ceil().clamp(256.0, 16000.0) as u32
        } else {
            16000
        };
        let mut changed = 0;
        for object in doc.objects.values_mut() {
            let Ok(stream) = object.as_stream_mut() else {
                continue;
            };
            let name = |key: &[u8]| stream.dict.get(key).and_then(Object::as_name).ok();
            if name(b"Subtype") != Some(b"Image")
                || !matches!(name(b"ColorSpace"), Some(b"DeviceRGB" | b"DeviceGray"))
                || stream.dict.has(b"Decode")
                || stream.dict.has(b"DecodeParms")
                || stream.dict.has(b"SMask")
                || stream.dict.has(b"Mask")
            {
                continue;
            }
            let filter = name(b"Filter").map(|v| v.to_vec());
            let width = stream
                .dict
                .get(b"Width")
                .and_then(Object::as_i64)
                .unwrap_or(0);
            let height = stream
                .dict
                .get(b"Height")
                .and_then(Object::as_i64)
                .unwrap_or(0);
            if width <= 0
                || height <= 0
                || (width as u64).saturating_mul(height as u64) > pixel_limit
            {
                continue;
            }
            let decoded = if filter.as_deref() == Some(b"DCTDecode") {
                let mut reader = image::ImageReader::with_format(
                    std::io::Cursor::new(&stream.content),
                    image::ImageFormat::Jpeg,
                );
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(pixel_limit.saturating_mul(4));
                reader.limits(limits);
                let Ok(img) = reader.decode() else { continue };
                img
            } else if (filter.as_deref() == Some(b"FlateDecode") || filter.is_none())
                && stream
                    .dict
                    .get(b"BitsPerComponent")
                    .and_then(Object::as_i64)
                    .ok()
                    == Some(8)
            {
                let Ok(raw) = stream.get_plain_content_with_limit(
                    (width as usize)
                        .saturating_mul(height as usize)
                        .saturating_mul(3),
                ) else {
                    continue;
                };
                if name(b"ColorSpace") == Some(b"DeviceGray") {
                    let Some(img) = image::GrayImage::from_raw(width as u32, height as u32, raw)
                    else {
                        continue;
                    };
                    image::DynamicImage::ImageLuma8(img)
                } else {
                    let Some(img) = image::RgbImage::from_raw(width as u32, height as u32, raw)
                    else {
                        continue;
                    };
                    image::DynamicImage::ImageRgb8(img)
                }
            } else {
                continue;
            };
            let decoded = if decoded.width().max(decoded.height()) > bound {
                decoded.resize(bound, bound, image::imageops::FilterType::Lanczos3)
            } else {
                decoded
            };
            let rgb = decoded.to_rgb8();
            let Ok(encoded) = oc_image_codecs::jpeg(
                rgb.as_raw(),
                rgb.width() as usize,
                rgb.height() as usize,
                quality,
            ) else {
                continue;
            };
            if encoded.len() >= stream.content.len() {
                continue;
            }
            stream.dict.set("Width", i64::from(rgb.width()));
            stream.dict.set("Height", i64::from(rgb.height()));
            stream.dict.set("BitsPerComponent", 8);
            stream
                .dict
                .set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
            stream
                .dict
                .set("Filter", Object::Name(b"DCTDecode".to_vec()));
            stream.set_content(encoded);
            changed += 1;
        }
        if changed > 0 {
            report.removed.push(format!("Optimized {changed} images with JPEG quality {quality} and a conservative {dpi} DPI page bound; image detail reduced"));
        }
    }
    let unused = doc.prune_objects().len();
    if unused > 0 {
        report
            .removed
            .push(format!("Removed {unused} unreachable objects"));
    }
    doc.compress();

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PageError::Parse(e.to_string()))?;

    if out.len() >= bytes.len() {
        report.removed.clear();
        report.removed.push(format!(
            "nothing: this document is already compressed, so it was left as it \
             was ({} bytes)",
            bytes.len()
        ));
        return Ok((bytes.to_vec(), report));
    }

    let saved = bytes.len() - out.len();
    report.removed.push(format!(
        "{saved} bytes removed ({}% smaller); page structure preserved",
        saved * 100 / bytes.len().max(1)
    ));
    Ok((out, report))
}

/// Parse a 1-based page-range spec against a known page count.
///
/// Accepts `N`, `N-M`, and open-ended `N-` (to the last page). Separators are
/// commas; whitespace anywhere is ignored.
///
/// # Why this resolves in the worker
///
/// It needs the page count, and the host does not have one without opening the
/// document — which is the worker's job and nobody else's. Resolving here costs
/// nothing because the document is already open; resolving in the host would
/// cost a round trip whose only product is a number.
///
/// # Out of range is an error, not a clamp
///
/// `9-` against a five-page document fails. Silently returning pages 1..5
/// would hand back a result for a request nobody made — the user asked for
/// page 9, and the honest answer is that it does not exist. The message names
/// both numbers.
///
/// # Errors
///
/// [`PageError::BadOrder`] for a malformed spec or an empty result, and
/// [`PageError::NoSuchPage`] for a page beyond the document.
pub fn resolve_spec(spec: &str, have: usize) -> Result<Vec<usize>, PageError> {
    if have == 0 {
        return Err(PageError::BadOrder(
            "this document has no pages".to_string(),
        ));
    }

    let mut out: Vec<usize> = Vec::new();
    let mut saw_a_part = false;

    for raw in spec.split(',') {
        let part = raw.trim();
        if part.is_empty() {
            // A trailing or doubled comma is a typo, not a request. Ignoring
            // it silently would accept "1,,3" and ",,," alike, and the second
            // is a spec that selects nothing while looking deliberate.
            continue;
        }
        saw_a_part = true;

        let (first, last) = match part.split_once('-') {
            None => {
                let n = parse_page(part)?;
                (n, n)
            }
            Some((lo, hi)) => {
                let lo = lo.trim();
                let hi = hi.trim();
                // `-3` is not "up to 3": a spec with no start is as likely to
                // be a typo for `1-3` as for anything else, and guessing which
                // silently selects pages nobody asked for.
                if lo.is_empty() {
                    return Err(PageError::BadOrder(format!(
                        "{part:?} has no first page; write 1-{hi} if that is what you meant"
                    )));
                }
                let first = parse_page(lo)?;
                // `N-` IS open-ended, and deliberately so: "from here to the
                // end" is the one form whose meaning does not depend on
                // guessing.
                let last = if hi.is_empty() { have } else { parse_page(hi)? };
                (first, last)
            }
        };

        // BOUNDS BEFORE ORDER, AND THE END BEFORE THE LOOP.
        //
        // Two orderings matter here, and both were wrong first time.
        //
        // `9-` against a five-page document resolves its open end to 5, which
        // then looks like a backwards range 9..5 -- so the bounds check must
        // come before the order check, or the message describes the arithmetic
        // instead of the mistake.
        //
        // And `2-9` must name NINE. Discovering the problem inside the loop
        // reports page 6, which is where the walk hit the wall and not what
        // anybody typed; the user asked for pages up to 9 and that is the
        // number they need to see.
        if first > have {
            return Err(PageError::NoSuchPage { asked: first, have });
        }
        if last > have {
            return Err(PageError::NoSuchPage { asked: last, have });
        }
        if last < first {
            return Err(PageError::BadOrder(format!(
                "{part:?} runs backwards; pages are selected in document order"
            )));
        }
        for n in first..=last {
            out.push(n);
        }
    }

    if !saw_a_part {
        return Err(PageError::BadOrder("no pages were named".to_string()));
    }

    // Sorted and deduplicated, because `reorder` refuses a repeated page and
    // this function must never hand it one. "3,1,1-2" is a request for pages
    // 1, 2 and 3, and the alternative -- refusing it -- would be pedantry
    // about the order somebody typed.
    out.sort_unstable();
    out.dedup();

    if out.is_empty() {
        return Err(PageError::BadOrder(
            "that selection is empty; a document must keep at least one page".to_string(),
        ));
    }
    Ok(out)
}

/// One 1-based page number.
fn parse_page(text: &str) -> Result<usize, PageError> {
    let n: usize = text
        .parse()
        .map_err(|_| PageError::BadOrder(format!("{text:?} is not a page number")))?;
    // Page 0 does not exist in any user-facing numbering, and accepting it
    // would make an off-by-one silently produce the wrong page.
    if n == 0 {
        return Err(PageError::BadOrder(
            "pages are numbered from 1, so there is no page 0".to_string(),
        ));
    }
    Ok(n)
}

/// The pages NOT named by a selection, in document order.
///
/// What `mode=drop` needs. Kept beside [`resolve_spec`] because the two are
/// halves of one idea and a caller that computed the complement itself would
/// be the second place that has to agree about 1-based numbering.
///
/// # Errors
///
/// [`PageError::BadOrder`] when the selection covers every page: an empty
/// document is not a document, and a tool that produced one on request would
/// be doing something no reader can open.
pub fn complement_of(selected: &[usize], have: usize) -> Result<Vec<usize>, PageError> {
    let keep: Vec<usize> = (1..=have).filter(|n| !selected.contains(n)).collect();
    if keep.is_empty() {
        return Err(PageError::BadOrder(format!(
            "that would remove all {have} pages, and a PDF must keep at least one"
        )));
    }
    Ok(keep)
}

/// An attribute a page may inherit from an ancestor `/Pages` node.
///
/// # Why this exists rather than `dict.get(key)`
///
/// `/Rotate`, `/MediaBox`, `/CropBox` and `/Resources` are **inheritable**:
/// the PDF specification says a page without one of its own takes its parent's.
/// A page dictionary with no `/Rotate` is therefore not necessarily unrotated,
/// and reading it as `0` turns "rotate this by 90" into "set this to 90",
/// which silently *un*-rotates a document whose pages were all turned at the
/// tree level.
///
/// The walk is depth-bounded: a malformed file can point `/Parent` at a cycle,
/// and this runs on untrusted input.
fn inherited<'a>(doc: &'a Document, page_id: ObjectId, key: &[u8]) -> Option<&'a Object> {
    const MAX_DEPTH: usize = 64;

    let mut id = page_id;
    for _ in 0..MAX_DEPTH {
        let dict = doc.get_object(id).ok()?.as_dict().ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value);
        }
        id = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

/// Turn the selected pages by a relative angle.
///
/// `turn` is **relative**, not absolute: "rotate 90" means a quarter turn from
/// wherever the page already sits. Absolute would be a different tool and a
/// worse one — a user looking at a sideways scan wants it turned, and has no
/// reason to know what `/Rotate` it currently carries.
///
/// # Errors
///
/// [`PageError`] for a document that will not open, a page outside it, or a
/// turn that is not a multiple of 90.
pub fn rotate(bytes: &[u8], pages: &[usize], turn: i64) -> Result<(Vec<u8>, Report), PageError> {
    if turn % 90 != 0 {
        return Err(PageError::BadOrder(format!(
            "a PDF page turns in quarter circles; {turn} is not a multiple of 90"
        )));
    }

    let mut doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    let ordered = page_ids(&doc);
    let have = ordered.len();

    for &n in pages {
        let id = *ordered
            .get(n - 1)
            .ok_or(PageError::NoSuchPage { asked: n, have })?;

        // The EFFECTIVE angle, inherited if the page does not set one.
        let existing = inherited(&doc, id, b"Rotate")
            .and_then(|o| doc.dereference(o).ok()?.1.as_i64().ok())
            .unwrap_or(0);

        // Normalised into {0, 90, 180, 270}. Negative and out-of-range values
        // are legal in the wild -- `/Rotate -90` and `/Rotate 450` both occur
        // -- and Rust's `%` keeps the sign of the dividend, so the extra
        // `+ 360` is what stops a negative result.
        let angle = ((existing + turn) % 360 + 360) % 360;

        let dict = doc
            .get_object_mut(id)
            .map_err(|e| PageError::Parse(e.to_string()))?
            .as_dict_mut()
            .map_err(|e| PageError::Parse(e.to_string()))?;
        // Written onto the PAGE, never the ancestor: a page's own value wins,
        // and touching the parent would turn every sibling with it.
        dict.set("Rotate", Object::Integer(angle));
    }

    let report = Report {
        pages: have,
        ..Report::default()
    };
    let out = save(&mut doc)?;
    Ok((out, report))
}

/// Crop the selected pages to a rectangle, given as insets in points.
///
/// `insets` is `[left, bottom, right, top]` measured **inwards from the
/// current crop**, not as absolute user-space coordinates. That is the
/// difference between a tool that works on any document and one that only
/// works on documents whose `/MediaBox` starts at the origin — which is not a
/// safe assumption: scanned and imposed PDFs routinely start elsewhere.
///
/// # Errors
///
/// [`PageError`] for a document that will not open, a page outside it, or a
/// crop that leaves nothing.
pub fn crop(
    bytes: &[u8],
    pages: &[usize],
    insets: [f32; 4],
) -> Result<(Vec<u8>, Report), PageError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    let ordered = page_ids(&doc);
    let have = ordered.len();

    for &n in pages {
        let id = *ordered
            .get(n - 1)
            .ok_or(PageError::NoSuchPage { asked: n, have })?;

        // The page's effective bounds. `/CropBox` if it has one, `/MediaBox`
        // otherwise -- both inheritable, so both go through the parent walk.
        let media = rect_of(&doc, id, b"MediaBox")
            .ok_or_else(|| PageError::Parse(format!("page {n} has no usable MediaBox")))?;
        let current = rect_of(&doc, id, b"CropBox").unwrap_or(media);

        let [x0, y0, x1, y1] = current;
        let want = [
            x0 + insets[0],
            y0 + insets[1],
            x1 - insets[2],
            y1 - insets[3],
        ];

        // INTERSECTED WITH THE MEDIA BOX, per the specification: a crop box
        // larger than the media box is undefined and readers disagree about
        // it. Clamping is what makes "crop by -10" mean "no further out than
        // the page" rather than "undefined".
        let clipped = [
            want[0].max(media[0]),
            want[1].max(media[1]),
            want[2].min(media[2]),
            want[3].min(media[3]),
        ];

        if clipped[2] <= clipped[0] || clipped[3] <= clipped[1] {
            return Err(PageError::BadOrder(format!(
                "that crop leaves nothing of page {n}"
            )));
        }

        let dict = doc
            .get_object_mut(id)
            .map_err(|e| PageError::Parse(e.to_string()))?
            .as_dict_mut()
            .map_err(|e| PageError::Parse(e.to_string()))?;
        // `/MediaBox` IS LEFT ALONE, and only `/CropBox` is written. Readers
        // honour the crop box, and keeping the media box means the operation
        // is reversible: the content is hidden, not discarded.
        dict.set(
            "CropBox",
            Object::Array(clipped.iter().map(|v| Object::Real(*v)).collect::<Vec<_>>()),
        );
    }

    let report = Report {
        pages: have,
        ..Report::default()
    };
    let out = save(&mut doc)?;
    Ok((out, report))
}

/// A page's rectangle attribute, following inheritance.
fn rect_of(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<[f32; 4]> {
    let array = doc
        .dereference(inherited(doc, page_id, key)?)
        .ok()?
        .1
        .as_array()
        .ok()?;
    if array.len() != 4 {
        return None;
    }
    let mut out = [0.0_f32; 4];
    for (slot, value) in out.iter_mut().zip(array) {
        *slot = match doc.dereference(value).ok()?.1 {
            Object::Integer(i) => *i as f32,
            Object::Real(r) => *r,
            _ => return None,
        };
    }
    // A rectangle may be written with either corner first; the specification
    // says a reader must normalise it, and so must we or an inset would be
    // applied to the wrong edge.
    Some([
        out[0].min(out[2]),
        out[1].min(out[3]),
        out[0].max(out[2]),
        out[1].max(out[3]),
    ])
}

/// Page object ids in document order.
fn page_ids(doc: &Document) -> Vec<ObjectId> {
    let mut numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    numbers.sort_unstable();
    numbers
        .iter()
        .filter_map(|n| doc.get_pages().get(n).copied())
        .collect()
}

/// Serialise, with the same error the rest of this module reports.
fn save(doc: &mut Document) -> Result<Vec<u8>, PageError> {
    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PageError::Write(e.to_string()))?;
    Ok(out)
}

/// # Errors
///
/// [`PageError`].
pub fn reorder(bytes: &[u8], order: &[usize]) -> Result<(Vec<u8>, Report), PageError> {
    let mut doc = Document::load_mem(bytes).map_err(|e| PageError::Parse(e.to_string()))?;
    let pages = doc.get_pages();
    let have = pages.len();

    if order.is_empty() {
        return Err(PageError::BadOrder(
            "the order lists no pages; a document must keep at least one".to_string(),
        ));
    }
    if order.len() > have {
        return Err(PageError::BadOrder(format!(
            "the order lists {} pages; the document has only {have}",
            order.len()
        )));
    }
    let mut seen = vec![false; have];
    for &p in order {
        if p == 0 || p > have {
            return Err(PageError::NoSuchPage { asked: p, have });
        }
        if seen[p - 1] {
            return Err(PageError::BadOrder(format!(
                "page {p} appears twice; reordering keeps every page exactly once"
            )));
        }
        seen[p - 1] = true;
    }

    // Ordered by page NUMBER, which is what the caller's indices mean.
    let mut by_number: Vec<(u32, ObjectId)> = pages.into_iter().collect();
    by_number.sort_by_key(|(n, _)| *n);

    // The report counts what the OUTPUT has, not what the input had. With a
    // subset those differ, and the receipt describing the file has to describe
    // the file that exists.
    let mut report = Report {
        pages: order.len(),
        ..Report::default()
    };
    if order.len() < have {
        report.removed.push(format!(
            "{} of the document's {have} pages were not kept",
            have - order.len()
        ));
    }
    let page_ids: Vec<ObjectId> = order
        .iter()
        .map(|&p| {
            let id = by_number[p - 1].1;
            report.links_kept += count_links(&doc, id);
            id
        })
        .collect();

    rebuild_page_tree(&mut doc, &page_ids)?;
    report.removed.extend(strip_document_metadata(&mut doc));

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PageError::Write(e.to_string()))?;
    Ok((out, report))
}

/// Point the catalog's page tree at exactly these pages, in this order.
///
/// The existing `/Pages` node is reused so that anything else referring to it
/// stays valid; only its `/Kids` and `/Count` change, and each page's
/// `/Parent` is repointed at it.
fn rebuild_page_tree(doc: &mut Document, page_ids: &[ObjectId]) -> Result<(), PageError> {
    let pages_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Pages").ok())
        .and_then(|o| o.as_reference().ok())
        .unwrap_or_else(|| {
            let mut d = lopdf::Dictionary::new();
            d.set("Type", "Pages");
            doc.add_object(d)
        });

    for id in page_ids {
        if let Ok(page) = doc.get_object_mut(*id) {
            if let Ok(dict) = page.as_dict_mut() {
                dict.set("Parent", pages_id);
            }
        }
    }

    let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
    let count = i64::try_from(page_ids.len()).unwrap_or(i64::MAX);
    if let Ok(pages) = doc.get_object_mut(pages_id).and_then(Object::as_dict_mut) {
        pages.set("Kids", kids);
        pages.set("Count", count);
        pages.set("Type", "Pages");
    }

    // A merged document has no catalog of its own until one is made; a
    // reordered one already has the right catalog and this is a no-op.
    let has_catalog = doc.catalog().is_ok();
    if !has_catalog {
        let mut d = lopdf::Dictionary::new();
        d.set("Type", "Catalog");
        d.set("Pages", pages_id);
        let catalog = doc.add_object(d);
        doc.trailer.set("Root", catalog);
    }

    doc.max_id = doc.objects.keys().map(|(id, _)| *id).max().unwrap_or(0);
    Ok(())
}

/// How many link annotations a page carries.
fn count_links(doc: &Document, page: ObjectId) -> usize {
    doc.get_object(page)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Annots").ok())
        .and_then(|a| resolve_array(doc, a))
        .map_or(0, |annots| {
            annots
                .iter()
                .filter(|a| {
                    let dict = match a {
                        Object::Reference(id) => {
                            doc.get_object(*id).ok().and_then(|o| o.as_dict().ok())
                        }
                        Object::Dictionary(d) => Some(d),
                        _ => None,
                    };
                    dict.and_then(|d| d.get(b"Subtype").ok())
                        .and_then(|s| s.as_name().ok())
                        .is_some_and(|n| n == b"Link")
                })
                .count()
        })
}

fn resolve_array<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Vec<Object>> {
    match obj {
        Object::Array(a) => Some(a),
        Object::Reference(id) => doc.get_object(*id).ok().and_then(|o| o.as_array().ok()),
        _ => None,
    }
}

/// Give one document's named destinations a private namespace.
///
/// **The fix for the LaTeX collision.** Every name defined by this document is
/// prefixed, and every reference to a name from this document's own links and
/// outlines is prefixed identically — so they still find each other, and no
/// longer find anyone else's `section.1`.
fn namespace_destinations(doc: &mut Document, index: usize) {
    let prefix = format!("d{index}_");

    // 1. The definitions, in the catalog's /Names /Dests tree and in the
    //    older /Dests dictionary. Both are rewritten; a document may use
    //    either, and LaTeX has produced both over the years.
    let name_tree_ids: Vec<ObjectId> = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|n| n.as_reference().ok())
        .and_then(|id| doc.get_object(id).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Dests").ok())
        .and_then(|d| d.as_reference().ok())
        .into_iter()
        .collect();
    for id in name_tree_ids {
        prefix_name_tree(doc, id, &prefix);
    }

    let legacy: Vec<ObjectId> = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Dests").ok())
        .and_then(|d| d.as_reference().ok())
        .into_iter()
        .collect();
    for id in legacy {
        if let Ok(dict) = doc.get_object_mut(id).and_then(Object::as_dict_mut) {
            let renamed: Vec<(Vec<u8>, Object)> = dict
                .iter()
                .map(|(k, v)| {
                    let mut key = prefix.clone().into_bytes();
                    key.extend_from_slice(k);
                    (key, v.clone())
                })
                .collect();
            let mut fresh = lopdf::Dictionary::new();
            for (k, v) in renamed {
                fresh.set(k, v);
            }
            *dict = fresh;
        }
    }

    // 2. The references. Every /Dest that names a destination as a string, and
    //    every /GoTo action's /D, gets the same prefix.
    let ids: Vec<ObjectId> = doc.objects.keys().copied().collect();
    for id in ids {
        if let Ok(obj) = doc.get_object_mut(id) {
            prefix_refs_in(obj, &prefix);
        }
    }
}

/// Prefix every key of a name tree, recursing into `/Kids`.
fn prefix_name_tree(doc: &mut Document, id: ObjectId, prefix: &str) {
    let kids: Vec<ObjectId> = doc
        .get_object(id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Kids").ok())
        .and_then(|k| k.as_array().ok())
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    for kid in kids {
        prefix_name_tree(doc, kid, prefix);
    }

    if let Ok(dict) = doc.get_object_mut(id).and_then(Object::as_dict_mut) {
        if let Ok(Object::Array(names)) = dict.get(b"Names").cloned() {
            // A /Names array alternates key, value, key, value.
            let mut next = Vec::with_capacity(names.len());
            let mut iter = names.into_iter();
            while let (Some(key), Some(value)) = (iter.next(), iter.next()) {
                next.push(prefixed_string(&key, prefix));
                next.push(value);
            }
            dict.set("Names", next);
        }
        // /Limits carries the first and last key of the subtree; leaving them
        // unprefixed would make a reader's binary search miss every entry.
        if let Ok(Object::Array(limits)) = dict.get(b"Limits").cloned() {
            let next: Vec<Object> = limits.iter().map(|l| prefixed_string(l, prefix)).collect();
            dict.set("Limits", next);
        }
    }
}

fn prefixed_string(obj: &Object, prefix: &str) -> Object {
    match obj {
        Object::String(bytes, fmt) => {
            let mut next = prefix.as_bytes().to_vec();
            next.extend_from_slice(bytes);
            Object::String(next, *fmt)
        }
        other => other.clone(),
    }
}

/// Rewrite destination-by-name references inside one object.
fn prefix_refs_in(obj: &mut Object, prefix: &str) {
    match obj {
        Object::Dictionary(dict) => {
            // A /Dest naming a destination as a string, rather than an inline
            // array, is the form that depends on the name table.
            if let Ok(dest @ Object::String(..)) = dict.get(b"Dest").cloned() {
                dict.set("Dest", prefixed_string(&dest, prefix));
            }
            let is_goto = dict
                .get(b"S")
                .ok()
                .and_then(|s| s.as_name().ok())
                .is_some_and(|n| n == b"GoTo");
            if is_goto {
                if let Ok(d @ Object::String(..)) = dict.get(b"D").cloned() {
                    dict.set("D", prefixed_string(&d, prefix));
                }
            }
            for (_, v) in dict.iter_mut() {
                prefix_refs_in(v, prefix);
            }
        }
        Object::Array(items) => {
            for item in items.iter_mut() {
                prefix_refs_in(item, prefix);
            }
        }
        _ => {}
    }
}

/// Flatten one document's name tree into `(name, destination)` pairs.
///
/// Walks `/Kids`, because a large document's tree is a B-tree rather than one
/// flat array, and a merge that only read the root would carry across the
/// handful of names that happened to live there.
fn collect_destinations(doc: &Document, out: &mut Vec<(Vec<u8>, Object)>) {
    let root = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|n| n.as_reference().ok())
        .and_then(|id| doc.get_object(id).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Dests").ok())
        .and_then(|d| d.as_reference().ok());
    if let Some(id) = root {
        flatten_name_tree(doc, id, out);
    }

    // The older flat form. A document may use either; both are namespaced
    // above, so both are collected here.
    let legacy = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Dests").ok())
        .and_then(|d| d.as_reference().ok());
    if let Some(id) = legacy {
        if let Ok(dict) = doc.get_object(id).and_then(Object::as_dict) {
            for (k, v) in dict.iter() {
                out.push((k.to_vec(), v.clone()));
            }
        }
    }
}

fn flatten_name_tree(doc: &Document, id: ObjectId, out: &mut Vec<(Vec<u8>, Object)>) {
    let Ok(dict) = doc.get_object(id).and_then(Object::as_dict) else {
        return;
    };
    if let Ok(names) = dict.get(b"Names").and_then(Object::as_array) {
        for pair in names.chunks(2) {
            if let (Some(Object::String(key, _)), Some(value)) = (pair.first(), pair.get(1)) {
                out.push((key.clone(), value.clone()));
            }
        }
    }
    let kids: Vec<ObjectId> = dict
        .get(b"Kids")
        .and_then(Object::as_array)
        .map(|a| a.iter().filter_map(|o| o.as_reference().ok()).collect())
        .unwrap_or_default();
    for kid in kids {
        flatten_name_tree(doc, kid, out);
    }
}

/// Give the merged document one name tree holding every source's destinations.
///
/// Sorted by name, because a conforming reader binary-searches a name tree and
/// an unsorted one makes lookups miss entries that are present. The namespace
/// prefixes already keep documents apart; sorting is what makes them findable.
fn install_destinations(doc: &mut Document, mut pairs: Vec<(Vec<u8>, Object)>) {
    if pairs.is_empty() {
        return;
    }
    pairs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut names = Vec::with_capacity(pairs.len() * 2);
    for (key, value) in pairs {
        names.push(Object::String(key, lopdf::StringFormat::Literal));
        names.push(value);
    }

    let mut dests = lopdf::Dictionary::new();
    dests.set("Names", names);
    let dests_id = doc.add_object(dests);

    let mut names_dict = lopdf::Dictionary::new();
    names_dict.set("Dests", dests_id);
    let names_id = doc.add_object(names_dict);

    if let Ok(catalog) = doc.catalog_mut() {
        catalog.set("Names", names_id);
    }
}

/// Read one document's outline titles so the merged file keeps its bookmarks.
///
/// Deliberately shallow: top-level entries only, each pointing at the document
/// it came from. A full outline graft would have to reconcile two trees'
/// destinations, and a wrong bookmark is more annoying than a missing one.
fn collect_bookmarks(doc: &Document, index: usize, out: &mut Vec<(Bookmark, usize)>) {
    let first_page = doc.get_pages().into_iter().next().map(|(_, id)| id);
    let Some(page) = first_page else { return };
    let title = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| o.as_reference().ok())
        .and_then(|id| doc.get_object(id).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"First").ok())
        .and_then(|f| f.as_reference().ok())
        .and_then(|id| doc.get_object(id).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Title").ok())
        .and_then(|t| t.as_str().ok())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .unwrap_or_else(|| format!("Document {}", index + 1));

    out.push((Bookmark::new(title, [0.0, 0.0, 0.0], 0, page), index));
}

/// Attach the collected bookmarks to the merged document.
fn apply_bookmarks(doc: &mut Document, bookmarks: &[(Bookmark, usize)]) {
    if bookmarks.is_empty() {
        return;
    }
    for (mark, _) in bookmarks {
        doc.add_bookmark(mark.clone(), None);
    }
    doc.adjust_zero_pages();
    if let Some(outline) = doc.build_outline() {
        if let Ok(catalog) = doc.catalog_mut() {
            catalog.set("Outlines", Object::Reference(outline));
        }
    }
}

#[cfg(test)]
mod tests;
