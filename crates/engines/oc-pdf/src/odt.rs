//! Writing a minimal, valid `.odt`.
//!
//! # Hand-built, for the same reason `docx.rs` is
//!
//! An ODT is a ZIP of XML parts, and the subset that expresses "headings and
//! paragraphs of text" is four small files. Every ODF-writing crate available
//! brings a full object model — styles, tables, drawings, metadata — to
//! produce those same four files, inside a worker whose entire job is handling
//! untrusted input. The trade is a couple of hundred lines here against tens
//! of thousands of lines of dependency in the sandbox, for a format we are
//! WRITING, where the bytes are ours and nothing hostile is parsed.
//!
//! # The one rule ODF has that OOXML does not
//!
//! **`mimetype` must be the first entry in the archive, STORED rather than
//! deflated, and carry no extra field.** It is how a reader identifies the
//! package by reading the first few dozen bytes without inflating anything,
//! and it is specified that way in ODF 1.2 §3.3. A deflated `mimetype` still
//! unzips, still contains the right string, and is still rejected by strict
//! readers — a failure that looks like a corrupt file rather than a
//! misordered one.

use std::io::Write;

use crate::text::{Paragraph, Style};

/// The declared type of a text document.
const MIMETYPE: &str = "application/vnd.oasis.opendocument.text";

const MANIFEST: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.2">"#,
    r#"<manifest:file-entry manifest:full-path="/" manifest:version="1.2" manifest:media-type="application/vnd.oasis.opendocument.text"/>"#,
    r#"<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>"#,
    r#"<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>"#,
    r#"</manifest:manifest>"#,
);

/// Two heading styles and a body style, which is all the pipeline carries.
const STYLES: &str = concat!(
    r#"<?xml version="1.0" encoding="UTF-8"?>"#,
    r#"<office:document-styles xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
    r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" office:version="1.2">"#,
    r#"<office:styles>"#,
    r#"<style:style style:name="Standard" style:family="paragraph"/>"#,
    r#"<style:style style:name="Heading_20_1" style:display-name="Heading 1" style:family="paragraph" style:parent-style-name="Standard">"#,
    r#"<style:text-properties fo:font-size="20pt" fo:font-weight="bold"/></style:style>"#,
    r#"<style:style style:name="Heading_20_2" style:display-name="Heading 2" style:family="paragraph" style:parent-style-name="Standard">"#,
    r#"<style:text-properties fo:font-size="15pt" fo:font-weight="bold"/></style:style>"#,
    r#"<style:style style:name="Preformatted" style:family="paragraph" style:parent-style-name="Standard">"#,
    r#"<style:text-properties style:font-name="Courier New" fo:font-family="&apos;Courier New&apos;"/></style:style>"#,
    r#"<style:style style:name="Quotation" style:family="paragraph" style:parent-style-name="Standard">"#,
    r#"<style:paragraph-properties fo:margin-left="1cm"/><style:text-properties fo:font-style="italic"/></style:style>"#,
    r#"</office:styles></office:document-styles>"#,
);

/// Build the `.odt` bytes for these paragraphs.
///
/// # Errors
///
/// Only if the in-memory ZIP writer fails, which would be a bug here rather
/// than anything about the input.
pub fn write(paragraphs: &[Paragraph]) -> Result<Vec<u8>, String> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));

    // FIRST, AND STORED. See the module note: this is the one ordering rule
    // ODF has, and getting it wrong produces a file that unzips correctly and
    // is refused by readers.
    let stored: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file("mimetype", stored)
        .map_err(|e| format!("could not start the mimetype entry: {e}"))?;
    zip.write_all(MIMETYPE.as_bytes())
        .map_err(|e| format!("could not write the mimetype entry: {e}"))?;

    let deflated: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in [
        ("META-INF/manifest.xml", MANIFEST.to_string()),
        ("styles.xml", STYLES.to_string()),
        ("content.xml", content_xml(paragraphs)),
    ] {
        zip.start_file(name, deflated)
            .map_err(|e| format!("could not start the {name} part: {e}"))?;
        zip.write_all(body.as_bytes())
            .map_err(|e| format!("could not write the {name} part: {e}"))?;
    }

    let cursor = zip
        .finish()
        .map_err(|e| format!("could not finish the odt package: {e}"))?;
    Ok(cursor.into_inner())
}

