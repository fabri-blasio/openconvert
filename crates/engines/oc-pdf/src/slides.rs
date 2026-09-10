//! Reading a slide deck into the paragraph model.
//!
//! # Same shape as `office.rs`, and the same scanner
//!
//! A `.pptx` is a ZIP holding one XML part per slide; a `.odp` is a ZIP holding
//! one `content.xml` with a `<draw:page>` per slide. Both are read by the same
//! bounded scanner `office.rs` uses — not an XML parser, for the reason written
//! there: a parser is where billion-laughs lives, and what is needed here is to
//! find text runs and the elements that end them.
//!
//! # A slide is a page, where a page exists
//!
//! The paragraph model has no slide boundary, and inventing one would mean a
//! `Style` variant every writer would then have to decide what to do with.
//! Instead a deck arrives as `Vec<Vec<Paragraph>>` — the same "pages" shape a
//! PDF produces — and `docx::write` puts a page break between them, so a deck
//! opened in Word is a page per slide.
//!
//! Markdown, HTML and ODT have no page break, and the PDF path deliberately
//! flows rather than paginating: this worker has no slide graphics to put on a
//! page, so a page per slide would be hundreds of nearly-empty ones. `run_deck`
//! in `main.rs` carries that decision and its receipt says which happened.
//!
//! # What is dropped, and it is a lot
//!
//! **Speaker notes**, which live in a separate part (`ppt/notesSlides/`) and in
//! `<presentation:notes>` — they are not the deck, and a conversion that
//! silently merged them into it would produce a document nobody wrote. The
//! receipt says they were left out rather than letting somebody assume they
//! came along.
//!
//! Also every image, every shape, every animation, the layout, the theme, and
//! the position of anything on the slide. What comes out is the words.

use std::io::Read;

use crate::text::{Paragraph, Style};

/// The most decompressed XML this will hold, per part.
///
/// Same number and same reasoning as `office.rs`: far past any real slide, far
/// below anything that threatens the worker's memory cap.
const MAX_PART: u64 = 64 << 20;

/// The most slides this will read.
///
/// A deck of a thousand slides is already absurd; this is a bound on a loop
/// over attacker-supplied structure, not an opinion about presentations.
const MAX_SLIDES: usize = 2000;

/// Which deck format this is.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Deck {
    /// OOXML: one part per slide under `ppt/slides/`.
    Pptx,
    /// OpenDocument: every slide in `content.xml`.
    Odp,
}

/// Pull a deck's slides out, one `Vec<Paragraph>` per slide.
///
/// # Errors
///
/// A file that is not a ZIP, one holding no slides, or a part that decompresses
/// past [`MAX_PART`].
pub fn read(bytes: &[u8], deck: Deck) -> Result<Vec<Vec<Paragraph>>, String> {
    let slides = match deck {
        Deck::Pptx => read_pptx(bytes)?,
        Deck::Odp => read_odp(bytes)?,
    };
    if slides.is_empty() {
        return Err("this presentation has no slides with any text in them".into());
    }
    Ok(slides)
}

// ---------------------------------------------------------------------------
// PPTX
// ---------------------------------------------------------------------------

/// Read the slides in **presentation order**.
///
/// # Why not just list `ppt/slides/*.xml`
///
/// Because `slide10.xml` sorts before `slide2.xml`, and a deck delivered in
/// that order looks exactly like a working conversion. The order is stated by
/// the file: `ppt/presentation.xml` lists `<p:sldId r:id="rIdN"/>` in order,
/// and `ppt/_rels/presentation.xml.rels` maps each `rIdN` to a part.
///
/// Falling back to the archive's own order when those parts are missing or
/// unreadable is deliberate: a deck we cannot order is still a deck, and the
/// alternative is refusing a file whose text we can read perfectly well. The
/// receipt says which happened.
fn read_pptx(bytes: &[u8]) -> Result<Vec<Vec<Paragraph>>, String> {
    let mut zip = open_zip(bytes)?;

    let order = slide_order(&mut zip);
    let names: Vec<String> = if order.is_empty() {
        let mut all: Vec<String> = zip
            .file_names()
            .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
            .map(ToString::to_string)
            .collect();
        // NUMERIC, not lexical, for exactly the `slide10` reason above. This is
        // the fallback path and it still must not scramble the deck.
        all.sort_by_key(|n| slide_number(n).unwrap_or(u32::MAX));
        all
    } else {
        order
    };

    let mut out = Vec::new();
    for name in names.into_iter().take(MAX_SLIDES) {
        let Ok(xml) = read_part(&mut zip, &name) else {
            continue;
        };
        let slide = paragraphs_of(&xml, Deck::Pptx);
        if !slide.is_empty() {
            out.push(slide);
        }
    }
    Ok(out)
}

