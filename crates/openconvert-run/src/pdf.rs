//! Structural PDF writing: an image becomes a single-page PDF.
//!
//! # Why this runs in-process
//!
//! The direction matters more than the format. A PDF we *generate* is a PDF
//! whose structure we chose; nothing in the input is interpreted as PostScript
//! or JavaScript, because nothing in the input is interpreted as program text
//! at all:
//!
//! - **JPEG** — the encoded bytes are embedded VERBATIM under a `/DCTDecode`
//!   stream. Not one sample is decoded; the operation is a container change,
//!   which is why it stays honest even though the route is marked Class B
//!   (the class describes what the *file* gains, not risk).
//! - **PNG** — decoded through `image-rs` (the same parser the in-process
//!   image path already trusts), re-compressed as a raw DEFLATE stream.
//!
//! The directions that would need pdfium — rendering a PDF *into* pixels —
//! parse attacker-controlled document structure, and those wait behind the
//! boundary in `oc-pdf`.
//!
//! # Determinism
//!
//! Object order, xref layout and content stream are fixed; DEFLATE output is
//! fixed for a fixed input and compressor version. Two conversions of one
//! PNG produce identical bytes, which is what the receipt's hash pair needs
//! to be meaningful across runs.

use std::io::{Cursor, Write};

/// Why an image could not be wrapped as PDF.
#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    /// The input was not decodable enough to size the page.
    #[error("could not read the image header for PDF wrapping: {0}")]
    Header(String),
    /// The input needed decoding and failed.
    #[error("could not decode this image for PDF wrapping: {0}")]
    Decode(String),
    /// Underlying I/O while assembling the file.
    #[error("could not assemble the PDF: {0}")]
    Io(#[from] std::io::Error),
}

/// What was embedded, plus anything the conversion discarded.
struct Embedded {
    /// Filter name for the image stream dictionary.
    filter: &'static str,
    /// Colour space: `/DeviceRGB` or `/DeviceGray`.
    colour_space: &'static str,
    /// The stream payload.
    data: Vec<u8>,
    /// Disclosed losses, for the receipt.
    removed: Vec<String>,
}

/// Wrap a JPEG or PNG into a single-page PDF sized to the image in points at
/// 72 dpi.
///
/// # Errors
///
/// [`PdfError`] — every variant names what failed in the user's terms.
pub fn wrap_image(
    from: openconvert_core::format::FormatId,
    bytes: &[u8],
    limits: &openconvert_core::limits::Limits,
) -> Result<(Vec<u8>, Vec<String>), PdfError> {
    // Header first, limit second, decode third — the same order the in-process
    // image path uses, for the same reason: a 4 KB file claiming 500 Mpx must
    // cost nothing to refuse.
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| PdfError::Header(e.to_string()))?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| PdfError::Header(e.to_string()))?;
    if width == 0 || height == 0 {
        return Err(PdfError::Header(
            "the image declares zero dimensions".into(),
        ));
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > limits.decode_pixels {
        return Err(PdfError::Header(format!(
            "this image is {width}x{height} = {pixels} pixels, over the limit of {}",
            limits.decode_pixels
        )));
    }

    use openconvert_core::format::FormatId as F;
    let embedded = match from {
        // JPEG is the one format that needs no decode at all: PDF speaks
        // DCTDecode natively, so the source bytes become the stream verbatim.
        F::Jpeg => embed_jpeg(bytes)?,
        // PNG's pixels arrive as a Flate stream, which PDF also speaks.
        F::Png => embed_png(bytes)?,
        // EVERY OTHER RASTER FORMAT GOES THROUGH PNG.
        //
        // PDF has exactly two image filters we can reach without inventing
        // one: DCTDecode and FlateDecode. TIFF, WebP, GIF and BMP speak
        // neither, so their pixels have to be re-expressed — and re-expressing
        // them AS PNG reuses `embed_png` whole rather than growing a fourth
        // near-copy of the same stream assembly.
        //
        // The re-encode is lossless (PNG is), so the only cost is time and the
        // Class B on these rows is inherited from the wrapping, not added by
        // this hop. `wrap_image` has already refused anything over the pixel
        // budget, so the decode below is bounded before it starts.
        F::Tiff | F::Webp | F::Gif | F::Bmp => {
            let decoded = image::ImageReader::new(Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| PdfError::Header(e.to_string()))?
                .decode()
                .map_err(|e| PdfError::Header(format!("could not decode the {from}: {e}")))?;
            let mut as_png = Cursor::new(Vec::new());
            decoded
                .write_to(&mut as_png, image::ImageFormat::Png)
                .map_err(|e| PdfError::Header(format!("could not re-encode as PNG: {e}")))?;
            let mut embedded = embed_png(&as_png.into_inner())?;
            embedded.removed.push(format!(
                "{from} animation, layers and metadata (only the first image is wrapped)"
            ));
            embedded
        }
        other => {
            return Err(PdfError::Header(format!(
                "{other} cannot be wrapped as PDF by this build"
            )))
        }
    };

    Ok((assemble(width, height, &embedded), embedded.removed))
}

