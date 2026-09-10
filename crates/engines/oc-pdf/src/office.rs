//! Text extraction from the two ZIP-based office formats.
//!
//! # These are archives, and that is the security story
//!
//! A `.docx` is a ZIP with `word/document.xml` inside it; a `.odt` is a ZIP
//! with `content.xml`. So this is archive handling wearing a document's name,
//! and it inherits the archive problems: a small file can declare a very large
//! member, and a member can be a decompression bomb. Both are bounded here
//! rather than trusted, using the same running-total rule `oc-archive` applies.
//!
//! # Extraction, not conversion
//!
//! What comes out is the text and nothing else. Styles, tables, images,
//! headers, footnotes, comments and tracked changes are all discarded — a
//! table's cells arrive as a run of lines with no indication they were a
//! table. That is a lot to lose, so the conversion says so rather than
//! implying a document survived.

use std::io::Read;

use crate::text::{Paragraph, Style};

/// The most decompressed XML this will hold.
///
/// The document part of a real `.docx` is measured in hundreds of kilobytes
/// even for a long report. 64 MiB is far past that and far below anything that
/// threatens the worker's memory cap.
const MAX_PART: u64 = 64 << 20;

/// Which member of the archive holds the text.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Office {
    /// OOXML: `word/document.xml`.
    Docx,
    /// OpenDocument: `content.xml`.
    Odt,
}

impl Office {
    const fn part(self) -> &'static str {
        match self {
            Self::Docx => "word/document.xml",
            Self::Odt => "content.xml",
        }
    }

    /// Elements that end a line of text.
    ///
    /// Both formats mark paragraphs with an element rather than with any
    /// character in the text, so without this every paragraph in the document
    /// runs together into one line.
    const fn breaks(self) -> &'static [&'static str] {
        match self {
            Self::Docx => &["</w:p>", "<w:br", "</w:tr>"],
            Self::Odt => &[
                "</text:p>",
                "</text:h>",
                "<text:line-break",
                "</table:table-row>",
            ],
        }
    }

    /// The element whose character data is the text.
    const fn text_tag(self) -> &'static str {
        match self {
            Self::Docx => "w:t",
            Self::Odt => "text:",
        }
    }
}

/// Pull the plain text out of an office document.
///
/// # Errors
///
/// A file that is not a ZIP, one missing the part that holds the text, or a
/// part that decompresses past [`MAX_PART`].
pub fn extract(bytes: &[u8], kind: Office) -> Result<String, String> {
    Ok(text_of(&read_member(bytes, kind)?, kind))
}

/// The same text, with the headings the document names.
///
/// # Why this is separate from [`extract`] rather than replacing it
///
/// `extract` is what `-> txt` uses, its output is pinned by tests, and plain
/// text has nowhere to put a heading anyway. Nothing would improve by routing
/// it through here, and a shared scanner would put the risk of changing `txt`
/// output into every change made for Markdown, PDF or ODT.
///
/// # What a heading is, here
///
/// A claim the FILE makes, not one this code infers. That is the difference
/// between this and `text::layout`, which reconstructs headings from glyph
/// sizes because a PDF records none:
///
/// * DOCX says `<w:pStyle w:val="Heading1"/>` inside the paragraph's `w:pPr`.
/// * ODT uses a different element for the paragraph itself, `<text:h>`, and
///   carries the depth in `text:outline-level`.
///
/// Levels are clamped to the two [`Style::Heading`] distinguishes. A `.docx`
/// can name nine, and rendering nine would promise a hierarchy the rest of the
/// pipeline does not carry.
///
/// # Errors
///
/// As [`extract`].
pub fn structured(bytes: &[u8], kind: Office) -> Result<Vec<Paragraph>, String> {
    let xml = read_member(bytes, kind)?;
    Ok(paragraphs_of(&xml, kind))
}