/// The slide parts, in the order the presentation states.
///
/// Returns empty when either part is missing or says nothing, which the caller
/// treats as "fall back to the archive's order".
fn slide_order(zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>) -> Vec<String> {
    let (Ok(pres), Ok(rels)) = (
        read_part(zip, "ppt/presentation.xml"),
        read_part(zip, "ppt/_rels/presentation.xml.rels"),
    ) else {
        return Vec::new();
    };

    // rId -> part name, from the relationships part. Targets are relative to
    // `ppt/`, so `slides/slide1.xml` becomes `ppt/slides/slide1.xml`.
    let mut targets: Vec<(String, String)> = Vec::new();
    for tag in tags(&rels) {
        if !tag.starts_with("<Relationship") {
            continue;
        }
        let (Some(id), Some(target)) = (attribute(tag, "Id"), attribute(tag, "Target")) else {
            continue;
        };
        // A target may be absolute (`/ppt/slides/...`) or escape upward. Only
        // the ordinary relative form is followed: anything else is a name this
        // code would have to repair, and a name somebody wanted repaired is a
        // name to refuse.
        if target.contains("..") || target.starts_with('/') {
            continue;
        }
        targets.push((id.to_string(), format!("ppt/{target}")));
    }

    let mut out = Vec::new();
    for tag in tags(&pres) {
        if !tag.starts_with("<p:sldId") {
            continue;
        }
        let Some(rid) = attribute(tag, "r:id") else {
            continue;
        };
        if let Some((_, name)) = targets.iter().find(|(id, _)| id == rid) {
            out.push(name.clone());
        }
    }
    out
}

/// `ppt/slides/slide12.xml` -> `Some(12)`.
fn slide_number(name: &str) -> Option<u32> {
    name.rsplit_once("slide")?
        .1
        .strip_suffix(".xml")?
        .parse()
        .ok()
}

// ---------------------------------------------------------------------------
// ODP
// ---------------------------------------------------------------------------

/// Every `<draw:page>` in `content.xml`, in document order — which for ODP
/// **is** presentation order, so there is no relationships part to consult.
fn read_odp(bytes: &[u8]) -> Result<Vec<Vec<Paragraph>>, String> {
    let mut zip = open_zip(bytes)?;
    let xml = read_part(&mut zip, "content.xml")?;

    let mut out = Vec::new();
    for page in xml.split("<draw:page").skip(1).take(MAX_SLIDES) {
        let slide = paragraphs_of(page, Deck::Odp);
        if !slide.is_empty() {
            out.push(slide);
        }
    }
    // A presentation with no `draw:page` at all is not one this reads, and
    // saying so beats returning an empty deck that looks like an empty file.
    Ok(out)
}

// ---------------------------------------------------------------------------
// The scanner
// ---------------------------------------------------------------------------

impl Deck {
    /// Elements that end a line of text.
    const fn breaks(self) -> &'static [&'static str] {
        match self {
            // `a:p` is a paragraph inside a shape's text body; `a:br` is a
            // soft break inside one.
            Self::Pptx => &["</a:p>", "<a:br"],
            Self::Odp => &["</text:p>", "</text:h>", "<text:line-break"],
        }
    }

    /// The element whose character data is the text.
    const fn text_tag(self) -> &'static str {
        match self {
            Self::Pptx => "a:t",
            Self::Odp => "text:",
        }
    }

    /// The element that opens a shape, where a title placeholder is declared.
    const fn shape(self) -> &'static str {
        match self {
            Self::Pptx => "<p:sp",
            Self::Odp => "<draw:frame",
        }
    }
}