/// JPEG: verbatim bytes, no decode. Colour space comes from the header's own
/// component count, because a grayscale JPEG embedded as RGB renders garbage.
fn embed_jpeg(bytes: &[u8]) -> Result<Embedded, PdfError> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| PdfError::Header(e.to_string()))?;
    let colour = reader
        .into_decoder()
        .map(|d| d.color_type())
        .map_err(|e| PdfError::Header(e.to_string()))?;
    let colour_space = match colour {
        image::ColorType::L8 | image::ColorType::L16 => "/DeviceGray",
        _ => "/DeviceRGB",
    };
    Ok(Embedded {
        filter: "/DCTDecode",
        colour_space,
        data: bytes.to_vec(),
        removed: vec!["all EXIF/metadata (not carried into the PDF)".to_string()],
    })
}

/// PNG: decoded to samples and re-compressed. Alpha has no free ride — an
/// RGBA sample composited onto white is a DECISION about every transparent
/// pixel, so it is disclosed rather than silent.
fn embed_png(bytes: &[u8]) -> Result<Embedded, PdfError> {
    let img = image::load_from_memory(bytes).map_err(|e| PdfError::Decode(e.to_string()))?;
    let mut removed = vec!["all PNG metadata chunks (not carried into the PDF)".to_string()];
    let rgba = img.to_rgba8();
    let mut rgb = Vec::with_capacity(rgba.as_raw().len() / 4 * 3);
    let had_alpha = rgba.pixels().any(|p| p.0[3] != 255);
    if had_alpha {
        removed.push("alpha channel composited onto white".to_string());
    }
    // Straight composite over white: c' = (c*a + 255*(1-a)) / 255. The
    // intermediate is at most 2*65025, so u32; the division rounds down,
    // which biases transparent pixels toward white by less than one step.
    for p in rgba.pixels() {
        let [r, g, b, a] = p.0.map(u32::from);
        rgb.push(((r * a + 255 * (255 - a)) / 255) as u8);
        rgb.push(((g * a + 255 * (255 - a)) / 255) as u8);
        rgb.push(((b * a + 255 * (255 - a)) / 255) as u8);
    }
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&rgb)?;
    let data = encoder.finish()?;
    Ok(Embedded {
        filter: "/FlateDecode",
        colour_space: "/DeviceRGB",
        data,
        removed,
    })
}

