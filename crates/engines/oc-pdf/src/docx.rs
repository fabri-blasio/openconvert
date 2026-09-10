//! Writing a minimal, valid `.docx`.
//!
//! # Why this is hand-built rather than a crate
//!
//! A DOCX is a ZIP of XML parts, and the subset needed to express "headings
//! and paragraphs of text" is four small files. Every DOCX-writing crate
//! available brings a full OOXML object model — styles, numbering, tables,
//! drawings, relationships — to produce those same four files, and it would be
//! pulled into a worker whose entire job is handling untrusted input. The
//! trade is a few hundred lines here against a few tens of thousands of lines
//! of dependency inside the sandbox, for a format we are WRITING (where the
//! bytes are ours and no parsing of hostile input happens at all).
//!
//! This is the same reasoning that keeps `embed_image` off the `lopdf`
//! dependency next door, and it is written down for the same reason: the next
//! person to want a table will be tempted to add the crate, and should know
//! what that costs and where.
//!
//! # What it produces
//!
//! The four parts Word requires and nothing else:
//!
//! - `[Content_Types].xml` — the type of every part. Word refuses the file
//!   outright without it.
//! - `_rels/.rels` — the package relationship pointing at the main document.
//! - `word/_rels/document.xml.rels` — the document's own (empty) relationships.
//!   Word tolerates its absence; some readers do not.
//! - `word/document.xml` — the text.
//! - `word/styles.xml` — definitions for `Heading1` and `Heading2`, because a
//!   paragraph referencing a style that does not exist renders as body text
//!   and the heading detection would silently do nothing.
//!
//! No fonts, no theme, no settings. Word supplies its own defaults for all
//! three, and shipping ours would be asserting a design the PDF never told us.

use std::io::Write as _;

use crate::text::{Paragraph, Style};

/// Anything Word will not accept inside a text run.
///
/// XML 1.0 has no escape for most control characters — they cannot appear in a
/// conforming document at all, not even numerically — so they are dropped
/// rather than encoded. A single stray `\u{0}` from a damaged content stream
/// otherwise produces a file Word refuses to open with no explanation.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(' '),
            c if (c as u32) < 0x20 => {}
            // Unpaired surrogates cannot reach here (Rust `char` excludes
            // them); the remaining XML-illegal range is the noncharacters.
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const PACKAGE_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const DOCUMENT_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

/// `Heading1` and `Heading2`, defined because a `pStyle` naming a style the
/// document does not define renders as body text.
///
/// Sizes are in half-points, which is what `w:sz` takes — 32 is 16 pt. They
/// are Word's own heading proportions rather than anything measured from the
/// PDF: the PDF's own point sizes were evidence that a line WAS a heading, not
/// an instruction to reproduce them, and carrying them across would produce a
/// document whose headings cannot be restyled.
const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="200" w:after="100"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="26"/><w:szCs w:val="26"/></w:rPr></w:style>
</w:styles>"#;

/// Build `word/document.xml` from the pages.
///
/// Pages are separated by an explicit page break rather than by nothing: the
/// PDF's pagination is one of the few pieces of layout that survives being
/// re-flowed, and a reader comparing the two documents side by side is the
/// most likely reader this conversion has.
fn document_xml(pages: &[Vec<Paragraph>]) -> String {
    let mut body = String::new();
    let with_text: Vec<&Vec<Paragraph>> = pages.iter().filter(|p| !p.is_empty()).collect();
    for (i, page) in with_text.iter().enumerate() {
        if i > 0 {
            body.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        for para in page.iter() {
            // CODE IS ONE PARAGRAPH PER LINE, because a `w:p` is a paragraph
            // and Word has no preformatted one. Emitting the whole block as a
            // single run would collapse every line break in it.
            if para.style == Style::Code {
                for line in para.text.lines() {
                    body.push_str(&format!(
                        "<w:p><w:pPr><w:pStyle w:val=\"Preformatted\"/></w:pPr>\
                         <w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                        escape(line)
                    ));
                }
                continue;
            }

            let style = match para.style {
                Style::Body | Style::Bullet { .. } | Style::Ordered { .. } => String::new(),
                Style::Heading(level) => {
                    format!(
                        "<w:pPr><w:pStyle w:val=\"Heading{}\"/></w:pPr>",
                        level.clamp(1, 2)
                    )
                }
                Style::Quote => "<w:pPr><w:pStyle w:val=\"Quote\"/></w:pPr>".to_string(),
                // Handled above; `continue` means this is unreachable.
                Style::Code => String::new(),
            };

            // LIST MARKERS AS TEXT, and the receipt says so.
            //
            // A real Word list needs a `numbering.xml` part, a `w:numPr`
            // reference per paragraph, and an abstract numbering definition per
            // level. Writing the marker into the text instead is visibly
            // poorer -- and a half-built numbering definition that Word renders
            // as a list and LibreOffice renders as nothing is worse than text
            // that always looks right. `odt.rs` makes the identical trade, so
            // the two outputs of one conversion agree.
            let marker = match para.style {
                Style::Bullet { depth } => {
                    format!("{}• ", "\u{a0}\u{a0}".repeat(usize::from(depth)))
                }
                Style::Ordered { depth, number } => {
                    format!("{}{number}. ", "\u{a0}\u{a0}".repeat(usize::from(depth)))
                }
                _ => String::new(),
            };

            // `xml:space="preserve"` because a paragraph may legitimately end
            // in a space and Word collapses it otherwise.
            body.push_str(&format!(
                "<w:p>{style}<w:r><w:t xml:space=\"preserve\">{marker}{}</w:t></w:r></w:p>",
                escape(para.text.trim())
            ));
        }
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body}<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>"#
    )
}

