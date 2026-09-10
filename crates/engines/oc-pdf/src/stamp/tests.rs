//! What a stamp owes a transparent PNG.
//!
//! The assertions are about the PDF's *structure* rather than about rendered
//! pixels, because the failure being guarded against is structural: an image
//! embedded without its soft mask produces a perfectly valid PDF that renders
//! a signature inside an opaque rectangle. Only the presence and content of
//! `/SMask` distinguishes the two, so that is what is checked.

use super::*;

/// A one-page PDF with a content stream, so the append path has something to
/// append to.
fn one_page_pdf() -> Vec<u8> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content = doc.add_object(Stream::new(
        Dictionary::new(),
        b"BT /F1 12 Tf 72 720 Td (hello) Tj ET".to_vec(),
    ));
    let mut page = Dictionary::new();
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    page.set("Contents", content);
    page.set("MediaBox", vec![0.into(), 0.into(), 612.into(), 792.into()]);
    let page_id = doc.add_object(page);

    let mut tree = Dictionary::new();
    tree.set("Type", "Pages");
    tree.set("Kids", vec![Object::Reference(page_id)]);
    tree.set("Count", 1_i64);
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

/// A 2x2 PNG: one opaque pixel, one fully transparent, two in between.
fn transparent_png() -> Vec<u8> {
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

/// Find the stamp's image XObject and return `(image dict, smask stream data)`.
fn stamped_image(doc: &Document) -> (Dictionary, Vec<u8>) {
    let page_id = *doc
        .get_pages()
        .values()
        .next()
        .expect("the document has a page");
    let resources = doc
        .get_object(page_id)
        .and_then(Object::as_dict)
        .and_then(|d| d.get(b"Resources"))
        .and_then(Object::as_dict)
        .expect("the page has resources");
    let xobjects = resources
        .get(b"XObject")
        .and_then(Object::as_dict)
        .expect("resources carry an XObject dictionary");
    let image_id = xobjects
        .get(b"OpenConvertStamp")
        .and_then(Object::as_reference)
        .expect("the stamp is registered under its name");
    let stream = doc
        .get_object(image_id)
        .and_then(Object::as_stream)
        .expect("the stamp is a stream");
    let smask_id = stream
        .dict
        .get(b"SMask")
        .and_then(Object::as_reference)
        .expect("THE SOFT MASK IS MISSING: the stamp would render fully opaque");
    let smask = doc
        .get_object(smask_id)
        .and_then(Object::as_stream)
        .expect("the soft mask is a stream")
        .decompressed_content()
        .expect("the soft mask decompresses");
    (stream.dict.clone(), smask)
}

/// **The transparency property.** The mask must carry the source alpha.
#[test]
fn a_transparent_png_keeps_its_transparency() {
    let (out, report) =
        stamp(&one_page_pdf(), &transparent_png(), Placement::default()).expect("stamp");
    let doc = Document::load_mem(&out).expect("reload");
    let (dict, smask) = stamped_image(&doc);

    assert_eq!(
        dict.get(b"ColorSpace")
            .and_then(Object::as_name)
            .expect("a colour space"),
        b"DeviceRGB",
        "the colour channels go in as RGB; alpha rides the mask"
    );
    assert_eq!(
        smask,
        vec![255, 0, 128, 64],
        "the soft mask must be the source alpha, in row order"
    );
    assert!(
        report.removed.iter().any(|r| r.contains("transparency")),
        "the receipt should say the transparency survived: {:?}",
        report.removed
    );
}

/// The page still draws what it drew before; the stamp is added, not swapped in.
#[test]
fn stamping_appends_rather_than_replacing_the_page() {
    let (out, _) = stamp(&one_page_pdf(), &transparent_png(), Placement::default()).expect("stamp");
    let doc = Document::load_mem(&out).expect("reload");
    let page_id = *doc.get_pages().values().next().expect("a page");
    let contents = doc
        .get_object(page_id)
        .and_then(Object::as_dict)
        .and_then(|d| d.get(b"Contents"))
        .and_then(Object::as_array)
        .expect("contents became an array");
    assert_eq!(contents.len(), 2, "the original stream must still be there");

    let mut text = Vec::new();
    for item in contents {
        if let Ok(id) = item.as_reference() {
            if let Ok(stream) = doc.get_object(id).and_then(Object::as_stream) {
                text.extend_from_slice(&stream.decompressed_content().unwrap_or_default());
            }
        }
    }
    let joined = String::from_utf8_lossy(&text);
    assert!(joined.contains("hello"), "the page's own drawing survived");
    assert!(
        joined.contains("/OpenConvertStamp Do"),
        "the stamp is drawn"
    );
    assert!(
        joined.contains('q') && joined.contains('Q'),
        "the stamp must save and restore the graphics state so it cannot leak"
    );
}

/// A page that does not exist is named, not clamped to the first one.
#[test]
fn a_missing_page_is_refused() {
    let at = Placement {
        page: 9,
        ..Placement::default()
    };
    assert!(matches!(
        stamp(&one_page_pdf(), &transparent_png(), at),
        Err(PageError::NoSuchPage { asked: 9, have: 1 })
    ));
}

/// Something that is not an image is refused with a message that says so.
#[test]
fn a_non_image_is_refused() {
    let err = stamp(&one_page_pdf(), b"this is not a png", Placement::default())
        .expect_err("arbitrary bytes are not a stamp");
    assert!(err.to_string().contains("stamp image"));
}
