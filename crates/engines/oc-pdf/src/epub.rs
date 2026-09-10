//! Reading an EPUB into the paragraph model.
//!
//! # Three files deep, and every step is stated by the book
//!
//! `META-INF/container.xml` names the OPF package document. The OPF carries a
//! **manifest** (id to href) and a **spine** (itemrefs, in reading order). The
//! spine is what says which chapter comes first, and it is authoritative:
//! filenames are not, and neither is the archive's own order.
//!
//! Each spine item is XHTML, which is what [`crate::htmlread`] already reads —
//! headings, lists, code blocks and quotations included. That module was
//! written for `html -> *` and this is the second thing it pays for.
//!
//! # DRM
//!
//! A `META-INF/encryption.xml` means the book's content is encrypted. This
//! refuses it **by name** rather than producing a document of whatever
//! happened to be readable. Removing DRM is not something this product does,
//! and half-converting a protected book is worse than refusing it.
//!
//! # What is dropped
//!
//! Images, styling, the cover, page-list and navigation documents, and every
//! piece of the container that is not a spine item. What comes out is the
//! book's prose in its reading order.

use crate::text::Paragraph;

/// The most spine items this will follow.
///
/// A book of five thousand chapters is not a book; this bounds a loop over
/// attacker-supplied structure.
const MAX_ITEMS: usize = 5000;

/// Read a book's chapters, one `Vec<Paragraph>` per spine item.
///
/// # Errors
///
/// Not a ZIP, no container or package document, DRM-protected, or a part that
/// decompresses past the cap `slides::read_part` enforces.
pub fn read(bytes: &[u8]) -> Result<Vec<Vec<Paragraph>>, String> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("this file is not a readable ZIP container: {e}"))?;

    // BEFORE ANYTHING IS READ. A protected book must be refused, not partly
    // converted, and the check is one lookup in the archive's index.
    if zip.index_for_name("META-INF/encryption.xml").is_some() {
        return Err(
            "this EPUB is DRM-protected: its contents are encrypted and this build does not \
             remove protection. Convert a copy you can open without it."
                .into(),
        );
    }

    let container = crate::slides::read_part(&mut zip, "META-INF/container.xml")
        .map_err(|_| "this archive has no META-INF/container.xml; it is not an EPUB".to_string())?;
    let opf_path = rootfile(&container)
        .ok_or("this EPUB's container names no package document".to_string())?;
    let opf = crate::slides::read_part(&mut zip, &opf_path)
        .map_err(|_| format!("this EPUB names {opf_path} as its package document and has none"))?;

    // Hrefs in the manifest are relative to the OPF, not to the archive root.
    // A book whose package sits in `OEBPS/content.opf` lists `chapter1.xhtml`
    // and means `OEBPS/chapter1.xhtml`.
    let base = opf_path.rsplit_once('/').map_or("", |(dir, _)| dir);

    let mut out = Vec::new();
    for href in spine_hrefs(&opf).into_iter().take(MAX_ITEMS) {
        let Some(path) = join(base, &href) else {
            continue;
        };
        let Ok(xhtml) = crate::slides::read_part(&mut zip, &path) else {
            continue;
        };
        let chapter = crate::htmlread::parse(&xhtml);
        if !chapter.is_empty() {
            out.push(chapter);
        }
    }
    if out.is_empty() {
        return Err("this EPUB's spine leads to no readable text".into());
    }
    Ok(out)
}

/// The package document's path, from the container.
fn rootfile(container: &str) -> Option<String> {
    crate::slides::tags(container)
        .filter(|t| t.starts_with("<rootfile"))
        .find_map(|t| crate::slides::attribute(t, "full-path"))
        .map(ToString::to_string)
}

/// Every spine item's href, in spine order.
///
/// # Why both halves are needed
///
/// The spine names manifest **ids**, not paths, and the manifest maps ids to
/// hrefs. Reading only the manifest gives every document in the book including
/// the navigation and the cover, in whatever order they were listed; reading
/// only the spine gives ids that name nothing.
fn spine_hrefs(opf: &str) -> Vec<String> {
    let mut manifest: Vec<(&str, &str)> = Vec::new();
    let mut order: Vec<&str> = Vec::new();

    for tag in crate::slides::tags(opf) {
        if tag.starts_with("<item ") || tag.starts_with("<item\t") || tag.starts_with("<item\n") {
            if let (Some(id), Some(href)) = (
                crate::slides::attribute(tag, "id"),
                crate::slides::attribute(tag, "href"),
            ) {
                manifest.push((id, href));
            }
        } else if tag.starts_with("<itemref") {
            // `linear="no"` marks material outside the primary reading order —
            // pop-up footnotes and the like. Kept: a footnote is still the
            // book's text, and dropping it would lose content silently, which
            // is the failure this whole module is arranged against.
            if let Some(idref) = crate::slides::attribute(tag, "idref") {
                order.push(idref);
            }
        }
    }

    order
        .into_iter()
        .filter_map(|id| {
            manifest
                .iter()
                .find(|(mid, _)| *mid == id)
                .map(|(_, href)| (*href).to_string())
        })
        .collect()
}