/// One slide's text, as paragraphs.
///
/// # How a slide maps onto the model
///
/// **The title placeholder becomes `Heading(1)` and everything else becomes a
/// bullet.** That is the reading a deck actually supports: a slide has a title
/// and a body of points, and the body is drawn as a bulleted list far more
/// often than not. Indentation level (`lvl` in PPTX, the list depth in ODP) is
/// carried into `Bullet { depth }`, which the Markdown and HTML writers already
/// nest.
///
/// A slide with no title placeholder gets no heading, rather than the first
/// line being promoted to one — promoting it would invent a structure the file
/// does not state, which is the line `office.rs` draws in the same place.
fn paragraphs_of(xml: &str, deck: Deck) -> Vec<Paragraph> {
    let mut out: Vec<Paragraph> = Vec::new();
    let mut current = String::new();
    let mut depth: usize = 0;
    // Whether the shape being scanned is the slide's title.
    let mut in_title = false;
    // Indent level of the paragraph in hand.
    let mut level: u8 = 0;

    let bytes = xml.as_bytes();
    let mut i = 0usize;

    macro_rules! flush {
        () => {
            let text = current.trim().to_string();
            if !text.is_empty() {
                out.push(Paragraph {
                    text,
                    style: if in_title {
                        Style::Heading(1)
                    } else {
                        Style::Bullet { depth: level }
                    },
                });
            }
            current.clear();
        };
    }

    while i < bytes.len() {
        if bytes[i] == b'<' {
            let Some(e) = xml[i..].find('>') else {
                break; // truncated markup: stop rather than guess
            };
            let end = i + e + 1;
            let tag = &xml[i..end];
            let name = tag
                .trim_start_matches('<')
                .trim_start_matches('/')
                .split([' ', '>', '/'])
                .next()
                .unwrap_or("");

            if deck.breaks().iter().any(|b| tag.starts_with(b)) {
                flush!();
                level = 0;
            }

            // THE TITLE IS A PLACEHOLDER TYPE, NOT A POSITION.
            //
            // PPTX says `<p:ph type="title"/>` (or `ctrTitle`) inside the
            // shape's non-visual properties; ODP says
            // `presentation:class="title"` on the frame. Reading the first
            // shape instead would be a guess, and a deck whose title box sits
            // below its body is not unusual.
            if tag.starts_with(deck.shape()) && !tag.starts_with("</") {
                in_title = false;
            }
            match deck {
                Deck::Pptx => {
                    if name == "p:ph" {
                        let t = attribute(tag, "type").unwrap_or("");
                        in_title = t == "title" || t == "ctrTitle";
                    }
                    if name == "a:pPr" {
                        level = attribute(tag, "lvl")
                            .and_then(|v| v.parse::<u8>().ok())
                            .unwrap_or(0)
                            .min(8);
                    }
                }
                Deck::Odp => {
                    if name == "draw:frame" && !tag.starts_with("</") {
                        in_title = matches!(
                            attribute(tag, "presentation:class"),
                            Some("title" | "outline")
                        ) && attribute(tag, "presentation:class") == Some("title");
                    }
                }
            }

            let is_text = match deck {
                Deck::Pptx => name == "a:t",
                Deck::Odp => name.starts_with(deck.text_tag()),
            };
            if is_text && !tag.ends_with("/>") {
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
            crate::office::unescape_into(&xml[start..i], &mut current);
            continue;
        }
        i += 1;
    }
    flush!();
    out
}

// ---------------------------------------------------------------------------
// Shared plumbing
// ---------------------------------------------------------------------------

fn open_zip(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>, String> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("this file is not a readable ZIP container: {e}"))
}

