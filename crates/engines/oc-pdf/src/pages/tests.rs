//! What a merge owes a LaTeX document.
//!
//! The fixtures are built here rather than checked in, because the property
//! under test is a *collision*: two independently compiled documents that both
//! define `section.1`. A pair of checked-in PDFs would have to be trusted to
//! still contain that collision; a pair built in the test cannot stop.

use super::*;
use lopdf::{Dictionary, Object, Stream};

#[test]
fn compression_levels_reduce_embedded_jpeg_and_preserve_page_geometry() {
    let img = image::RgbImage::from_fn(256, 256, |x, y| {
        image::Rgb([
            (x * 17 + y * 31) as u8,
            (x * 7 + y * 13) as u8,
            (x * 3 + y * 19) as u8,
        ])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 98)
        .encode_image(&img)
        .unwrap();
    let mut doc = Document::load_mem(&latex_like(2, "compression")).unwrap();
    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
    dict.set("Filter", "DCTDecode");
    dict.set("ColorSpace", "DeviceRGB");
    dict.set("Width", 256);
    dict.set("Height", 256);
    dict.set("BitsPerComponent", 8);
    let image_id = doc.add_object(Stream::new(dict, jpeg));
    let mut xobjects = Dictionary::new();
    xobjects.set("Photo", image_id);
    let mut resources = Dictionary::new();
    resources.set("XObject", xobjects);
    let page_id = doc.get_pages()[&1];
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"q 256 0 0 256 0 0 cm /Photo Do Q".to_vec(),
    ));
    doc.get_object_mut(page_id)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content);
    let mut original = Vec::new();
    doc.save_to(&mut original).unwrap();
    let (balanced, _) = compress_with_quality(&original, Some(85), 1_000_000).unwrap();
    let (smallest, report) = compress_with_quality(&original, Some(45), 1_000_000).unwrap();
    assert!(balanced.len() < original.len());
    assert!(smallest.len() < balanced.len());
    assert_eq!(count(&smallest).unwrap(), 2);
    assert_eq!(geometry(&smallest, 1).unwrap(), (612.0, 792.0, 0));
    assert!(report.removed.iter().any(|s| s.contains("JPEG")));
    let (again, _) = compress(&smallest).unwrap();
    assert!(again.len() <= smallest.len());
}

#[test]
fn geometry_reports_crop_box_points_and_rotation() {
    let (cropped, _) = crop(&latex_like(2, "geometry"), &[2], [10.0, 20.0, 30.0, 40.0]).unwrap();
    let (turned, _) = rotate(&cropped, &[2], 90).unwrap();
    assert_eq!(geometry(&turned, 2).unwrap(), (572.0, 732.0, 90));
    assert_eq!(geometry(&turned, 1).unwrap(), (612.0, 792.0, 0));
}

/// A PDF shaped like a LaTeX one: N pages, a named destination spelled the way
/// `hyperref` spells them, and a link on page 1 pointing at it.
fn latex_like(pages: usize, marker: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    let mut page_ids = Vec::new();
    for i in 0..pages {
        let content = doc.add_object(Stream::new(
            Dictionary::new(),
            format!("BT /F1 12 Tf 72 720 Td ({marker} page {}) Tj ET", i + 1).into_bytes(),
        ));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set("Contents", content);
        page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        page_ids.push(doc.add_object(page));
    }

    // A link on page 1 to the LAST page, by name -- the shape of a cross
    // reference.
    let mut action = Dictionary::new();
    action.set("S", Object::Name(b"GoTo".to_vec()));
    action.set(
        "D",
        Object::String(b"section.1".to_vec(), lopdf::StringFormat::Literal),
    );
    let mut link = Dictionary::new();
    link.set("Type", Object::Name(b"Annot".to_vec()));
    link.set("Subtype", Object::Name(b"Link".to_vec()));
    link.set("Rect", vec![72.into(), 700.into(), 200.into(), 720.into()]);
    link.set("A", action);
    let link_id = doc.add_object(link);
    if let Ok(p) = doc
        .get_object_mut(page_ids[0])
        .and_then(Object::as_dict_mut)
    {
        p.set("Annots", vec![Object::Reference(link_id)]);
    }

    // The destination table: `section.1` points at the last page.
    let last = *page_ids.last().expect("at least one page");
    let mut dests = Dictionary::new();
    dests.set(
        "Names",
        vec![
            Object::String(b"section.1".to_vec(), lopdf::StringFormat::Literal),
            Object::Array(vec![Object::Reference(last), Object::Name(b"Fit".to_vec())]),
        ],
    );
    let dests_id = doc.add_object(dests);
    let mut names = Dictionary::new();
    names.set("Dests", dests_id);
    let names_id = doc.add_object(names);

    let kids: Vec<Object> = page_ids.iter().map(|id| Object::Reference(*id)).collect();
    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set("Kids", kids);
    tree.set("Count", i64::try_from(pages).unwrap_or(0));
    doc.objects.insert(pages_id, Object::Dictionary(tree));

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("Names", names_id);
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("write the fixture");
    out
}