/// Resolve an href against the package document's directory.
///
/// Returns `None` for anything that climbs out of the archive or is absolute.
/// A name this code would have to repair is a name somebody wanted repaired.
fn join(base: &str, href: &str) -> Option<String> {
    // A fragment addresses a place inside a document, not another document.
    let href = href.split('#').next().unwrap_or(href);
    if href.is_empty() || href.starts_with('/') || href.contains("..") || href.contains("://") {
        return None;
    }
    Some(if base.is_empty() {
        href.to_string()
    } else {
        format!("{base}/{href}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Style;
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

    const CONTAINER: &str = "<container><rootfiles>\
        <rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/>\
        </rootfiles></container>";

    fn book() -> Vec<u8> {
        // THE SPINE IS NOT THE MANIFEST ORDER, and neither is alphabetical:
        // the book reads two, then one, then three.
        let opf = "<package><manifest>\
            <item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\"/>\
            <item id=\"c1\" href=\"one.xhtml\" media-type=\"application/xhtml+xml\"/>\
            <item id=\"c2\" href=\"two.xhtml\" media-type=\"application/xhtml+xml\"/>\
            <item id=\"c3\" href=\"three.xhtml\" media-type=\"application/xhtml+xml\"/>\
            </manifest><spine>\
            <itemref idref=\"c2\"/><itemref idref=\"c1\"/><itemref idref=\"c3\"/>\
            </spine></package>";
        zipped(&[
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("OEBPS/content.opf", opf),
            (
                "OEBPS/nav.xhtml",
                "<html><body><nav><ol><li>Contents</li></ol></nav></body></html>",
            ),
            (
                "OEBPS/one.xhtml",
                "<html><body><h1>Chapter One</h1><p>The first.</p></body></html>",
            ),
            (
                "OEBPS/two.xhtml",
                "<html><body><h1>Chapter Two</h1><p>The second.</p></body></html>",
            ),
            (
                "OEBPS/three.xhtml",
                "<html><body><h1>Chapter Three</h1><ul><li>a point</li></ul></body></html>",
            ),
        ])
    }

    /// The trap: reading order comes from the spine, and the spine here
    /// deliberately disagrees with both the manifest and the alphabet.
    #[test]
    fn chapters_come_out_in_spine_order() {
        let chapters = read(&book()).expect("read the book");
        let titles: Vec<&str> = chapters.iter().map(|c| c[0].text.as_str()).collect();
        assert_eq!(titles, ["Chapter Two", "Chapter One", "Chapter Three"]);
        // And the navigation document is not a spine item, so it is absent.
        assert!(!titles.contains(&"Contents"));
    }

    /// XHTML structure survives, because `htmlread` reads it rather than
    /// stripping it. Before that module existed, an EPUB would have converted
    /// to an empty file.
    #[test]
    fn a_chapter_keeps_its_headings_and_lists() {
        let chapters = read(&book()).expect("read");
        assert_eq!(chapters[0][0].style, Style::Heading(1));
        assert_eq!(chapters[2][1].style, Style::Bullet { depth: 0 });
        assert_eq!(chapters[2][1].text, "a point");
    }

    #[test]
    fn a_protected_book_is_refused_by_name() {
        let mut members: Vec<(&str, &str)> = vec![
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            ("META-INF/encryption.xml", "<encryption/>"),
            ("OEBPS/content.opf", "<package/>"),
        ];
        members.sort_by_key(|(n, _)| *n);
        let err = read(&zipped(&members)).expect_err("DRM must be refused");
        assert!(err.contains("DRM"), "{err}");
        assert!(
            err.contains("does not remove"),
            "the refusal must say why, not just that: {err}"
        );
    }

    #[test]
    fn an_href_escaping_the_archive_is_not_followed() {
        assert_eq!(join("OEBPS", "one.xhtml"), Some("OEBPS/one.xhtml".into()));
        assert_eq!(join("", "one.xhtml"), Some("one.xhtml".into()));
        assert_eq!(join("OEBPS", "a.xhtml#top"), Some("OEBPS/a.xhtml".into()));
        assert_eq!(join("OEBPS", "../../etc/passwd"), None);
        assert_eq!(join("OEBPS", "/etc/passwd"), None);
        assert_eq!(join("OEBPS", "https://example.com/x"), None);
    }

    #[test]
    fn a_book_with_no_readable_text_is_refused_rather_than_empty() {
        let members = [
            ("mimetype", "application/epub+zip"),
            ("META-INF/container.xml", CONTAINER),
            (
                "OEBPS/content.opf",
                "<package><manifest/><spine/></package>",
            ),
        ];
        let err = read(&zipped(&members)).expect_err("nothing to convert");
        assert!(err.contains("no readable text"), "{err}");
    }
}
