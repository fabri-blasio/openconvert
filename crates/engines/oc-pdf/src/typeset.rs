//! Writing a PDF from reconstructed paragraphs.
//!
//! # What this is, and firmly is not
//!
//! It is a **typesetter for text**: paragraphs in, a paged PDF out, with
//! headings larger than body text and lines wrapped to a measured width. It is
//! the inverse of `text.rs`, which recovers paragraphs from a PDF's glyphs.
//!
//! It is not a Word layout engine. A `.docx` records tables, images, columns,
//! floats, footnotes, numbering and a style system, and none of that survives
//! here — the receipt says so in as many words. What arrives is the document's
//! text, in its reading order, on pages.
//!
//! That is worth having anyway, because it is what most people mean by
//! "convert this to PDF" for a text document, and the alternative on offer was
//! nothing at all.
//!
//! # Why the base-14 fonts
//!
//! Helvetica is one of the fourteen fonts every PDF reader is required to
//! have, so nothing is embedded. Embedding a font would mean shipping one,
//! licensing it, subsetting it, and writing a `CMap` — and the reason to do
//! all that is glyph coverage this converter does not have anyway: the base-14
//! encoding is `WinAnsiEncoding`, which is Latin-1 and no more.
//!
//! Characters outside it are transliterated where there is an obvious
//! equivalent and replaced with `?` where there is not, and the receipt
//! reports it. Dropping them silently would produce a document that looks
//! finished and has holes in it.

use lopdf::{dictionary, Document, Object, Stream};

use crate::text::{Paragraph, Style};

/// Page geometry, in PDF points. A4, which is the default nearly everywhere
/// this program is used.
const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 56.0;

/// Body size, and the two heading sizes above it.
const BODY_SIZE: f32 = 11.0;
const HEADING_SIZES: [f32; 2] = [20.0, 15.0];

/// Multiplied by the font size to get the baseline-to-baseline distance.
const LEADING: f32 = 1.32;

/// Monospace, for preformatted blocks. Courier is base-14 like Helvetica, so
/// this needs no embedded font either.
const CODE_SIZE: f32 = 9.5;

/// How far one level of list nesting or quotation moves the left edge.
const INDENT_STEP: f32 = 18.0;

/// The average width of a Helvetica glyph as a fraction of the font size.
///
/// **An approximation, and the honest kind.** Helvetica is proportional, so
/// exact wrapping needs its widths table — 224 numbers whose only job is to
/// make lines break a word or two later. 0.5 is close to the average for
/// mixed-case Latin text and errs SHORT, which means a line breaks early
/// rather than running into the margin. Early is invisible; late is a defect
/// you can see from across the room.
const AVG_GLYPH: f32 = 0.5;

/// Turn paragraphs into a PDF.
///
/// # Errors
///
/// Only if `lopdf` cannot serialise the document it was handed, which would be
/// a bug here rather than anything about the input.
pub fn write(paragraphs: &[Paragraph]) -> Result<(Vec<u8>, Lost), String> {
    let mut lost = Lost::default();

    // Lay every paragraph out into pages of positioned lines first, so the
    // page objects can be built once each rather than reopened.
    let mut pages: Vec<Vec<Placed>> = Vec::new();
    let mut page: Vec<Placed> = Vec::new();
    let mut y = PAGE_H - MARGIN;

    for para in paragraphs {
        let size = match para.style {
            Style::Heading(level) => {
                // Six declared levels, three distinct sizes. Beyond the third
                // the difference would be under a point, which is a
                // distinction the page cannot show and the reader cannot see.
                let idx = usize::from(level.clamp(1, 6)).min(HEADING_SIZES.len());
                HEADING_SIZES[idx - 1]
            }
            Style::Code => CODE_SIZE,
            _ => BODY_SIZE,
        };
        let leading = size * LEADING;
        let indent = MARGIN + f32::from(para.style.depth()) * INDENT_STEP;

        // A little air above a heading, and none above the first line on a
        // page -- a paragraph gap at the top margin is just a wider margin.
        if !page.is_empty() {
            y -= if matches!(para.style, Style::Heading(_)) {
                leading * 0.9
            } else {
                leading * 0.45
            };
        }

        // A PREFORMATTED BLOCK IS NOT RE-WRAPPED. Its line breaks are the
        // content -- indentation, alignment, the shape of the code -- and a
        // typesetter that reflows one has destroyed the thing it was asked to
        // carry. Lines too long for the measure are cut rather than folded,
        // and the receipt says so.
        let lines: Vec<Vec<u8>> = if para.style.is_preformatted() {
            para.text
                .lines()
                .map(|line| {
                    let mut encoded = encode(line, &mut lost);
                    let room = usable_chars(size, indent);
                    if encoded.len() > room {
                        encoded.truncate(room);
                        lost.truncated += 1;
                    }
                    encoded
                })
                .collect()
        } else {
            let encoded = encode(&para.text, &mut lost);
            wrap_to(&encoded, size, usable_chars(size, indent))
        };

        let marker = marker_for(para.style);
        for (i, line) in lines.into_iter().enumerate() {
            if y - leading < MARGIN {
                pages.push(std::mem::take(&mut page));
                y = PAGE_H - MARGIN;
            }
            y -= leading;
            // The marker sits on the first line only; continuations align with
            // the text rather than under the bullet.
            let (x, text) = if i == 0 && !marker.is_empty() {
                let mut with = encode(&marker, &mut lost);
                with.extend_from_slice(&line);
                (indent, with)
            } else if marker.is_empty() {
                (indent, line)
            } else {
                (indent + marker.len() as f32 * size * AVG_GLYPH, line)
            };
            page.push(Placed { text, size, x, y });
        }
    }
    if !page.is_empty() {
        pages.push(page);
    }
    // A document with no pages is not a document any reader will open.
    if pages.is_empty() {
        pages.push(Vec::new());
    }

    Ok((build(&pages)?, lost))
}