/// Pack the pages into a `.docx`.
///
/// # Errors
///
/// Only if the ZIP writer fails, which for an in-memory cursor means an
/// allocation failure.
pub fn write(pages: &[Vec<Paragraph>]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    // Deflate, matching what every DOCX in the world uses. A stored-only
    // package is valid and roughly four times the size.
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // ORDER MATTERS TO SOME READERS. `[Content_Types].xml` first is required
    // by OPC, and while Word tolerates it anywhere, several stricter readers
    // (and the OPC specification itself) do not.
    for (name, body) in [
        ("[Content_Types].xml", CONTENT_TYPES.to_string()),
        ("_rels/.rels", PACKAGE_RELS.to_string()),
        ("word/_rels/document.xml.rels", DOCUMENT_RELS.to_string()),
        ("word/document.xml", document_xml(pages)),
        ("word/styles.xml", STYLES.to_string()),
    ] {
        zip.start_file(name, options)
            .map_err(|e| format!("could not start the {name} part: {e}"))?;
        zip.write_all(body.as_bytes())
            .map_err(|e| format!("could not write the {name} part: {e}"))?;
    }

    Ok(zip
        .finish()
        .map_err(|e| format!("could not finish the document package: {e}"))?
        .into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Paragraph {
        Paragraph {
            text: text.to_string(),
            style: Style::Body,
        }
    }

    fn read_part(bytes: &[u8], name: &str) -> String {
        use std::io::Read as _;
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a readable ZIP");
        let mut part = zip.by_name(name).expect("the part exists");
        let mut s = String::new();
        part.read_to_string(&mut s).expect("valid UTF-8");
        s
    }

    /// **The package has every part Word requires.**
    ///
    /// Missing `[Content_Types].xml` or the package relationship produces a
    /// file Word refuses with "the file is corrupt", which is a failure the
    /// user cannot act on and this conversion must never produce.
    #[test]
    fn the_package_has_the_parts_word_requires() {
        let out = write(&[vec![body("hello")]]).expect("writes");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&out)).expect("a readable ZIP");
        let names: Vec<String> = (0..zip.len())
            .map(|i| zip.by_index(i).expect("entry").name().to_string())
            .collect();
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "word/document.xml",
            "word/styles.xml",
        ] {
            assert!(names.contains(&required.to_string()), "missing {required}");
        }
        assert_eq!(
            names[0], "[Content_Types].xml",
            "OPC requires the content types part first"
        );
    }

    /// A `.docx` is a ZIP, and `sniff` identifies it by looking inside for
    /// `word/document.xml`. If ours is not found there, every receipt and
    /// every subsequent conversion misidentifies our own output.
    #[test]
    fn our_output_is_identifiable_as_a_docx() {
        let out = write(&[vec![body("hello")]]).expect("writes");
        assert_eq!(&out[..4], &[0x50, 0x4B, 0x03, 0x04], "ZIP local header");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&out)).expect("a readable ZIP");
        assert!(zip.by_name("word/document.xml").is_ok());
    }

    /// **XML metacharacters in the text do not produce a broken document.**
    ///
    /// A PDF containing `Q&A` or `<draft>` is entirely ordinary, and an
    /// unescaped ampersand is the classic way a generated OOXML file becomes
    /// unopenable.
    #[test]
    fn xml_metacharacters_are_escaped() {
        let out = write(&[vec![body("Q&A about <draft> \"quotes\"")]]).expect("writes");
        let xml = read_part(&out, "word/document.xml");
        assert!(xml.contains("Q&amp;A about &lt;draft&gt;"), "{xml}");
        assert!(!xml.contains("Q&A"), "raw ampersand survived");
    }

    /// Control characters cannot be represented in XML at all and are dropped.
    #[test]
    fn control_characters_are_removed_rather_than_encoded() {
        let out = write(&[vec![body("before\u{0}\u{7}after")]]).expect("writes");
        let xml = read_part(&out, "word/document.xml");
        assert!(xml.contains("beforeafter"), "{xml}");
        assert!(!xml.contains('\u{0}'));
    }

    /// A heading references a style the document actually defines.
    #[test]
    fn headings_reference_a_defined_style() {
        let out = write(&[vec![
            Paragraph {
                text: "Title".into(),
                style: Style::Heading(1),
            },
            body("text"),
        ]])
        .expect("writes");
        let xml = read_part(&out, "word/document.xml");
        assert!(xml.contains(r#"<w:pStyle w:val="Heading1"/>"#), "{xml}");
        let styles = read_part(&out, "word/styles.xml");
        assert!(styles.contains(r#"w:styleId="Heading1""#), "{styles}");
    }

    /// Pages are separated by a page break, and a page with no text does not
    /// produce a stray blank one.
    #[test]
    fn empty_pages_do_not_produce_stray_breaks() {
        let out = write(&[vec![body("one")], Vec::new(), vec![body("two")]]).expect("writes");
        let xml = read_part(&out, "word/document.xml");
        assert_eq!(
            xml.matches(r#"<w:br w:type="page"/>"#).count(),
            1,
            "two pages of text is one break: {xml}"
        );
    }
}