/// One pass, emitting a paragraph per break rather than a newline.
///
/// Deliberately the same shape as [`text_of`]: a scanner, not an XML parser,
/// for the reason written there — a parser is where billion-laughs lives, and
/// this needs to find text runs and paragraph starts and nothing else.
fn paragraphs_of(xml: &str, kind: Office) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut current = String::new();
    let mut style = Style::Body;
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    let mut depth: usize = 0;

    // Close the paragraph in hand, if it has anything in it.
    //
    // Deliberately does NOT reset `style`. The reset belongs at the paragraph
    // break, which is the only place a new paragraph begins — the final flush
    // after the loop has nothing following it, so resetting there is a write
    // nobody reads.
    macro_rules! flush {
        () => {
            let text = current.trim().to_string();
            if !text.is_empty() {
                out.push(Paragraph { text, style });
            }
            current.clear();
        };
    }

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = match xml[i..].find('>') {
                Some(e) => i + e + 1,
                None => break, // truncated markup: stop rather than guess
            };
            let tag = &xml[i..end];
            let name = tag
                .trim_start_matches('<')
                .trim_start_matches('/')
                .split([' ', '>', '/'])
                .next()
                .unwrap_or("");

            if kind.breaks().iter().any(|b| tag.starts_with(b)) {
                flush!();
                // A new paragraph starts unstyled until its own markup says
                // otherwise.
                style = Style::Body;
            }

            // The heading claim, read off the element that makes it.
            match kind {
                Office::Docx => {
                    if name == "w:pStyle" {
                        if let Some(level) = docx_heading_level(tag) {
                            style = Style::Heading(level);
                        }
                    }
                }
                Office::Odt => {
                    if name == "text:h" && !tag.starts_with("</") {
                        style = Style::Heading(odt_outline_level(tag));
                    }
                }
            }

            let is_text_tag = match kind {
                Office::Docx => name == "w:t",
                Office::Odt => name.starts_with(kind.text_tag()),
            };
            if is_text_tag && !tag.ends_with("/>") {
                if tag.starts_with("</") {
                    depth = depth.saturating_sub(1);
                } else {
                    depth += 1;
                }
            }
            i = end;
            continue;
        }
        if depth > 0 {
            let start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            unescape_into(&xml[start..i], &mut current);
            continue;
        }
        i += 1;
    }
    flush!();
    out
}

/// `<w:pStyle w:val="Heading2"/>` -> `Some(2)`, clamped to the two levels
/// `Style::Heading` carries. Anything that is not a heading style is `None`.
fn docx_heading_level(tag: &str) -> Option<u8> {
    let val = attribute(tag, "w:val")?;
    // Word writes "Heading1"; some producers write "heading 1".
    let rest = val
        .trim()
        .strip_prefix("Heading")
        .or_else(|| val.trim().strip_prefix("heading").map(|r| r.trim_start()))?;
    let n: u8 = rest.trim().parse().ok()?;
    Some(n.clamp(1, 2))
}

/// `<text:h text:outline-level="3">` -> 2, since that is as deep as this
/// pipeline distinguishes. A missing level means the shallowest heading.
fn odt_outline_level(tag: &str) -> u8 {
    attribute(tag, "text:outline-level")
        .and_then(|v| v.trim().parse::<u8>().ok())
        .unwrap_or(1)
        .clamp(1, 2)
}

/// One attribute's value out of a start tag, without parsing the tag.
///
/// Handles both quote characters and returns `None` rather than guessing at
/// anything malformed — this reads untrusted markup, and a scanner that
/// improvises is a scanner that can be steered.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(name)?;
    let rest = tag[at + name.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let rest = &rest[quote.len_utf8()..];
    let close = rest.find(quote)?;
    Some(&rest[..close])
}

/// The document part, read under the size cap.
///
/// Factored out so [`extract`] and [`structured`] cannot disagree about the
/// bound. The cap is the whole security story of this module and it should
/// exist once.
///
/// # Errors
///
/// A file that is not a ZIP, one missing the part that holds the text, or a
/// part that decompresses past [`MAX_PART`].
fn read_member(bytes: &[u8], kind: Office) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("this file is not a readable ZIP container: {e}"))?;
    let mut part = zip.by_name(kind.part()).map_err(|_| {
        format!(
            "this archive has no {}; it is not the document it claims to be",
            kind.part()
        )
    })?;

    // Bounded by the READ, not by the declared size: a ZIP entry's header can
    // claim anything, and the number that matters is what actually arrives.
    let mut xml = String::new();
    let read = part
        .by_ref()
        .take(MAX_PART + 1)
        .read_to_string(&mut xml)
        .map_err(|e| format!("could not read {}: {e}", kind.part()))?;
    if read as u64 > MAX_PART {
        return Err(format!(
            "{} expands past the {MAX_PART}-byte limit",
            kind.part()
        ));
    }
    Ok(xml)
}

