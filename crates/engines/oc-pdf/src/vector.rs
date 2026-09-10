//! Drawing paths onto a page.
//!
//! # Why this is separate from `typeset.rs`
//!
//! `typeset` lays TEXT out: it owns line breaking, a font, and the decision
//! about where a paragraph ends. This owns none of that and needs none of it —
//! a drawing is already positioned, and the only questions are what scale puts
//! it on the page and how the curves are written into a content stream.
//!
//! They share `lopdf` and nothing else, which is the right amount.
//!
//! # The page is fitted, not cropped
//!
//! A DXF states no page and no units that this build trusts: `$INSUNITS` says
//! millimetres in the files spiked, and says nothing at all in plenty of
//! others. So the drawing is scaled to fit A4 with a margin, uniformly in both
//! axes — a drawing squashed to fill the page would be a drawing whose
//! measurements are wrong, which on a cutting file is the whole content.
//!
//! The scale factor goes in the receipt. It is the one number somebody would
//! need to get their millimetres back.

use lopdf::{dictionary, Document, Object, Stream};

use crate::dxf::Drawing;

/// A4 in PDF points, which is the page every other writer here produces.
const PAGE_W: f64 = 595.0;
const PAGE_H: f64 = 842.0;
/// The margin, in points. Half an inch.
const MARGIN: f64 = 36.0;

/// Stroke width, in points. Thin enough not to hide a narrow slot, thick
/// enough to see.
const LINE_WIDTH: f64 = 0.5;

/// What placing the drawing on the page did to it.
pub struct Placement {
    /// Points per drawing unit.
    pub scale: f64,
}

/// Draw a `Drawing` as a one-page PDF.
///
/// # Errors
///
/// A drawing with no extent — every point in the same place — has no scale that
/// fits it, and `lopdf`'s own save errors.
pub fn write(drawing: &Drawing) -> Result<(Vec<u8>, Placement), String> {
    let (x0, y0, x1, y1) = drawing
        .bounds()
        .ok_or("this drawing has no geometry to place on a page")?;
    let (w, h) = (x1 - x0, y1 - y0);
    if !(w.is_finite() && h.is_finite()) || (w <= 0.0 && h <= 0.0) {
        return Err(
            "this drawing has no extent: every point in it is in the same place".to_string(),
        );
    }

    // UNIFORM, so the drawing keeps its proportions. The smaller of the two
    // ratios is what fits both axes.
    let sx = if w > 0.0 {
        (PAGE_W - 2.0 * MARGIN) / w
    } else {
        f64::MAX
    };
    let sy = if h > 0.0 {
        (PAGE_H - 2.0 * MARGIN) / h
    } else {
        f64::MAX
    };
    let scale = sx.min(sy);
    if !scale.is_finite() || scale <= 0.0 {
        return Err("this drawing cannot be scaled onto a page".to_string());
    }

    // Centred in whatever room is left over.
    let ox = (PAGE_W - w * scale) / 2.0 - x0 * scale;
    let oy = (PAGE_H - h * scale) / 2.0 - y0 * scale;

    let mut content = String::with_capacity(drawing.paths.len() * 64);
    // Black, butt caps, round joins: a cut line, not a decoration.
    content.push_str(&format!("{LINE_WIDTH} w 0 G 0 J 1 j\n"));
    for path in &drawing.paths {
        let mut points = path.points.iter();
        let Some(first) = points.next() else { continue };
        content.push_str(&format!(
            "{:.3} {:.3} m\n",
            first.x.mul_add(scale, ox),
            first.y.mul_add(scale, oy)
        ));
        for p in points {
            content.push_str(&format!(
                "{:.3} {:.3} l\n",
                p.x.mul_add(scale, ox),
                p.y.mul_add(scale, oy)
            ));
        }
        // `h` closes the subpath, which is what makes a closed polyline's last
        // segment appear at all — without it the outline has a gap exactly
        // where the shape should meet itself.
        if path.closed {
            content.push_str("h\n");
        }
        content.push_str("S\n");
    }

    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), PAGE_W.into(), PAGE_H.into()],
        "Contents" => content_id,
        // No fonts, no images, no external objects: a drawing is strokes.
        "Resources" => dictionary! {},
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| format!("could not write the PDF: {e}"))?;
    Ok((out, Placement { scale }))
}