/// The document body.
fn content_xml(paragraphs: &[Paragraph]) -> String {
    let mut out = String::with_capacity(paragraphs.len() * 96 + 512);
    out.push_str(concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        r#"<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
        r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2">"#,
        r#"<office:body><office:text>"#,
    ));
    for para in paragraphs {
        let body = escape(&para.text);
        match para.style {
            Style::Heading(level) => {
                let depth = level.clamp(1, 6);
                out.push_str(&format!(
                    r#"<text:h text:style-name="Heading_20_{depth}" text:outline-level="{depth}">{body}</text:h>"#
                ));
            }
            // LIST MARKERS AS TEXT, and the receipt says so.
            //
            // A real ODF list is `<text:list><text:list-item>` with a matching
            // `<text:list-style>` in styles.xml, nested once per level. Writing
            // the marker into the paragraph instead is visibly poorer -- but a
            // half-built list structure that some readers render as a list and
            // others as nothing is worse than text that always looks right.
            // This is the same trade `docx.rs` makes, kept identical so the two
            // outputs of one conversion do not disagree.
            Style::Bullet { depth } => {
                let indent = "\u{a0}\u{a0}".repeat(usize::from(depth));
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard">{indent}• {body}</text:p>"#
                ));
            }
            Style::Ordered { depth, number } => {
                let indent = "\u{a0}\u{a0}".repeat(usize::from(depth));
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard">{indent}{number}. {body}</text:p>"#
                ));
            }
            Style::Code => {
                // One paragraph per line: ODF collapses runs of spaces and has
                // no preformatted paragraph type, so the line breaks have to be
                // real paragraph breaks or they vanish.
                for line in para.text.lines() {
                    out.push_str(&format!(
                        r#"<text:p text:style-name="Preformatted">{}</text:p>"#,
                        escape(line)
                    ));
                }
            }
            Style::Quote => {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Quotation">{body}</text:p>"#
                ));
            }
            Style::Body => {
                out.push_str(&format!(
                    r#"<text:p text:style-name="Standard">{body}</text:p>"#
                ));
            }
        }
    }
    out.push_str("</office:text></office:body></office:document-content>");
    out
}

/// XML-escape, and drop the control characters XML 1.0 cannot represent.
///
/// The same rule `docx.rs` applies, for the same reason: a stray `0x01` from a
/// PDF's text stream makes the whole part unparseable, and there is no escape
/// sequence for it — XML 1.0 simply cannot hold one.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn para(text: &str, style: Style) -> Paragraph {
        Paragraph {
            text: text.to_string(),
            style,
        }
    }

    fn open(bytes: &[u8]) -> zip::ZipArchive<std::io::Cursor<&[u8]>> {
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("the odt must be a readable ZIP")
    }

    fn member(bytes: &[u8], name: &str) -> String {
        let mut zip = open(bytes);
        let mut f = zip.by_name(name).expect("member");
        let mut s = String::new();
        f.read_to_string(&mut s).expect("read");
        s
    }

    /// The rule that is easy to get wrong and produces a file readers refuse.
    #[test]
    fn mimetype_is_first_and_stored() {
        let bytes = write(&[para("x", Style::Body)]).expect("write");
        let zip = open(&bytes);
        assert_eq!(
            zip.file_names().next(),
            Some("mimetype"),
            "mimetype must be the first entry in the archive"
        );

        let mut zip = open(&bytes);
        let entry = zip.by_index(0).expect("first entry");
        assert_eq!(entry.compression(), zip::CompressionMethod::Stored);

        assert_eq!(member(&bytes, "mimetype"), MIMETYPE);
    }

    /// Headings survive as headings, which is the whole point of not writing
    /// plain text.
    #[test]
    fn headings_become_text_h_and_body_becomes_text_p() {
        let bytes = write(&[
            para("The Title", Style::Heading(1)),
            para("A section", Style::Heading(2)),
            para("Prose.", Style::Body),
        ])
        .expect("write");
        let content = member(&bytes, "content.xml");
        assert!(content.contains(r#"text:outline-level="1">The Title</text:h>"#));
        assert!(content.contains(r#"text:outline-level="2">A section</text:h>"#));
        assert!(content.contains(r#"<text:p text:style-name="Standard">Prose.</text:p>"#));
    }

    /// The package a reader needs is all present.
    #[test]
    fn the_required_parts_are_all_there() {
        let bytes = write(&[para("x", Style::Body)]).expect("write");
        let zip = open(&bytes);
        let names: Vec<&str> = zip.file_names().collect();
        for required in [
            "mimetype",
            "META-INF/manifest.xml",
            "content.xml",
            "styles.xml",
        ] {
            assert!(names.contains(&required), "missing {required} in {names:?}");
        }
    }

    /// Markup in the text cannot become markup in the document.
    #[test]
    fn text_is_escaped() {
        let bytes = write(&[para("<b>&</b> \"q\"", Style::Body)]).expect("write");
        let content = member(&bytes, "content.xml");
        assert!(content.contains("&lt;b&gt;&amp;&lt;/b&gt;"), "{content}");
        assert!(!content.contains("<b>"));
    }

    /// A control character cannot make the part unparseable.
    #[test]
    fn control_characters_are_dropped() {
        let bytes = write(&[para("a\u{1}b", Style::Body)]).expect("write");
        let content = member(&bytes, "content.xml");
        assert!(content.contains("ab"), "{content}");
        assert!(!content.contains('\u{1}'));
    }

    /// And this round-trips through our own reader, which is the strongest
    /// check available without another office suite.
    #[test]
    fn our_own_extractor_reads_it_back() {
        let bytes = write(&[
            para("The Title", Style::Heading(1)),
            para("Prose.", Style::Body),
        ])
        .expect("write");
        let back = crate::office::structured(&bytes, crate::office::Office::Odt).expect("read");
        assert_eq!(back.len(), 2, "{back:?}");
        assert_eq!(back[0].style, Style::Heading(1));
        assert_eq!(back[0].text, "The Title");
        assert_eq!(back[1].style, Style::Body);
        assert_eq!(back[1].text, "Prose.");
    }
}