/// Every destination name a document DEFINES.
fn destination_names(doc: &Document) -> Vec<String> {
    let mut found = Vec::new();
    let tree = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Names").ok())
        .and_then(|n| n.as_reference().ok())
        .and_then(|id| doc.get_object(id).ok())
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Dests").ok())
        .and_then(|d| d.as_reference().ok());
    if let Some(id) = tree {
        if let Ok(names) = doc
            .get_object(id)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"Names"))
            .and_then(Object::as_array)
        {
            for pair in names.chunks(2) {
                if let Some(Object::String(bytes, _)) = pair.first() {
                    found.push(String::from_utf8_lossy(bytes).into_owned());
                }
            }
        }
    }
    found.sort();
    found
}

/// Every destination name a document REFERENCES from a GoTo action.
fn referenced_names(doc: &Document) -> Vec<String> {
    let mut found = Vec::new();
    for obj in doc.objects.values() {
        collect_goto(obj, &mut found);
    }
    found.sort();
    found
}

fn collect_goto(obj: &Object, out: &mut Vec<String>) {
    match obj {
        Object::Dictionary(d) => {
            let is_goto = d
                .get(b"S")
                .ok()
                .and_then(|s| s.as_name().ok())
                .is_some_and(|n| n == b"GoTo");
            if is_goto {
                if let Ok(Object::String(bytes, _)) = d.get(b"D") {
                    out.push(String::from_utf8_lossy(bytes).into_owned());
                }
            }
            for (_, v) in d.iter() {
                collect_goto(v, out);
            }
        }
        Object::Array(a) => {
            for v in a {
                collect_goto(v, out);
            }
        }
        _ => {}
    }
}

/// **The collision, and the reason this module exists.**
///
/// Two documents both defining `section.1` must not end up sharing it. Without
/// namespacing the merged file has one `section.1`, and both documents' links
/// resolve to whichever page won -- silently, with nothing reporting an error.
#[test]
fn merging_two_latex_documents_keeps_each_ones_links_to_itself() {
    let (merged, report) = merge(&[latex_like(3, "A"), latex_like(2, "B")]).expect("merge");

    assert_eq!(report.pages, 5, "every page from both documents");
    assert_eq!(report.links_kept, 2, "one link from each document");

    let doc = Document::load_mem(&merged).expect("reload the merged file");

    let defined = destination_names(&doc);
    assert_eq!(
        defined,
        vec!["d0_section.1".to_string(), "d1_section.1".to_string()],
        "each document's destination must survive under its own name"
    );

    let referenced = referenced_names(&doc);
    assert_eq!(
        referenced,
        vec!["d0_section.1".to_string(), "d1_section.1".to_string()],
        "each link must have been rewritten to its own document's name"
    );

    for name in &referenced {
        assert!(
            defined.contains(name),
            "{name} is referenced but never defined -- that link is dead"
        );
    }
}

/// The simpler failure, checked on its own so a regression says which broke.
#[test]
fn merging_keeps_the_link_annotations() {
    let (merged, _) = merge(&[latex_like(2, "A"), latex_like(2, "B")]).expect("merge");
    let doc = Document::load_mem(&merged).expect("reload");
    let links: usize = doc
        .get_pages()
        .into_values()
        .map(|id| count_links(&doc, id))
        .sum();
    assert_eq!(links, 2, "both Annots arrays must have come across");
}

/// Page order is the order asked for, and every page is still there.
#[test]
fn reordering_permutes_without_losing_a_page() {
    let (out, report) = reorder(&latex_like(4, "A"), &[4, 1, 3, 2]).expect("reorder");
    assert_eq!(report.pages, 4);
    let doc = Document::load_mem(&out).expect("reload");
    assert_eq!(doc.get_pages().len(), 4, "no page may be dropped");
}

/// **Selecting a subset is allowed; duplicating a page is not.**
///
/// This test used to assert the opposite of its first case: that a short order
/// must be refused because it "would silently truncate the document". The
/// reasoning was sound about SILENCE and wrong about who was asking. Every card
/// on the reorder board has a bin, both odd/even shortcuts exist to drop half
/// the pages, and the numbering help says "Anything you leave out is dropped
/// from the output" -- so a short order is the user's explicit request, arriving
/// through three separate controls built to make it.
///
/// The refusal reached them as "That did not produce a file. Nothing was
/// written.", several layers above the real reason, and produced two bug
/// reports: the save failing, and the shortcuts appearing to remove the wrong
/// pages.
///
/// Truncation is no longer silent either way -- the report names how many pages
/// were not kept, and that line reaches the receipt.
#[test]
fn reordering_may_select_a_subset_but_not_repeat_a_page() {
    let pdf = latex_like(3, "A");

    let (out, report) = reorder(&pdf, &[1, 3]).expect("a subset is a selection, not an error");
    assert_eq!(report.pages, 2, "the report counts the OUTPUT's pages");
    let doc = Document::load_mem(&out).expect("reload");
    assert_eq!(doc.get_pages().len(), 2);
    assert!(
        report.removed.iter().any(|r| r.contains("not kept")),
        "dropping a page has to be disclosed: {:?}",
        report.removed
    );

    assert!(
        matches!(reorder(&pdf, &[1, 1, 2]), Err(PageError::BadOrder(_))),
        "a repeated page is a mistake, not a selection: copying a page is not \
         what reordering means"
    );
    assert!(
        matches!(reorder(&pdf, &[]), Err(PageError::BadOrder(_))),
        "a document with no pages is not a document"
    );
    assert!(
        matches!(reorder(&pdf, &[1, 2, 3, 1]), Err(PageError::BadOrder(_))),
        "more entries than the document has pages cannot be a selection"
    );
    assert!(
        matches!(reorder(&pdf, &[1, 2, 9]), Err(PageError::NoSuchPage { .. })),
        "a page that does not exist is named, not clamped"
    );
}