/// What could not be carried across, for the receipt.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Lost {
    /// Characters replaced because `WinAnsiEncoding` has no glyph for them.
    pub unmappable: usize,
    /// Preformatted lines cut because they were wider than the page.
    ///
    /// Counted rather than folded: a code line that wraps has silently changed
    /// meaning, and one that is cut is visibly incomplete. Visible beats
    /// silent, and the receipt reports the count either way.
    pub truncated: usize,
}

/// One line, placed on a page.
struct Placed {
    text: Vec<u8>,
    size: f32,
    /// Left edge. Not always the margin: list items and quotations indent.
    x: f32,
    y: f32,
}

/// How many glyphs fit between an indent and the right margin.
fn usable_chars(size: f32, indent: f32) -> usize {
    let usable = PAGE_W - MARGIN - indent;
    ((usable / (size * AVG_GLYPH)).floor() as usize).max(8)
}

/// The literal marker a list item carries, if any.
///
/// Written into the text rather than drawn, for the same reason `docx.rs` and
/// `odt.rs` write theirs as text: a marker that is part of the line cannot
/// drift out of alignment with it.
fn marker_for(style: Style) -> String {
    match style {
        Style::Bullet { .. } => "\u{2022} ".to_string(),
        Style::Ordered { number, .. } => format!("{number}. "),
        Style::Quote => "| ".to_string(),
        _ => String::new(),
    }
}

/// Break `text` into lines that fit the measure.
fn wrap_to(text: &[u8], _size: f32, per_line: usize) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut line: Vec<u8> = Vec::new();
    for word in text.split(|b| *b == b' ') {
        if word.is_empty() {
            continue;
        }
        // A single word longer than the measure is broken rather than allowed
        // to run off the page: a URL or a hash has no space to break at, and
        // the alternative is text in the margin.
        if word.len() > per_line {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            for chunk in word.chunks(per_line) {
                lines.push(chunk.to_vec());
            }
            continue;
        }
        let extra = if line.is_empty() { 0 } else { 1 };
        if line.len() + extra + word.len() > per_line {
            lines.push(std::mem::take(&mut line));
        } else if extra == 1 {
            line.push(b' ');
        }
        line.extend_from_slice(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

/// UTF-8 to `WinAnsiEncoding`, counting what could not be carried.
///
/// The transliterations are the ones that occur constantly in real documents
/// and have an unambiguous ASCII equivalent — the quotation marks and dashes a
/// word processor substitutes as you type. Everything else outside Latin-1
/// becomes `?` and is counted, because a silent hole is worse than a visible
/// one.
fn encode(text: &str, lost: &mut Lost) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let replacement: &[u8] = match ch {
            '\u{2018}' | '\u{2019}' | '\u{201B}' => b"'",
            '\u{201C}' | '\u{201D}' | '\u{201F}' => b"\"",
            '\u{2013}' | '\u{2014}' | '\u{2212}' => b"-",
            '\u{2026}' => b"...",
            '\u{00A0}' | '\u{2007}' | '\u{202F}' => b" ",
            '\u{2022}' => b"-",
            '\t' => b" ",
            // Latin-1 maps one to one onto WinAnsi for everything a text
            // document is likely to hold.
            c if (c as u32) < 0x100 && !c.is_control() => {
                out.push(c as u32 as u8);
                continue;
            }
            _ => {
                lost.unmappable += 1;
                b"?"
            }
        };
        out.extend_from_slice(replacement);
    }
    out
}