/// Character data from the text elements, with paragraph breaks kept.
///
/// # Why this is a scanner and not an XML parser
///
/// A full parser would be correct and would also mean pulling a dependency
/// into a worker that currently links one C library and nothing else. What is
/// needed here is narrow: find the text runs, keep their contents, and note
/// where paragraphs end. A scanner does that in a bounded single pass over
/// input it never interprets — no entity expansion beyond the five named ones,
/// no DTD, no external references. Those absences are the point: XML parsers
/// are where billion-laughs lives.
fn text_of(xml: &str, kind: Office) -> String {
    let mut out = String::with_capacity(xml.len() / 4);
    let bytes = xml.as_bytes();
    let mut i = 0usize;
    // How many text elements deep the scanner is.
    //
    // A COUNTER, not a flag. ODT nests spans inside paragraphs, and a boolean
    // set to false by `</text:span>` swallows the rest of the paragraph after
    // it -- "Body text with a span" instead of "Body text with a span inside."
    // DOCX's `w:t` does not nest, so the counter is simply always 0 or 1 there.
    let mut depth: usize = 0;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let end = match xml[i..].find('>') {
                Some(e) => i + e + 1,
                None => break, // truncated markup: stop rather than guess
            };
            let tag = &xml[i..end];

            if kind.breaks().iter().any(|b| tag.starts_with(b)) && !out.ends_with('\n') {
                out.push('\n');
            }
            // `<w:t>` and `<w:t xml:space="preserve">` both open text; `<w:tab/>`
            // must not, which is why the check is on the tag name and not on a
            // prefix of the whole element.
            let name = tag
                .trim_start_matches('<')
                .trim_start_matches('/')
                .split([' ', '>', '/'])
                .next()
                .unwrap_or("");
            let is_text_tag = match kind {
                Office::Docx => name == "w:t",
                // ODT nests spans inside paragraphs; their character data is
                // all text, so any `text:` element counts.
                Office::Odt => name.starts_with(kind.text_tag()),
            };
            if is_text_tag && !tag.ends_with("/>") {
                if tag.starts_with("</") {
                    depth = depth.saturating_sub(1);
                } else {
                    depth += 1;
                }
            }
            i = end;
            continue;
        }
        if depth > 0 {
            let start = i;
            while i < bytes.len() && bytes[i] != b'<' {
                i += 1;
            }
            unescape_into(&xml[start..i], &mut out);
            continue;
        }
        i += 1;
    }

    // Collapse the runs of blank lines that paragraph markers leave behind.
    let mut cleaned = String::with_capacity(out.len());
    let mut blank = 0usize;
    for line in out.lines() {
        let t = line.trim_end();
        if t.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        cleaned.push_str(t);
        cleaned.push('\n');
    }
    cleaned.trim_end().to_string()
}

/// The five XML entities, and numeric references.
///
/// Deliberately nothing else: a custom entity is a DTD feature, and expanding
/// those is how a 4 KB file becomes a gigabyte of text.
pub(crate) fn unescape_into(s: &str, out: &mut String) {
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&p| p <= 12) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..semi];
        match ent {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            _ => {
                let code = ent
                    .strip_prefix("#x")
                    .or_else(|| ent.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()));
                match code.and_then(char::from_u32) {
                    Some(c) => out.push(c),
                    // An entity we do not expand is kept verbatim rather than
                    // dropped: losing it silently would corrupt the text.
                    None => out.push_str(&tail[..=semi]),
                }
            }
        }
        rest = &tail[semi + 1..];
    }
    out.push_str(rest);
}

#[cfg(test)]
mod tests {
    use super::{text_of, Office};

    /// The nesting case a boolean got wrong.
    #[test]
    fn text_after_a_closing_span_is_kept() {
        let xml = "<text:p>Body with <text:span>a span</text:span> inside.</text:p>";
        assert_eq!(text_of(xml, Office::Odt), "Body with a span inside.");
    }

    /// `<w:tab/>` is not text, and `<w:t>` with attributes is.
    #[test]
    fn docx_runs_join_and_self_closing_tags_are_not_text() {
        let xml = concat!(
            r#"<w:p><w:r><w:t xml:space="preserve">Hello </w:t></w:r>"#,
            "<w:r><w:tab/><w:t>world</w:t></w:r></w:p>",
            "<w:p><w:r><w:t>next</w:t></w:r></w:p>"
        );
        assert_eq!(text_of(xml, Office::Docx), "Hello world\nnext");
    }

    /// Only the five named entities and numeric references expand.
    #[test]
    fn entities_expand_and_unknown_ones_survive_verbatim() {
        let xml = "<w:t>a &amp; b &lt;c&gt; &#233; &#x2014; &nope; &unterminated</w:t>";
        assert_eq!(
            text_of(xml, Office::Docx),
            "a & b <c> \u{e9} \u{2014} &nope; &unterminated"
        );
    }

    /// A DTD entity is NOT expanded, which is what keeps billion-laughs out.
    #[test]
    fn a_custom_entity_is_not_expanded() {
        let xml = "<w:t>&lol9;</w:t>";
        assert_eq!(text_of(xml, Office::Docx), "&lol9;");
    }

    /// Truncated markup stops the scan rather than reading past it.
    #[test]
    fn truncated_markup_does_not_run_away() {
        let xml = "<w:p><w:r><w:t>kept</w:t></w:r><w:unclosed";
        assert_eq!(text_of(xml, Office::Docx), "kept");
    }
}