/// One document is not a merge.
#[test]
fn merging_needs_two_documents() {
    assert!(merge(&[latex_like(1, "A")]).is_err());
}

// ---------------------------------------------------------------------------
// Does document metadata survive the object-graph paths?
// ---------------------------------------------------------------------------
//
// The settings screen states, as a fact, that metadata does not carry across a
// conversion. That is easy to believe for the RE-ENCODE paths — an image
// decoded to pixels and re-encoded cannot carry an EXIF block it never read —
// and it is not obvious at all here. `merge` and `reorder` do not rasterise
// anything; they move objects between documents. So the claim has to be
// measured on these two specifically, or it is an assumption printed in the
// interface.

/// A document carrying an `/Info` dictionary: author, title, producer.
fn with_info(pages: usize, author: &str) -> Vec<u8> {
    let mut doc = Document::load_mem(&latex_like(pages, "doc")).expect("fixture reloads");

    let mut info = Dictionary::new();
    info.set("Author", Object::string_literal(author));
    info.set("Title", Object::string_literal("Confidential draft"));
    info.set("Producer", Object::string_literal("SomeCorp Writer 4.2"));
    let info_id = doc.add_object(info);
    doc.trailer.set("Info", info_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("write fixture");
    out
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn has_info(bytes: &[u8]) -> bool {
    Document::load_mem(bytes)
        .map(|d| d.trailer.get(b"Info").is_ok())
        .unwrap_or(false)
}

/// **Merging drops the document metadata, and does not leave it in the bytes.**
///
/// Two separate claims, and the second is the one worth testing. `merge` builds
/// a fresh catalog and a fresh trailer, so no reader will show the author's
/// name — but it also moves every object of every input across, and an
/// unreferenced `/Info` dictionary sitting in the file is still the author's
/// name sitting in the file. "No reader surfaces it" and "it is not there" are
/// different promises, and a privacy claim has to mean the second one.
#[test]
fn merging_does_not_carry_document_metadata() {
    let marker = "JANE-DOE-SECRET";
    let (merged, _) = merge(&[with_info(2, marker), with_info(1, "OTHER-AUTHOR")]).expect("merge");

    assert!(!has_info(&merged), "the merged document must have no /Info");
    assert!(
        !contains(&merged, marker),
        "the author's name is still in the merged BYTES, unreferenced but readable \
         by anyone running `strings` on the file"
    );
}

/// The same question for a reorder, which keeps one document's own trailer.
#[test]
fn reordering_does_not_carry_document_metadata() {
    let marker = "JANE-DOE-SECRET";
    let (out, _) = reorder(&with_info(3, marker), &[3, 2, 1]).expect("reorder");

    assert!(!has_info(&out), "the reordered document must have no /Info");
    assert!(
        !contains(&out, marker),
        "the author's name survived a reorder in the output bytes"
    );
}

// ---------------------------------------------------------------------------
// The rest of what a LaTeX document links with
// ---------------------------------------------------------------------------
//
// `merging_two_latex_documents_keeps_each_ones_links_to_itself` covers ONE
// shape: a `/GoTo` action naming a destination, which is what `\ref` and
// `\cite` compile to. A real hyperref document emits three more, and a merge
// that keeps only the first would lose the table of contents and every
// external citation link while passing that test.
//
//   * `/A << /S /URI >>`      -- \href and \url
//   * `/Dest` ON THE ANNOTATION rather than inside an `/A` action
//   * `/Dest [page /XYZ ...]` -- an ARRAY pointing straight at a page object,
//                                which is what a destination resolves to once
//                                hyperref has no name for it
//   * `/Outlines`             -- the sidebar table of contents
//
// The array case is the one worth being nervous about: it holds a raw object
// reference, so it only survives if the merge renumbers it along with
// everything else.

/// A document with the four link shapes above, plus an outline.
fn hyperref_like(pages: usize, marker: &str, url: &str) -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    let mut page_ids = Vec::new();
    for i in 0..pages {
        let content = doc.add_object(Stream::new(
            Dictionary::new(),
            format!("BT /F1 12 Tf 72 720 Td ({marker} page {}) Tj ET", i + 1).into_bytes(),
        ));
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", pages_id);
        page.set("Contents", content);
        page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
        page_ids.push(doc.add_object(page));
    }
    let last = *page_ids.last().expect("at least one page");

    // 1. \href{url}{...} -- an external link.
    let mut uri_action = Dictionary::new();
    uri_action.set("S", Object::Name(b"URI".to_vec()));
    uri_action.set("URI", Object::string_literal(url));
    let mut uri_link = Dictionary::new();
    uri_link.set("Type", Object::Name(b"Annot".to_vec()));
    uri_link.set("Subtype", Object::Name(b"Link".to_vec()));
    uri_link.set("Rect", vec![72.into(), 700.into(), 300.into(), 716.into()]);
    uri_link.set("A", uri_action);
    let uri_id = doc.add_object(uri_link);

    // 2. /Dest ON THE ANNOTATION, by name, with no /A wrapper.
    let mut bare_dest = Dictionary::new();
    bare_dest.set("Type", Object::Name(b"Annot".to_vec()));
    bare_dest.set("Subtype", Object::Name(b"Link".to_vec()));
    bare_dest.set("Rect", vec![72.into(), 660.into(), 300.into(), 676.into()]);
    bare_dest.set(
        "Dest",
        Object::String(b"section.1".to_vec(), lopdf::StringFormat::Literal),
    );
    let bare_id = doc.add_object(bare_dest);

    // 3. /Dest as an ARRAY straight at the last page -- a raw object reference
    //    that only survives renumbering.
    let mut array_dest = Dictionary::new();
    array_dest.set("Type", Object::Name(b"Annot".to_vec()));
    array_dest.set("Subtype", Object::Name(b"Link".to_vec()));
    array_dest.set("Rect", vec![72.into(), 620.into(), 300.into(), 636.into()]);
    array_dest.set(
        "Dest",
        Object::Array(vec![
            Object::Reference(last),
            Object::Name(b"XYZ".to_vec()),
            Object::Integer(72),
            Object::Integer(720),
            Object::Null,
        ]),
    );
    let array_id = doc.add_object(array_dest);

    if let Ok(page) = doc
        .get_object_mut(page_ids[0])
        .and_then(Object::as_dict_mut)
    {
        page.set(
            "Annots",
            Object::Array(vec![
                Object::Reference(uri_id),
                Object::Reference(bare_id),
                Object::Reference(array_id),
            ]),
        );
    }

    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set(
        "Kids",
        page_ids
            .iter()
            .map(|id| Object::Reference(*id))
            .collect::<Vec<_>>(),
    );
    tree.set("Count", i64::try_from(pages).unwrap_or(0));
    doc.objects.insert(pages_id, Object::Dictionary(tree));

    // 4. The outline: one entry pointing at the last page.
    let outlines_id = doc.new_object_id();
    let mut item = Dictionary::new();
    item.set("Title", Object::string_literal(format!("{marker} chapter")));
    item.set("Parent", outlines_id);
    item.set(
        "Dest",
        Object::Array(vec![
            Object::Reference(last),
            Object::Name(b"XYZ".to_vec()),
            Object::Null,
            Object::Null,
            Object::Null,
        ]),
    );
    let item_id = doc.add_object(item);
    let mut outlines = Dictionary::new();
    outlines.set("Type", Object::Name(b"Outlines".to_vec()));
    outlines.set("First", Object::Reference(item_id));
    outlines.set("Last", Object::Reference(item_id));
    outlines.set("Count", Object::Integer(1));
    doc.objects
        .insert(outlines_id, Object::Dictionary(outlines));

    // The named destination table hyperref writes.
    let mut names = Dictionary::new();
    names.set(
        "Names",
        Object::Array(vec![
            Object::String(b"section.1".to_vec(), lopdf::StringFormat::Literal),
            Object::Array(vec![
                Object::Reference(last),
                Object::Name(b"XYZ".to_vec()),
                Object::Null,
                Object::Null,
                Object::Null,
            ]),
        ]),
    );
    let names_id = doc.add_object(names);
    let mut dests = Dictionary::new();
    dests.set("Dests", Object::Reference(names_id));
    let dests_id = doc.add_object(dests);

    let mut catalog = Dictionary::new();
    catalog.set("Type", "Catalog");
    catalog.set("Pages", pages_id);
    catalog.set("Names", Object::Reference(dests_id));
    catalog.set("Outlines", Object::Reference(outlines_id));
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out).expect("write fixture");
    out
}