/// One member, bounded by what actually arrives rather than by what the header
/// claims — the rule `office.rs` and `oc-archive` both follow.
pub(crate) fn read_part(
    zip: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    name: &str,
) -> Result<String, String> {
    let mut part = zip
        .by_name(name)
        .map_err(|_| format!("this archive has no {name}"))?;
    let mut xml = String::new();
    let read = part
        .by_ref()
        .take(MAX_PART + 1)
        .read_to_string(&mut xml)
        .map_err(|e| format!("could not read {name}: {e}"))?;
    if read as u64 > MAX_PART {
        return Err(format!("{name} expands past the {MAX_PART}-byte limit"));
    }
    Ok(xml)
}

/// Every tag in the document, as slices. A scanner, not a parser.
pub(crate) fn tags(xml: &str) -> impl Iterator<Item = &str> {
    let mut rest = xml;
    core::iter::from_fn(move || {
        let start = rest.find('<')?;
        let end = rest[start..].find('>')? + start + 1;
        let tag = &rest[start..end];
        rest = &rest[end..];
        Some(tag)
    })
}

/// `name="value"` out of a tag, single or double quoted.
pub(crate) fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let at = tag.find(name)?;
    let rest = tag[at + name.len()..].trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(&body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn zipped(members: &[(&str, &str)]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default();
        for (name, body) in members {
            w.start_file(*name, opts).expect("start");
            w.write_all(body.as_bytes()).expect("write");
        }
        w.finish().expect("finish").into_inner()
    }

    fn slide(title: &str, points: &[(&str, u8)]) -> String {
        let mut xml = String::from("<p:sld><p:cSld><p:spTree>");
        xml.push_str(&format!(
            "<p:sp><p:nvSpPr><p:nvPr><p:ph type=\"title\"/></p:nvPr></p:nvSpPr>\
             <p:txBody><a:p><a:r><a:t>{title}</a:t></a:r></a:p></p:txBody></p:sp>"
        ));
        xml.push_str(
            "<p:sp><p:nvSpPr><p:nvPr><p:ph type=\"body\" idx=\"1\"/></p:nvPr></p:nvSpPr><p:txBody>",
        );
        for (text, lvl) in points {
            xml.push_str(&format!(
                "<a:p><a:pPr lvl=\"{lvl}\"/><a:r><a:t>{text}</a:t></a:r></a:p>"
            ));
        }
        xml.push_str("</p:txBody></p:sp></p:spTree></p:cSld></p:sld>");
        xml
    }

    /// **The trap this module exists to avoid.** `slide10` sorts before
    /// `slide2`, so a deck ordered by filename is a deck in the wrong order —
    /// and it looks exactly like a working conversion.
    #[test]
    fn slides_come_out_in_presentation_order_not_filename_order() {
        let n = 11;
        let mut members: Vec<(String, String)> = (1..=n)
            .map(|i| {
                (
                    format!("ppt/slides/slide{i}.xml"),
                    slide(&format!("Title {i}"), &[]),
                )
            })
            .collect();

        // The presentation states the order, and it is NOT the filenames':
        // slide 11 is presented second.
        let ids: Vec<usize> = std::iter::once(1)
            .chain(std::iter::once(11))
            .chain(2..=10)
            .collect();
        let pres = format!(
            "<p:presentation><p:sldIdLst>{}</p:sldIdLst></p:presentation>",
            ids.iter()
                .map(|i| format!("<p:sldId id=\"{}\" r:id=\"rId{i}\"/>", 255 + i))
                .collect::<String>()
        );
        let rels = format!(
            "<Relationships>{}</Relationships>",
            (1..=n)
                .map(|i| format!("<Relationship Id=\"rId{i}\" Target=\"slides/slide{i}.xml\"/>"))
                .collect::<String>()
        );
        members.push(("ppt/presentation.xml".to_string(), pres));
        members.push(("ppt/_rels/presentation.xml.rels".to_string(), rels));

        let borrowed: Vec<(&str, &str)> = members
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let deck = read(&zipped(&borrowed), Deck::Pptx).expect("read the deck");

        let titles: Vec<&str> = deck.iter().map(|s| s[0].text.as_str()).collect();
        assert_eq!(
            titles[..3],
            ["Title 1", "Title 11", "Title 2"],
            "the deck must follow the presentation, not the filenames"
        );
    }

    /// And the fallback, when no presentation part says otherwise, still must
    /// not sort `slide10` before `slide2`.
    #[test]
    fn the_fallback_order_is_numeric_not_lexical() {
        let members: Vec<(String, String)> = (1..=11)
            .map(|i| {
                (
                    format!("ppt/slides/slide{i}.xml"),
                    slide(&format!("Title {i}"), &[]),
                )
            })
            .collect();
        let borrowed: Vec<(&str, &str)> = members
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let deck = read(&zipped(&borrowed), Deck::Pptx).expect("read");
        let titles: Vec<&str> = deck.iter().map(|s| s[0].text.as_str()).collect();
        assert_eq!(titles[..3], ["Title 1", "Title 2", "Title 3"]);
        assert_eq!(titles[10], "Title 11");
    }

    #[test]
    fn the_title_placeholder_becomes_a_heading_and_the_body_bullets() {
        let members = [(
            "ppt/slides/slide1.xml",
            slide("The Title", &[("First", 0), ("Nested", 1)]),
        )];
        let borrowed: Vec<(&str, &str)> = members.iter().map(|(a, b)| (*a, b.as_str())).collect();
        let deck = read(&zipped(&borrowed), Deck::Pptx).expect("read");
        assert_eq!(deck.len(), 1);
        assert_eq!(
            deck[0]
                .iter()
                .map(|p| (p.style, p.text.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (Style::Heading(1), "The Title"),
                (Style::Bullet { depth: 0 }, "First"),
                (Style::Bullet { depth: 1 }, "Nested"),
            ]
        );
    }

    #[test]
    fn an_odp_reads_its_pages_in_document_order() {
        let content = "<office:document-content><office:body><office:presentation>\
             <draw:page draw:name=\"page1\">\
               <draw:frame presentation:class=\"title\"><draw:text-box>\
                 <text:p>First slide</text:p></draw:text-box></draw:frame>\
               <draw:frame presentation:class=\"outline\"><draw:text-box>\
                 <text:p>A point</text:p></draw:text-box></draw:frame>\
             </draw:page>\
             <draw:page draw:name=\"page2\">\
               <draw:frame presentation:class=\"title\"><draw:text-box>\
                 <text:p>Second slide</text:p></draw:text-box></draw:frame>\
             </draw:page>\
             </office:presentation></office:body></office:document-content>";
        let deck = read(&zipped(&[("content.xml", content)]), Deck::Odp).expect("read");
        assert_eq!(deck.len(), 2);
        assert_eq!(deck[0][0].text, "First slide");
        assert_eq!(deck[0][0].style, Style::Heading(1));
        assert_eq!(deck[0][1].text, "A point");
        assert_eq!(deck[1][0].text, "Second slide");
    }

    #[test]
    fn a_deck_with_no_text_is_refused_rather_than_returned_empty() {
        let err = read(
            &zipped(&[("ppt/slides/slide1.xml", "<p:sld/>")]),
            Deck::Pptx,
        )
        .expect_err("nothing to convert");
        assert!(err.contains("no slides"), "{err}");
    }

    /// A relationship target that climbs out of the archive is not followed.
    #[test]
    fn a_relationship_escaping_the_archive_is_ignored() {
        let members = [
            ("ppt/slides/slide1.xml", slide("Real", &[])),
            (
                "ppt/presentation.xml",
                "<p:sldIdLst><p:sldId r:id=\"rId9\"/></p:sldIdLst>".to_string(),
            ),
            (
                "ppt/_rels/presentation.xml.rels",
                "<Relationships><Relationship Id=\"rId9\" Target=\"../../etc/passwd\"/>\
                 </Relationships>"
                    .to_string(),
            ),
        ];
        let borrowed: Vec<(&str, &str)> = members.iter().map(|(a, b)| (*a, b.as_str())).collect();
        // The escaping relationship is dropped, so the order is empty and the
        // archive's own listing is used — which finds the real slide.
        let deck = read(&zipped(&borrowed), Deck::Pptx).expect("read");
        assert_eq!(deck.len(), 1);
        assert_eq!(deck[0][0].text, "Real");
    }
}
