//! Place an image on a PDF page — a signature, a logo, a watermark.
//!
//! # Transparency is the whole difficulty
//!
//! A PDF image XObject has no alpha channel. Transparency is expressed by a
//! SECOND image, the soft mask (`/SMask`), which is a greyscale image the same
//! size whose values say how opaque each pixel of the first one is. An
//! implementation that embeds only the colour channels produces a signature
//! sitting in an opaque white rectangle, which is exactly the outcome someone
//! asking to "add a transparent PNG" is trying to avoid.
//!
//! So this splits the decoded RGBA into two streams and writes both. The test
//! asserts the `/SMask` is present and that its values match the source alpha,
//! because a stamp that is subtly opaque looks fine until it is over text.
//!
//! # Why not `lopdf`'s `embed_image`
//!
//! It would do the decode for us and pulls a second image-decoding stack into a
//! worker whose entire job is parsing untrusted bytes. This crate already links
//! `image`; using it costs nothing and keeps the alpha handling here, where it
//! can be read.
//!
//! # Coordinates
//!
//! PDF user space has its origin at the BOTTOM-left of the page and is measured
//! in points (1/72 inch). `x` and `y` name the lower-left corner of the placed
//! image, so a stamp at `y = 72` sits one inch up from the bottom edge — which
//! is where a signature usually goes.

use lopdf::{Dictionary, Document, Object, Stream};

use crate::pages::PageError;

/// Where and how big.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    /// One-based page number.
    pub page: usize,
    /// Distance from the left edge, in points.
    pub x: f32,
    /// Distance from the bottom edge, in points.
    pub y: f32,
    /// Width on the page, in points. Height follows the image's aspect ratio,
    /// because a stamp stretched out of shape is never what was meant.
    pub width: f32,
}

impl Default for Placement {
    fn default() -> Self {
        // One inch in from the left, one inch up, two inches wide: a signature
        // on the bottom-left of a page, which is the common case.
        Self {
            page: 1,
            x: 72.0,
            y: 72.0,
            width: 144.0,
        }
    }
}

/// Draw `image_bytes` onto one page of `pdf_bytes`.
///
/// # Errors
///
/// [`PageError`] — an unreadable PDF, an unreadable image, or a page that does
/// not exist.
pub fn stamp(
    pdf_bytes: &[u8],
    image_bytes: &[u8],
    at: Placement,
) -> Result<(Vec<u8>, crate::pages::Report), PageError> {
    let mut doc = Document::load_mem(pdf_bytes).map_err(|e| PageError::Parse(e.to_string()))?;

    let pages = doc.get_pages();
    let have = pages.len();
    let page_id = pages
        .get(&u32::try_from(at.page).unwrap_or(u32::MAX))
        .copied()
        .ok_or(PageError::NoSuchPage {
            asked: at.page,
            have,
        })?;

    let img = image::load_from_memory(image_bytes)
        .map_err(|e| PageError::Parse(format!("could not read the stamp image: {e}")))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(PageError::Parse("the stamp image has no pixels".into()));
    }

    // Split RGBA into colour and opacity. Two streams, because that is what a
    // PDF understands: the second one IS the transparency.
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    let mut any_transparency = false;
    for px in img.pixels() {
        rgb.extend_from_slice(&px.0[..3]);
        alpha.push(px.0[3]);
        if px.0[3] != 255 {
            any_transparency = true;
        }
    }

    let mut smask_dict = Dictionary::new();
    smask_dict.set("Type", Object::Name(b"XObject".to_vec()));
    smask_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    smask_dict.set("Width", i64::from(w));
    smask_dict.set("Height", i64::from(h));
    smask_dict.set("ColorSpace", Object::Name(b"DeviceGray".to_vec()));
    smask_dict.set("BitsPerComponent", 8_i64);
    let mut smask = Stream::new(smask_dict, alpha);
    smask
        .compress()
        .map_err(|e| PageError::Write(e.to_string()))?;
    let smask_id = doc.add_object(smask);

    let mut image_dict = Dictionary::new();
    image_dict.set("Type", Object::Name(b"XObject".to_vec()));
    image_dict.set("Subtype", Object::Name(b"Image".to_vec()));
    image_dict.set("Width", i64::from(w));
    image_dict.set("Height", i64::from(h));
    image_dict.set("ColorSpace", Object::Name(b"DeviceRGB".to_vec()));
    image_dict.set("BitsPerComponent", 8_i64);
    image_dict.set("SMask", smask_id);
    let mut image_obj = Stream::new(image_dict, rgb);
    image_obj
        .compress()
        .map_err(|e| PageError::Write(e.to_string()))?;
    let image_id = doc.add_object(image_obj);

    // A name unlikely to collide with one the document already uses. A clash
    // would silently replace the document's own image with ours.
    let name = b"OpenConvertStamp".to_vec();
    add_xobject_to_resources(&mut doc, page_id, &name, image_id)?;

    // Height from the aspect ratio: the caller names one dimension and the
    // image decides the other.
    #[allow(clippy::cast_precision_loss)]
    let height = at.width * (h as f32) / (w as f32);

    // `q`/`Q` save and restore the graphics state, so the matrix set here
    // cannot leak into whatever the page draws afterwards. `cm` scales the
    // unit square the image occupies up to the size we want and moves it into
    // place; `Do` paints it.
    let content = format!(
        "\nq\n{width:.2} 0 0 {height:.2} {x:.2} {y:.2} cm\n/{n} Do\nQ\n",
        width = at.width,
        height = height,
        x = at.x,
        y = at.y,
        n = String::from_utf8_lossy(&name)
    );
    append_to_content(&mut doc, page_id, content.as_bytes())?;

    // The same privacy rule as merge and reorder: placing an image on a page
    // does not rasterise the document, so nothing about the operation loses the
    // author's name unless it is taken out deliberately.
    let stripped = crate::pages::strip_document_metadata(&mut doc);

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PageError::Write(e.to_string()))?;

    let mut report = crate::pages::Report {
        pages: have,
        ..crate::pages::Report::default()
    };
    report.removed.extend(stripped);
    report.removed.push(if any_transparency {
        "nothing; the stamp was drawn over the page with its transparency intact".to_string()
    } else {
        "nothing; the stamp image is fully opaque and was drawn as given".to_string()
    });
    Ok((out, report))
}