/// Every `/Link` annotation in the document, as its dictionary.
fn link_annots(doc: &Document) -> Vec<Dictionary> {
    let mut out = Vec::new();
    for (_, page_id) in doc.get_pages() {
        let annots = doc
            .get_object(page_id)
            .and_then(Object::as_dict)
            .and_then(|d| d.get(b"Annots"))
            .and_then(Object::as_array)
            .cloned()
            .unwrap_or_default();
        for a in annots {
            let dict = match a {
                Object::Reference(id) => doc.get_object(id).and_then(Object::as_dict).ok().cloned(),
                Object::Dictionary(d) => Some(d),
                _ => None,
            };
            if let Some(d) = dict {
                if d.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Link") {
                    out.push(d);
                }
            }
        }
    }
    out
}

/// **External links survive a merge.**
///
/// `\href` and `\url` compile to a `/URI` action, which has nothing to do with
/// the destination table and so is untouched by the namespacing — but "should
/// be untouched" is the sort of thing worth an assertion, because the merge
/// rewrites annotations to fix the internal ones.
#[test]
fn merging_keeps_external_links() {
    let (merged, _) = merge(&[
        hyperref_like(2, "A", "https://example.com/paper-a"),
        hyperref_like(2, "B", "https://example.com/paper-b"),
    ])
    .expect("merge");
    let doc = Document::load_mem(&merged).expect("reload");

    let mut urls: Vec<String> = Vec::new();
    for link in link_annots(&doc) {
        if let Ok(action) = link.get(b"A").and_then(Object::as_dict) {
            if action.get(b"S").and_then(Object::as_name).ok() == Some(b"URI") {
                if let Ok(u) = action.get(b"URI").and_then(Object::as_str) {
                    urls.push(String::from_utf8_lossy(u).into_owned());
                }
            }
        }
    }
    urls.sort();
    assert_eq!(
        urls,
        vec![
            "https://example.com/paper-a".to_string(),
            "https://example.com/paper-b".to_string()
        ],
        "both documents' external links must arrive intact"
    );
}