/// The same drawing as SVG.
///
/// # Why both
///
/// The geometry is the whole job and it is already done; the emitters are forty
/// lines each. A PDF is what somebody prints or sends; an SVG is what they open
/// in a browser or edit — and for a cutting file the SVG keeps the drawing's
/// own units, where the PDF has to scale it onto a page.
#[must_use]
pub fn to_svg(drawing: &Drawing) -> String {
    let (x0, y0, x1, y1) = drawing.bounds().unwrap_or((0.0, 0.0, 1.0, 1.0));
    let (w, h) = ((x1 - x0).max(1e-9), (y1 - y0).max(1e-9));

    let mut out = String::with_capacity(drawing.paths.len() * 64);
    out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{x0:.4} {y0:.4} {w:.4} {h:.4}\" \
         width=\"{w:.4}\" height=\"{h:.4}\">\n"
    ));
    // SVG's y axis runs DOWN and a drawing's runs up, so the whole picture is
    // flipped about its own vertical middle. Without this every drawing comes
    // out mirrored, which on a cutting file is a part that will not fit.
    out.push_str(&format!(
        "<g transform=\"translate(0,{:.4}) scale(1,-1)\" fill=\"none\" stroke=\"black\" \
         stroke-width=\"{:.4}\">\n",
        y0 + y1,
        w.max(h) / 1000.0
    ));
    for path in &drawing.paths {
        let mut points = path.points.iter();
        let Some(first) = points.next() else { continue };
        out.push_str(&format!("<path d=\"M {:.4} {:.4}", first.x, first.y));
        for p in points {
            out.push_str(&format!(" L {:.4} {:.4}", p.x, p.y));
        }
        if path.closed {
            out.push_str(" Z");
        }
        out.push_str("\"/>\n");
    }
    out.push_str("</g>\n</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dxf::{Path, Point};

    fn square() -> Drawing {
        Drawing {
            paths: vec![Path {
                points: vec![
                    Point { x: 0.0, y: 0.0 },
                    Point { x: 10.0, y: 0.0 },
                    Point { x: 10.0, y: 10.0 },
                    Point { x: 0.0, y: 10.0 },
                ],
                closed: true,
            }],
            skipped: Vec::new(),
        }
    }

    #[test]
    fn a_drawing_becomes_a_one_page_pdf() {
        let (bytes, place) = write(&square()).expect("write");
        assert!(bytes.starts_with(b"%PDF"), "not a PDF");
        // Ten units across a 523-point printable width.
        assert!(
            (place.scale - 52.3).abs() < 0.1,
            "scale {} is not the fit",
            place.scale
        );
        // It reopens, and it has exactly one page.
        let doc = lopdf::Document::load_mem(&bytes).expect("reopen");
        assert_eq!(doc.get_pages().len(), 1);
    }

    /// **Proportions are kept.** A drawing squashed to fill the page is one
    /// whose measurements are wrong, which for a cutting file is the content.
    #[test]
    fn a_wide_drawing_is_scaled_uniformly_not_stretched() {
        let wide = Drawing {
            paths: vec![Path {
                points: vec![Point { x: 0.0, y: 0.0 }, Point { x: 100.0, y: 1.0 }],
                closed: false,
            }],
            skipped: Vec::new(),
        };
        let (_, place) = write(&wide).expect("write");
        // 100 units across the printable width, and the height must not get
        // its own larger factor.
        assert!(
            (place.scale - (PAGE_W - 2.0 * MARGIN) / 100.0).abs() < 1e-9,
            "scale {} came from the wrong axis",
            place.scale
        );
    }

    /// SVG's y axis runs down. Without the flip every drawing is mirrored —
    /// a part that will not fit, and one that looks fine in a thumbnail.
    #[test]
    fn the_svg_is_flipped_so_the_drawing_is_not_mirrored() {
        let svg = to_svg(&square());
        assert!(svg.starts_with("<svg"), "{svg}");
        assert!(svg.contains("scale(1,-1)"), "no flip: {svg}");
        assert!(
            svg.contains("viewBox=\"0.0000 0.0000 10.0000 10.0000\""),
            "{svg}"
        );
        assert!(svg.contains(" Z\"/>"), "a closed path must close: {svg}");
    }

    #[test]
    fn a_drawing_with_no_extent_is_refused() {
        let dot = Drawing {
            paths: vec![Path {
                points: vec![Point { x: 5.0, y: 5.0 }, Point { x: 5.0, y: 5.0 }],
                closed: false,
            }],
            skipped: Vec::new(),
        };
        assert!(write(&dot).is_err(), "a point is not a drawing");
    }
}