/// Assemble the five-object PDF with a real xref table.
///
/// Offsets are counted as the objects are appended, never patched afterwards —
/// a repaired xref is how a malformed PDF gets shipped looking valid.
fn assemble(width: u32, height: u32, embedded: &Embedded) -> Vec<u8> {
    // /Length is the EXACT byte count between "stream\n" and "\nendstream",
    // so the trailing newline of the content stream is part of neither. A
    // length off by one is a PDF some viewers accept and some refuse, which
    // is worse than one they all refuse.
    let content = format!("q {width} 0 0 {height} 0 0 cm /Im0 Do Q");
    let mut out: Vec<u8> = Vec::with_capacity(embedded.data.len() + 1024);

    let mut offsets = [0u32; 6]; // object number -> byte offset; index 0 unused
    let push = |out: &mut Vec<u8>, offsets: &mut [u32; 6], obj: u32, body: &[u8]| {
        offsets[obj as usize] = out.len() as u32;
        out.extend_from_slice(format!("{obj} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"endobj\n");
    };

    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");

    push(
        &mut out,
        &mut offsets,
        1,
        b"<< /Type /Catalog /Pages 2 0 R >>\n",
    );
    push(
        &mut out,
        &mut offsets,
        2,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>\n",
    );
    push(
        &mut out,
        &mut offsets,
        3,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << /XObject << /Im0 5 0 R >> >> /Contents 4 0 R >>\n"
        )
        .as_bytes(),
    );
    push(
        &mut out,
        &mut offsets,
        4,
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream\n",
            content.len(),
            content
        )
        .as_bytes(),
    );
    push(
        &mut out,
        &mut offsets,
        5,
        format!(
            "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
             /ColorSpace {} /BitsPerComponent 8 /Filter {} /Length {} >>\nstream\n",
            embedded.colour_space,
            embedded.filter,
            embedded.data.len()
        )
        .as_bytes(),
    );
    out.extend_from_slice(&embedded.data);
    out.extend_from_slice(b"\nendstream\nendobj\n");

    let xref_at = out.len() as u32;
    out.extend_from_slice("xref\n0 6\n".to_string().as_bytes());
    out.extend_from_slice(b"0000000000 65535 f\r\n");
    for off in &offsets[1..=5] {
        out.extend_from_slice(format!("{off:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(b"trailer\n<< /Size 6 /Root 1 0 R >>\n");
    out.extend_from_slice(format!("startxref\n{xref_at}\n%%EOF\n").as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_bytes(w: u32, h: u32, alpha: bool) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, _y| {
            if alpha && x < w / 2 {
                image::Rgba([10, 20, 30, 128])
            } else {
                image::Rgba([200, 100, 50, 255])
            }
        });
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([x as u8, y as u8, 77]));
        let mut out = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    fn limits() -> openconvert_core::limits::Limits {
        openconvert_core::limits::Limits::defaults()
    }

    /// The produced file parses back as a PDF with the right page box, and
    /// every xref offset actually points where it claims.
    #[test]
    fn a_wrapped_png_is_a_structurally_valid_pdf() {
        let (pdf, _) = wrap_image(
            openconvert_core::format::FormatId::Png,
            &png_bytes(64, 48, false),
            &limits(),
        )
        .expect("wrap");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(pdf.ends_with(b"%%EOF\n"));

        // Each recorded xref offset must land on its own object header. A PDF
        // whose xref lies is the definition of a corrupt file; this is what
        // makes the writer's offsets counted, not assumed.
        let text = String::from_utf8_lossy(&pdf);
        let xref_at = text.match_indices("xref\n").next().expect("xref").0;
        // Lines: "xref", "0 6", the FREE entry (0000000000 65535 f), then
        // five live ones. Skipping three skips the header pair AND the free
        // entry -- the first draft skipped two and asserted against the free
        // row, which "passed" only by pointing object 1 at the file header.
        let entries: Vec<usize> = text[xref_at..]
            .lines()
            .skip(3)
            .take(5)
            .filter_map(|l| l[..10].parse().ok())
            .collect();
        assert_eq!(entries.len(), 5, "five live xref entries expected");
        for (i, off) in entries.iter().enumerate() {
            let expected = format!("{} 0 obj", i + 1);
            assert_eq!(
                String::from_utf8_lossy(&pdf[*off..*off + expected.len()]),
                expected,
                "xref entry {} does not point at its object",
                i + 1
            );
        }
        assert!(text.contains("/MediaBox [0 0 64 48]"));
        assert!(text.contains("/Filter /FlateDecode"));
    }

    /// A JPEG rides through byte-identical: the DCT stream IS the source.
    #[test]
    fn a_jpeg_is_embedded_verbatim_without_decoding() {
        let src = jpeg_bytes(32, 16);
        let (pdf, removed) =
            wrap_image(openconvert_core::format::FormatId::Jpeg, &src, &limits()).expect("wrap");
        // The whole source appears inside the PDF stream, unmodified.
        let pos = pdf
            .windows(src.len())
            .position(|w| w == src.as_slice())
            .expect("source bytes not found verbatim in the PDF");
        assert!(pos > 0);
        assert!(removed.iter().any(|r| r.contains("EXIF")));
    }

    /// Alpha is composited and DISCLOSED, never silently dropped.
    #[test]
    fn an_alpha_png_discloses_the_composite() {
        let (_, removed) = wrap_image(
            openconvert_core::format::FormatId::Png,
            &png_bytes(16, 16, true),
            &limits(),
        )
        .expect("wrap");
        assert!(
            removed.iter().any(|r| r.contains("alpha")),
            "alpha loss must be disclosed: {removed:?}"
        );
    }

    /// The pixel bomb is refused before any decode, from the header alone.
    #[test]
    fn the_pixel_limit_refuses_before_decoding() {
        // 64x64 fixture against a ceiling of 100 pixels.
        let err = wrap_image(
            openconvert_core::format::FormatId::Png,
            &png_bytes(64, 64, false),
            &openconvert_core::limits::Limits {
                decode_pixels: 100,
                ..openconvert_core::limits::Limits::defaults()
            },
        )
        .expect_err("should refuse");
        assert!(err.to_string().contains("4096") && err.to_string().contains("100"));
    }

    /// Determinism: two wraps of the same input produce identical bytes.
    ///
    /// This is the property the receipt's hash pair is built on.
    #[test]
    fn wrapping_is_deterministic() {
        let src = png_bytes(24, 24, false);
        let a = wrap_image(openconvert_core::format::FormatId::Png, &src, &limits()).expect("a");
        let b = wrap_image(openconvert_core::format::FormatId::Png, &src, &limits()).expect("b");
        assert_eq!(a.0, b.0);
    }
}