/// **A `/Dest` array pointing straight at a page still points at that page.**
///
/// This is the shape most at risk. It carries a raw object reference rather
/// than a name, so it survives only because the merge renumbers every id and
/// rewrites every reference. Get that wrong and the link silently lands on
/// whatever object inherited the old number — a page from the OTHER document,
/// most likely, which is the worst kind of wrong: it still works, and it goes
/// somewhere else.
#[test]
fn merging_keeps_direct_page_destinations() {
    let (merged, _) = merge(&[
        hyperref_like(3, "A", "https://example.com/a"),
        hyperref_like(2, "B", "https://example.com/b"),
    ])
    .expect("merge");
    let doc = Document::load_mem(&merged).expect("reload");

    let pages: Vec<ObjectId> = {
        let mut v: Vec<(u32, ObjectId)> = doc.get_pages().into_iter().collect();
        v.sort_by_key(|(n, _)| *n);
        v.into_iter().map(|(_, id)| id).collect()
    };
    assert_eq!(pages.len(), 5, "3 + 2 pages");

    let mut targets: Vec<ObjectId> = Vec::new();
    for link in link_annots(&doc) {
        if let Ok(arr) = link.get(b"Dest").and_then(Object::as_array) {
            if let Some(Object::Reference(id)) = arr.first() {
                targets.push(*id);
            }
        }
    }
    assert_eq!(targets.len(), 2, "one array destination per document");
    assert!(
        targets.iter().all(|t| pages.contains(t)),
        "a direct destination points at an object that is not a page any more"
    );
    assert_eq!(
        targets[0], pages[2],
        "document A's link must still land on A's LAST page (merged page 3)"
    );
    assert_eq!(
        targets[1], pages[4],
        "document B's link must still land on B's last page (merged page 5)"
    );
}

/// **The table of contents survives, with both documents in it.**
///
/// hyperref's `\tableofcontents` becomes `/Outlines`. A merged file with the
/// first document's outline and not the second is a book missing half its
/// contents page.
#[test]
fn merging_keeps_both_outlines() {
    let (merged, _) = merge(&[
        hyperref_like(2, "A", "https://example.com/a"),
        hyperref_like(2, "B", "https://example.com/b"),
    ])
    .expect("merge");
    let doc = Document::load_mem(&merged).expect("reload");

    let outlines_id = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"Outlines").ok())
        .and_then(|o| o.as_reference().ok());
    let outlines_id = outlines_id.expect("the merged document has an outline at all");

    // Walk the sibling chain from /First.
    let mut titles = Vec::new();
    let mut next = doc
        .get_object(outlines_id)
        .and_then(Object::as_dict)
        .and_then(|d| d.get(b"First"))
        .and_then(Object::as_reference)
        .ok();
    while let Some(id) = next {
        let Ok(item) = doc.get_object(id).and_then(Object::as_dict) else {
            break;
        };
        if let Ok(t) = item.get(b"Title").and_then(Object::as_str) {
            titles.push(String::from_utf8_lossy(t).into_owned());
        }
        next = item.get(b"Next").and_then(Object::as_reference).ok();
    }

    assert!(
        titles.iter().any(|t| t.contains('A')) && titles.iter().any(|t| t.contains('B')),
        "both documents' outline entries must be in the merged contents, got {titles:?}"
    );
}

/// **How many pages, answered by the thing that knows.**
///
/// Nothing could answer this before. `probe()` has no document branch, so the
/// desktop's `preview_file` fell through to `_ => 1` and reported ONE page for
/// every PDF ever opened. One wrong number produced three separate bug reports:
/// the reorder board drew a single card, the eye preview showed a single page,
/// and the signing column offered a single sheet.
#[test]
fn count_reports_the_real_page_count() {
    for n in [1_usize, 2, 5, 12] {
        let doc = latex_like(n, "x");
        assert_eq!(
            super::count(&doc).expect("a document we just built parses"),
            n,
            "a {n}-page document must report {n}"
        );
    }
}

/// A file that is not a PDF is refused, not reported as one page.
///
/// The control: the old behaviour returned 1 for everything, so a test that
/// only checked "returns a number" would have passed against the bug.
#[test]
fn count_refuses_something_that_is_not_a_document() {
    assert!(
        super::count(b"this is not a pdf at all").is_err(),
        "a non-document must fail rather than claim a page count"
    );
}

// ---------------------------------------------------------------------------
// Page-range specs
// ---------------------------------------------------------------------------

#[test]
fn a_spec_resolves_singles_ranges_and_open_ends() {
    assert_eq!(resolve_spec("1", 5).unwrap(), vec![1]);
    assert_eq!(resolve_spec("1-3", 5).unwrap(), vec![1, 2, 3]);
    assert_eq!(resolve_spec("1-3,5", 5).unwrap(), vec![1, 2, 3, 5]);
    // Open-ended runs to the last page, which is the one form whose meaning
    // does not depend on guessing.
    assert_eq!(resolve_spec("3-", 5).unwrap(), vec![3, 4, 5]);
    assert_eq!(resolve_spec("5-", 5).unwrap(), vec![5]);
}