/// Register the image under `name` in the page's `/Resources /XObject`.
///
/// Resources may be inherited from the page tree, so a page with no dictionary
/// of its own gets one rather than having the parent's mutated — mutating the
/// parent would stamp every page that inherits from it.
fn add_xobject_to_resources(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    name: &[u8],
    image_id: lopdf::ObjectId,
) -> Result<(), PageError> {
    let existing = doc
        .get_object(page_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Resources").ok())
        .cloned();

    let mut resources = match existing {
        Some(Object::Dictionary(d)) => d,
        Some(Object::Reference(id)) => doc
            .get_object(id)
            .ok()
            .and_then(|o| o.as_dict().ok())
            .cloned()
            .unwrap_or_default(),
        _ => Dictionary::new(),
    };

    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .and_then(|x| x.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    xobjects.set(name.to_vec(), image_id);
    resources.set("XObject", xobjects);

    // Written back onto the PAGE, always as its own dictionary.
    if let Ok(page) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
        page.set("Resources", resources);
        Ok(())
    } else {
        Err(PageError::Write("the page is not a dictionary".into()))
    }
}

/// Append drawing operators after everything the page already draws.
///
/// A page's `/Contents` may be one stream or an array of them; the PDF reader
/// concatenates an array before interpreting it, so adding one more entry is
/// the least invasive way to draw on top. Rewriting the existing stream would
/// mean decompressing and re-serialising content this code has no reason to
/// touch.
fn append_to_content(
    doc: &mut Document,
    page_id: lopdf::ObjectId,
    operators: &[u8],
) -> Result<(), PageError> {
    let added = doc.add_object(Stream::new(Dictionary::new(), operators.to_vec()));

    let current = doc
        .get_object(page_id)
        .ok()
        .and_then(|o| o.as_dict().ok())
        .and_then(|d| d.get(b"Contents").ok())
        .cloned();

    let next = match current {
        Some(Object::Array(mut items)) => {
            items.push(Object::Reference(added));
            Object::Array(items)
        }
        Some(existing @ Object::Reference(_)) => {
            Object::Array(vec![existing, Object::Reference(added)])
        }
        _ => Object::Array(vec![Object::Reference(added)]),
    };

    if let Ok(page) = doc.get_object_mut(page_id).and_then(Object::as_dict_mut) {
        page.set("Contents", next);
        Ok(())
    } else {
        Err(PageError::Write("the page is not a dictionary".into()))
    }
}

#[cfg(test)]
mod tests;