/// Escape a byte string for a PDF literal string.
fn escape(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 8);
    for &b in bytes {
        if b == b'(' || b == b')' || b == b'\\' {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

/// Assemble the pages into a document.
fn build(pages: &[Vec<Placed>]) -> Result<Vec<u8>, String> {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();

    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let mono_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Courier",
        "Encoding" => "WinAnsiEncoding",
    });
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id, "F2" => mono_id },
    });

    let mut kids: Vec<Object> = Vec::new();
    for page in pages {
        let mut content = Vec::new();
        content.extend_from_slice(b"BT\n");
        let mut current = f32::NAN;
        for placed in page {
            // Courier for the preformatted size, Helvetica otherwise. Keyed on
            // the size because that is what distinguishes them here, and both
            // fonts are declared in the page's resources.
            let font = if (placed.size - CODE_SIZE).abs() < f32::EPSILON {
                "F2"
            } else {
                "F1"
            };
            if placed.size != current {
                content.extend_from_slice(format!("/{font} {} Tf\n", placed.size).as_bytes());
                current = placed.size;
            }
            // Absolute placement per line: `Td` is relative to the previous
            // text position, and a document that positions relatively drifts
            // once a heading changes the leading mid-page.
            content.extend_from_slice(format!("1 0 0 1 {} {} Tm\n", placed.x, placed.y).as_bytes());
            content.push(b'(');
            content.extend_from_slice(&escape(&placed.text));
            content.extend_from_slice(b") Tj\n");
        }
        content.extend_from_slice(b"ET");

        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), PAGE_W.into(), PAGE_H.into()],
            "Resources" => resources_id,
        });
        kids.push(page_id.into());
    }

    let count = i64::try_from(kids.len()).unwrap_or(i64::MAX);
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| format!("could not write the pdf: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str, style: Style) -> Paragraph {
        Paragraph {
            text: text.to_string(),
            style,
        }
    }

    #[test]
    fn it_writes_a_pdf_a_reader_would_open() {
        let (bytes, lost) = write(&[
            para("The Title", Style::Heading(1)),
            para("Some body text.", Style::Body),
        ])
        .expect("write");
        assert!(bytes.starts_with(b"%PDF"), "no PDF header");
        assert!(bytes.ends_with(b"%%EOF\n") || bytes.ends_with(b"%%EOF"));
        assert_eq!(lost.unmappable, 0);

        // And lopdf can read back what it wrote, which is a stronger claim
        // than "the bytes start with %PDF".
        let doc = Document::load_mem(&bytes).expect("the output must parse");
        assert_eq!(doc.page_iter().count(), 1);
    }

    /// Long documents paginate rather than writing off the bottom of page one.
    #[test]
    fn text_longer_than_a_page_gets_more_pages() {
        let many: Vec<Paragraph> = (0..200)
            .map(|i| para(&format!("Paragraph number {i}."), Style::Body))
            .collect();
        let (bytes, _) = write(&many).expect("write");
        let doc = Document::load_mem(&bytes).expect("parse");
        assert!(
            doc.page_iter().count() > 1,
            "200 paragraphs fitted on one page, so pagination is not happening"
        );
    }

    /// A word with no space in it is broken, not run into the margin.
    #[test]
    fn an_unbreakable_word_is_broken_rather_than_overflowing() {
        let long = "x".repeat(400);
        let lines = wrap_to(long.as_bytes(), BODY_SIZE, usable_chars(BODY_SIZE, MARGIN));
        let usable = PAGE_W - 2.0 * MARGIN;
        assert!(lines.len() > 1, "a 400-character word was not broken");
        for line in &lines {
            let width = line.len() as f32 * BODY_SIZE * AVG_GLYPH;
            assert!(width <= usable, "a line is {width} wide against {usable}");
        }
    }

    /// Smart quotes survive as their ASCII equivalents and are not counted as
    /// losses; genuinely unmappable characters are counted.
    #[test]
    fn transliteration_is_counted_honestly() {
        let mut lost = Lost::default();
        let out = encode("\u{201C}quoted\u{201D} \u{2014} yes", &mut lost);
        assert_eq!(String::from_utf8_lossy(&out), "\"quoted\" - yes");
        assert_eq!(
            lost.unmappable, 0,
            "substitutable punctuation is not a loss"
        );

        let mut lost2 = Lost::default();
        let cjk = encode("hello \u{4E16}\u{754C}", &mut lost2);
        assert_eq!(lost2.unmappable, 2, "two unmappable characters");
        assert!(cjk.ends_with(b"??"));
    }

    /// Parentheses and backslashes cannot break out of a PDF string literal.
    #[test]
    fn string_literals_are_escaped() {
        let (bytes, _) = write(&[para(r"a ) and a \ and a (", Style::Body)]).expect("write");
        Document::load_mem(&bytes).expect("unescaped text broke the document");
    }

    /// An empty document still produces something openable.
    #[test]
    fn nothing_in_still_makes_a_valid_pdf() {
        let (bytes, _) = write(&[]).expect("write");
        let doc = Document::load_mem(&bytes).expect("parse");
        assert_eq!(doc.page_iter().count(), 1);
    }
}