#[test]
fn a_spec_tolerates_whitespace_and_stray_commas() {
    assert_eq!(resolve_spec("  1 - 3 ,  5  ", 5).unwrap(), vec![1, 2, 3, 5]);
    assert_eq!(resolve_spec("1,,3", 5).unwrap(), vec![1, 3]);
    assert_eq!(resolve_spec("1,3,", 5).unwrap(), vec![1, 3]);
}

/// Duplicates collapse and the result is sorted.
///
/// Load-bearing rather than cosmetic: `reorder` refuses a repeated page, so a
/// spec of "3,1,1-2" must not reach it as written.
#[test]
fn a_spec_sorts_and_collapses_duplicates() {
    assert_eq!(resolve_spec("3,1,1-2", 5).unwrap(), vec![1, 2, 3]);
    assert_eq!(resolve_spec("2,2,2", 5).unwrap(), vec![2]);
    // And the result is something `reorder` accepts.
    let (out, _) =
        reorder(&latex_like(5, "spec"), &resolve_spec("3,1,1-2", 5).unwrap()).expect("reorder");
    assert_eq!(count(&out).unwrap(), 3);
}

/// Out of range is an error, not a clamp.
#[test]
fn a_spec_past_the_end_names_both_numbers() {
    let err = resolve_spec("9-", 5).unwrap_err().to_string();
    assert!(err.contains('9') && err.contains('5'), "{err}");

    let err = resolve_spec("2-9", 5).unwrap_err().to_string();
    assert!(err.contains('9') && err.contains('5'), "{err}");

    let err = resolve_spec("6", 5).unwrap_err().to_string();
    assert!(err.contains('6') && err.contains('5'), "{err}");
}

#[test]
fn a_spec_refuses_zero_reversed_and_nonsense() {
    for bad in ["0", "0-2", "3-1", "abc", "1-abc", "-3", "", "  ", ","] {
        assert!(
            resolve_spec(bad, 5).is_err(),
            "{bad:?} should not have been accepted"
        );
    }
    // And each refusal says something specific rather than "invalid".
    assert!(resolve_spec("0", 5)
        .unwrap_err()
        .to_string()
        .contains("from 1"));
    assert!(resolve_spec("3-1", 5)
        .unwrap_err()
        .to_string()
        .contains("backwards"));
    assert!(resolve_spec("-3", 5)
        .unwrap_err()
        .to_string()
        .contains("no first page"));
}

#[test]
fn a_spec_against_an_empty_document_is_an_error() {
    assert!(resolve_spec("1", 0).is_err());
}

// ---------------------------------------------------------------------------
// The complement, which is what "remove these pages" means
// ---------------------------------------------------------------------------

#[test]
fn the_complement_is_what_is_left_in_order() {
    assert_eq!(complement_of(&[2, 4], 5).unwrap(), vec![1, 3, 5]);
    assert_eq!(complement_of(&[1], 3).unwrap(), vec![2, 3]);
    // A page named twice, or one outside the document, changes nothing.
    assert_eq!(complement_of(&[2, 2, 9], 3).unwrap(), vec![1, 3]);
}

/// Removing every page is refused, because a PDF with no pages is not a file
/// any reader opens.
#[test]
fn removing_everything_is_refused_by_name() {
    let err = complement_of(&[1, 2, 3], 3).unwrap_err().to_string();
    assert!(err.contains('3') && err.contains("at least one"), "{err}");
}

// ---------------------------------------------------------------------------
// Rotate and crop
// ---------------------------------------------------------------------------

/// Read a page's own `/Rotate`, without following inheritance.
fn own_rotate(bytes: &[u8], page: usize) -> Option<i64> {
    let doc = Document::load_mem(bytes).expect("load");
    let mut nums: Vec<u32> = doc.get_pages().keys().copied().collect();
    nums.sort_unstable();
    let id = *doc.get_pages().get(&nums[page - 1]).expect("page");
    doc.get_object(id)
        .ok()?
        .as_dict()
        .ok()?
        .get(b"Rotate")
        .ok()?
        .as_i64()
        .ok()
}

fn own_cropbox(bytes: &[u8], page: usize) -> Option<Vec<f32>> {
    let doc = Document::load_mem(bytes).expect("load");
    let mut nums: Vec<u32> = doc.get_pages().keys().copied().collect();
    nums.sort_unstable();
    let id = *doc.get_pages().get(&nums[page - 1]).expect("page");
    let array = doc
        .get_object(id)
        .ok()?
        .as_dict()
        .ok()?
        .get(b"CropBox")
        .ok()?
        .as_array()
        .ok()?;
    Some(
        array
            .iter()
            .map(|o| match o {
                Object::Integer(i) => *i as f32,
                Object::Real(r) => *r,
                _ => f32::NAN,
            })
            .collect(),
    )
}

/// Set an attribute on every PAGE dictionary.
///
/// `latex_like` gives each page its own `/MediaBox`, and a page's own value
/// wins over the tree's — so a fixture that set the box on the tree would be
/// testing inheritance rather than the geometry, and quietly measuring against
/// the fixture's default page size instead.
fn set_on_pages(bytes: &[u8], key: &str, value: &Object) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).expect("load");
    let ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for id in ids {
        doc.get_object_mut(id)
            .expect("page")
            .as_dict_mut()
            .expect("dict")
            .set(key, value.clone());
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

/// Remove an attribute from every PAGE, leaving only what the tree provides.
fn clear_on_pages(bytes: &[u8], key: &[u8]) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).expect("load");
    let ids: Vec<ObjectId> = doc.get_pages().values().copied().collect();
    for id in ids {
        doc.get_object_mut(id)
            .expect("page")
            .as_dict_mut()
            .expect("dict")
            .remove(key);
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

/// Set an attribute on the shared `/Pages` node, which every page inherits.
fn set_on_tree(bytes: &[u8], key: &str, value: Object) -> Vec<u8> {
    let mut doc = Document::load_mem(bytes).expect("load");
    let pages_id = doc
        .catalog()
        .expect("catalog")
        .get(b"Pages")
        .expect("pages")
        .as_reference()
        .expect("ref");
    doc.get_object_mut(pages_id)
        .expect("node")
        .as_dict_mut()
        .expect("dict")
        .set(key, value);
    let mut out = Vec::new();
    doc.save_to(&mut out).expect("save");
    out
}

/// Rotation is RELATIVE, and the existing angle may be inherited.
///
/// The trap this guards: a page with no `/Rotate` of its own can still be
/// rotated by its parent. Reading the page dictionary alone gives 0, and
/// "rotate by 90" then writes 90 — silently *un*-rotating a document whose
/// pages were all turned at the tree level.
#[test]
fn rotation_adds_to_an_inherited_angle() {
    let base = latex_like(2, "rot");
    // Every page inherits 90 from the tree, and none carries its own.
    let doc = set_on_tree(&base, "Rotate", Object::Integer(90));
    assert_eq!(
        own_rotate(&doc, 1),
        None,
        "the fixture must inherit, not own"
    );

    let (out, _) = rotate(&doc, &[1], 90).expect("rotate");
    assert_eq!(
        own_rotate(&out, 1),
        Some(180),
        "90 inherited plus a 90 turn is 180, not 90"
    );
}

/// Angles normalise into the four legal values, from anywhere.
#[test]
fn rotation_normalises_negative_and_oversized_angles() {
    let base = latex_like(1, "rot");
    for (turn, want) in [
        (90, 90),
        (180, 180),
        (270, 270),
        (360, 0),
        (450, 90),
        (-90, 270),
        (-450, 270),
    ] {
        let (out, _) = rotate(&base, &[1], turn).expect("rotate");
        assert_eq!(own_rotate(&out, 1), Some(want), "turning by {turn}");
    }
}

#[test]
fn rotation_refuses_a_turn_that_is_not_a_quarter() {
    let base = latex_like(1, "rot");
    let err = rotate(&base, &[1], 45).unwrap_err().to_string();
    assert!(err.contains("quarter") || err.contains("90"), "{err}");
}

#[test]
fn rotation_touches_only_the_pages_named() {
    let base = latex_like(3, "rot");
    let (out, _) = rotate(&base, &[2], 90).expect("rotate");
    assert_eq!(own_rotate(&out, 1), None);
    assert_eq!(own_rotate(&out, 2), Some(90));
    assert_eq!(own_rotate(&out, 3), None);
}

/// A crop is relative to the page's existing box, wherever that box starts.
///
/// The trap: `/MediaBox` does not have to begin at the origin. Treating the
/// inset as an absolute coordinate would crop the wrong region entirely on a
/// scanned or imposed document.
#[test]
fn cropping_is_relative_to_a_non_zero_origin() {
    let base = latex_like(1, "crop");
    // A page running from (100,100) to (400,700).
    let doc = set_on_pages(
        &base,
        "MediaBox",
        &Object::Array(vec![
            Object::Integer(100),
            Object::Integer(100),
            Object::Integer(400),
            Object::Integer(700),
        ]),
    );

    let (out, _) = crop(&doc, &[1], [10.0, 20.0, 30.0, 40.0]).expect("crop");
    let got = own_cropbox(&out, 1).expect("cropbox");
    assert_eq!(
        got,
        vec![110.0, 120.0, 370.0, 660.0],
        "insets are from the existing box"
    );
}

/// The crop box is clamped to the media box, per the specification.
#[test]
fn cropping_cannot_grow_past_the_media_box() {
    let base = latex_like(1, "crop");
    let doc = set_on_pages(
        &base,
        "MediaBox",
        &Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(200),
            Object::Integer(200),
        ]),
    );
    // Negative insets ask for a box larger than the page.
    let (out, _) = crop(&doc, &[1], [-50.0, -50.0, -50.0, -50.0]).expect("crop");
    let got = own_cropbox(&out, 1).expect("cropbox");
    assert_eq!(
        got,
        vec![0.0, 0.0, 200.0, 200.0],
        "clamped to the media box"
    );
}

#[test]
fn cropping_away_the_whole_page_is_refused() {
    let base = latex_like(1, "crop");
    let doc = set_on_pages(
        &base,
        "MediaBox",
        &Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(200),
            Object::Integer(200),
        ]),
    );
    let err = crop(&doc, &[1], [150.0, 0.0, 150.0, 0.0])
        .unwrap_err()
        .to_string();
    assert!(err.contains("leaves nothing"), "{err}");
}

/// `/MediaBox` is left alone, so the crop is reversible.
#[test]
fn cropping_does_not_touch_the_media_box() {
    let base = latex_like(1, "crop");
    let doc = set_on_pages(
        &base,
        "MediaBox",
        &Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(200),
            Object::Integer(200),
        ]),
    );
    let (out, _) = crop(&doc, &[1], [10.0, 10.0, 10.0, 10.0]).expect("crop");
    let after = Document::load_mem(&out).expect("load");
    let mut nums: Vec<u32> = after.get_pages().keys().copied().collect();
    nums.sort_unstable();
    let id = *after.get_pages().get(&nums[0]).expect("page");
    let media = after
        .get_object(id)
        .expect("obj")
        .as_dict()
        .expect("dict")
        .get(b"MediaBox")
        .expect("the media box must still be there")
        .as_array()
        .expect("array")
        .iter()
        .map(|o| o.as_i64().unwrap_or(-1))
        .collect::<Vec<_>>();
    assert_eq!(media, vec![0, 0, 200, 200], "the media box was modified");
    // And the crop landed where it should.
    assert_eq!(
        own_cropbox(&out, 1).expect("cropbox"),
        vec![10.0, 10.0, 190.0, 190.0]
    );
}

/// And the inheritance path, which is the other half of the same trap.
///
/// A page with no `/MediaBox` of its own takes the tree's. Reading only the
/// page dictionary would find nothing and refuse a perfectly ordinary
/// document.
#[test]
fn cropping_follows_an_inherited_media_box() {
    let base = latex_like(1, "crop");
    let stripped = clear_on_pages(&base, b"MediaBox");
    let doc = set_on_tree(
        &stripped,
        "MediaBox",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(500),
            Object::Integer(500),
        ]),
    );
    let (out, _) = crop(&doc, &[1], [25.0, 25.0, 25.0, 25.0]).expect("crop");
    assert_eq!(
        own_cropbox(&out, 1).expect("cropbox"),
        vec![25.0, 25.0, 475.0, 475.0]
    );
}

#[test]
fn a_page_outside_the_document_is_refused_by_both() {
    let base = latex_like(2, "oops");
    assert!(rotate(&base, &[5], 90).is_err());
    assert!(crop(&base, &[5], [1.0, 1.0, 1.0, 1.0]).is_err());
}

#[test]
fn geometry_resolves_indirect_inherited_boxes_and_rotation() {
    let mut doc = Document::load_mem(&latex_like(1, "indirect")).unwrap();
    let page = doc.get_pages()[&1];
    let parent = doc
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let x = doc.add_object(Object::Integer(612));
    let box_id = doc.add_object(Object::Array(vec![
        0.into(),
        0.into(),
        Object::Reference(x),
        792.into(),
    ]));
    let rotation = doc.add_object(Object::Integer(90));
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .remove(b"MediaBox");
    let dict = doc.get_object_mut(parent).unwrap().as_dict_mut().unwrap();
    dict.set("MediaBox", box_id);
    dict.set("Rotate", rotation);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert_eq!(geometry(&bytes, 1).unwrap(), (612.0, 792.0, 90));
}

#[test]
fn compression_downsamples_large_images_and_preserves_text() {
    let mut doc = Document::load_mem(&latex_like(1, "readable text")).unwrap();
    let image = image::RgbImage::from_fn(2400, 3200, |x, y| {
        image::Rgb([(x * 3 + y) as u8, (x + y * 2) as u8, (x * y) as u8])
    });
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&image)
        .unwrap();
    let mut dict = Dictionary::new();
    dict.set("Type", "XObject");
    dict.set("Subtype", "Image");
    dict.set("Filter", "DCTDecode");
    dict.set("ColorSpace", "DeviceRGB");
    dict.set("Width", 2400);
    dict.set("Height", 3200);
    dict.set("BitsPerComponent", 8);
    let id = doc.add_object(Stream::new(dict, jpeg));
    let page = doc.get_pages()[&1];
    let mut xobjects = Dictionary::new();
    xobjects.set("Photo", id);
    let mut resources = Dictionary::new();
    resources.set("XObject", xobjects);
    let mut font = Dictionary::new();
    font.set("Type", "Font");
    font.set("Subtype", "Type1");
    font.set("BaseFont", "Helvetica");
    let font_id = doc.add_object(font);
    let mut fonts = Dictionary::new();
    fonts.set("F1", font_id);
    resources.set("Font", fonts);
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Resources", resources);
    let mut content = b"q 612 0 0 792 0 0 cm /Photo Do Q\n".to_vec();
    content.extend_from_slice(&doc.get_page_content(page));
    let content_id = doc.add_object(Stream::new(Dictionary::new(), content));
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", content_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    let (out, _) = compress_with_quality(&bytes, Some(45), 40_000_000).unwrap();
    let after = Document::load_mem(&out).unwrap();
    let image = after.get_object(id).unwrap().as_stream().unwrap();
    assert!(image.dict.get(b"Height").unwrap().as_i64().unwrap() <= 1320);
    assert!(out.len() < bytes.len() / 3);
    assert_eq!(geometry(&out, 1).unwrap(), (612.0, 792.0, 0));
    assert!(String::from_utf8_lossy(&after.get_page_content(page)).contains("readable text"));
    eprintln!("Large image PDF: {} -> {} bytes", bytes.len(), out.len());
}
