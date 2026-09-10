//! Identifying a file from its leading bytes — **pure**, and therefore here.
//!
//! # Why this moved
//!
//! This logic lived in `openconvert-run::detect`, wrapped in `Read + Seek`. But
//! the wrapping was the only impure part: matching a signature, separating
//! Matroska from WebM by DocType, and recognising CSV by shape are all
//! functions from `&[u8]` to a [`FormatId`]. `03` §4 says pure logic belongs in
//! the pure core, and the test for "pure" is whether a second caller can use it
//! without dragging the filesystem along.
//!
//! A second caller now does: the browser tool pages compile this crate to
//! `wasm32` and hand it the first few hundred bytes of a dropped file, with no
//! `Read`, no `Seek` and no filesystem anywhere. `openconvert-run::detect` is a
//! thin adapter over these functions, so the desktop app and the web page
//! cannot disagree about what a file is — which would otherwise be a very easy
//! divergence to ship and a very hard one to notice.
//!
//! # What it deliberately does not do
//!
//! It does not read the extension. An extension is a claim made by whoever
//! named the file; it is recorded for **mismatch detection** and never used to
//! route. That separation is `SR-4`, and keeping the extension out of this
//! module is how it stays true.

use crate::format::{Family, FormatId, TABLE};

/// What the leading bytes say this file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ByteSniff {
    /// The format the content indicates.
    pub detected: FormatId,
    /// The file is validly two *different* things. An attack, not an ambiguity.
    pub polyglot: bool,
}

/// Identify a file from its head buffer.
///
/// `head` should hold at least [`crate::format::max_magic_reach`] bytes. A
/// shorter buffer is not an error — a short file is a short file, and the
/// signatures that reach past its end simply do not match.
#[must_use]
pub fn sniff_bytes(head: &[u8]) -> ByteSniff {
    let matches = matching_formats(head);
    let (detected, polyglot) = resolve(&matches, head);
    ByteSniff { detected, polyglot }
}

/// Every format whose signature matches. More than one needs deciding.
#[must_use]
pub fn matching_formats(head: &[u8]) -> Vec<FormatId> {
    let mut hits: Vec<FormatId> = TABLE
        .iter()
        .filter(|f| {
            f.magic.iter().any(|m| {
                head.len() >= m.offset + m.bytes.len()
                    && &head[m.offset..m.offset + m.bytes.len()] == m.bytes
            })
        })
        .map(|f| f.id)
        .collect();
    hits.dedup();
    hits
}

/// Decide what the file is, and whether the multiple matches are an attack.
///
/// # The distinction this function exists for
///
/// `polyglot` used to be `matches.len() > 1`. Two families in the format table
/// share a signature **by design** and say so in their own comments, so that
/// test fired on every member of both: every Matroska file and every ZIP
/// archive were refused, and a plain `.zip` was additionally reported as
/// `docx` because the first matching table row won.
///
/// A polyglot is a file that is validly two *different* things, and it is an
/// attack. Several members of one family is an ordinary file needing one more
/// read. Only the first is refused.
fn resolve(matches: &[FormatId], head: &[u8]) -> (FormatId, bool) {
    let Some(&first) = matches.first() else {
        return (structural(head), false);
    };
    if matches.len() == 1 {
        return (first, false);
    }

    // Every match in one family: not a polyglot, just undecided.
    if let Some(family) = first.family() {
        if matches.iter().all(|f| f.family() == Some(family)) {
            let decided = match family {
                Family::Ebml => ebml_doctype(head),
                Family::Zip => zip_member(head),
                Family::Tiff => tiff_flavour(head),
            };
            // A family we could not settle falls back to the table's first
            // match rather than refusing. It is still one of these formats, and
            // the route table's own requirements are what refuse it if the
            // guess is unusable -- whereas a refusal here would reject a
            // perfectly ordinary file for being ambiguous.
            return (decided.unwrap_or(first), false);
        }
    }

    // Matches spanning families, or unrelated formats. This is the case the
    // flag was written for.
    (first, true)
}

/// Matroska or WebM, from the DocType element in the EBML header.
///
/// The two are byte-identical at the signature; the table's own comment says
/// they are "separated by the DocType element, not by magic". This is that
/// separation.
fn ebml_doctype(head: &[u8]) -> Option<FormatId> {
    // `DocType` is a short ASCII string, and the head buffer covers the EBML
    // header comfortably. Searching for the value rather than walking the
    // element tree keeps this to one pass over a few hundred bytes, and the
    // full tree walk lives where it is needed for codecs.
    let window = &head[..head.len().min(1024)];
    if find(window, b"webm").is_some() {
        return Some(FormatId::Webm);
    }
    if find(window, b"matroska").is_some() {
        return Some(FormatId::Mkv);
    }
    None
}

/// Which TIFF-shaped format this is.
///
/// Three separations, in decreasing order of how much the bytes tell us:
///
/// 1. **CR2 and NEF settle themselves.** Their signatures continue past
///    TIFF's four bytes into a literal marker, so matching them at offset 0 is
///    the whole decision -- no IFD walk, no guess.
/// 2. **DNG is a TIFF with the `DNGVersion` tag**, and nothing else
///    distinguishes it. The specification says a DNG *is* a TIFF/EP file; the
///    tag is the only thing that makes it a DNG, so the tag is what is read.
/// 3. **Anything else with TIFF's magic is a TIFF**, which is a decision and
///    not a fallback: a file with TIFF's header and no RAW marker is exactly
///    what "TIFF" means.
fn tiff_flavour(head: &[u8]) -> Option<FormatId> {
    if head.starts_with(b"II*\x00\x10\x00\x00\x00CR")
        || head.starts_with(b"MM\x00\x2a\x10\x00\x00\x00CR")
    {
        return Some(FormatId::Cr2);
    }
    if head.starts_with(b"II*\x00\x10\x00\x00\x00NEF") {
        return Some(FormatId::Nef);
    }
    if has_dng_version_tag(head) {
        return Some(FormatId::Dng);
    }
    Some(FormatId::Tiff)
}

/// The `DNGVersion` tag (0xC612) in IFD0.
///
/// A bounded walk over a structure an attacker controls, so every read is
/// checked against the buffer we actually hold and the entry count is capped
/// rather than trusted -- a two-byte field can claim 65,535 entries. Running
/// out of buffer is a plain `false`: not finding the tag is the answer for
/// every TIFF that is not a DNG anyway.
fn has_dng_version_tag(head: &[u8]) -> bool {
    const DNG_VERSION: u16 = 0xC612;
    const MAX_ENTRIES: usize = 512;

    if head.len() < 8 {
        return false;
    }
    let big_endian = match &head[0..2] {
        b"MM" => true,
        b"II" => false,
        _ => return false,
    };
    let u16_at = |o: usize| -> u16 {
        let b = [head[o], head[o + 1]];
        if big_endian {
            u16::from_be_bytes(b)
        } else {
            u16::from_le_bytes(b)
        }
    };
    let u32_at = |o: usize| -> u32 {
        let b = [head[o], head[o + 1], head[o + 2], head[o + 3]];
        if big_endian {
            u32::from_be_bytes(b)
        } else {
            u32::from_le_bytes(b)
        }
    };

    let ifd0 = u32_at(4) as usize;
    if ifd0 < 8 || ifd0.saturating_add(2) > head.len() {
        return false;
    }
    let entries = u16_at(ifd0) as usize;
    for i in 0..entries.min(MAX_ENTRIES) {
        // 2 bytes of count, then 12 per entry; the tag is the first field.
        let Some(at) = ifd0.checked_add(2).and_then(|b| b.checked_add(i * 12)) else {
            return false;
        };
        if at.saturating_add(12) > head.len() {
            return false;
        }
        if u16_at(at) == DNG_VERSION {
            return true;
        }
    }
    false
}

/// Which ZIP-shaped format this is, from its first member.
///
/// OOXML puts `[Content_Types].xml` at the front; ODF requires an uncompressed
/// `mimetype` member first, with the media type as its literal content. A
/// plain archive has neither.
///
/// **Defaults to `Zip`**, which is the safe direction: an OOXML file read as an
/// archive is handled by the archive path, which is bounded and sandboxed. The
/// previous behaviour defaulted to `Docx` and sent every `.zip` down the
/// document path.
fn zip_member(head: &[u8]) -> Option<FormatId> {
    // OPENDOCUMENT AND EPUB SAY WHAT THEY ARE, IN THE CLEAR, FIRST.
    //
    // Both specifications require a `mimetype` member stored uncompressed as
    // the first entry, precisely so a reader can identify the file without
    // decompressing anything. So these are exact rather than inferred, and they
    // are checked first for that reason.
    for (marker, id) in [
        (
            &b"application/vnd.oasis.opendocument.text"[..],
            FormatId::Odt,
        ),
        (
            b"application/vnd.oasis.opendocument.presentation",
            FormatId::Odp,
        ),
        (
            b"application/vnd.oasis.opendocument.spreadsheet",
            FormatId::Ods,
        ),
        (b"application/epub+zip", FormatId::Epub),
    ] {
        if find(head, marker).is_some() {
            return Some(id);
        }
    }

    // AN EPUB HAS A SECOND MARKER THAT IS NOT COMPRESSIBLE.
    //
    // The `mimetype` check above reads a member's CONTENTS, which is only in
    // the clear because the specification requires that member stored. A
    // non-conforming writer that deflates it defeats every content marker at
    // once -- and an entry NAME is never compressed, so this one holds
    // regardless. `META-INF/container.xml` is required in every EPUB and
    // appears in no other format here.
    //
    // The three OpenDocument formats get no equivalent, and cannot: an ODT, an
    // ODP and an ODS carry the same members under the same names, and only the
    // mimetype separates them. A non-conforming one is detected as a zip, which
    // is what it looks like.
    if find(head, b"META-INF/container.xml").is_some() {
        return Some(FormatId::Epub);
    }

    // OOXML DOES NOT, so it is identified by its PART NAMES.
    //
    // This used to read `[Content_Types].xml` and answer `Docx`, which was
    // right for the one OOXML format in the table and wrong the moment there
    // were three: that member is in every one of them. Worse, it is not
    // reliably in the window — Excel writes it LAST, so a real `.xlsx` on this
    // machine has no `[Content_Types].xml` in its first 4 KB at all and was
    // detected as a plain zip.
    //
    // The part-name prefixes are the OOXML convention and they ARE in the
    // window: every file checked — three `.docx`, three `.xlsx`, one `.pptx` —
    // carries `word/`, `xl/` or `ppt/` inside the first 4 KB, because those are
    // the entry names in the local headers rather than anything compressed.
    for (prefix, id) in [
        (&b"word/"[..], FormatId::Docx),
        (b"ppt/", FormatId::Pptx),
        (b"xl/", FormatId::Xlsx),
    ] {
        if find(head, prefix).is_some() {
            return Some(id);
        }
    }

    // An OOXML file whose parts are all past the window. Which one it is is not
    // knowable from here, and `Docx` is the answer this has always given.
    if find(head, b"[Content_Types].xml").is_some() {
        return Some(FormatId::Docx);
    }
    Some(FormatId::Zip)
}

/// Whether this text opens the way a DXF does.
///
/// The first pair of a DXF is group code `0` with the value `SECTION`, and the
/// second is code `2` naming which section. Checking the whole opening rather
/// than the word `SECTION` is what keeps a document that merely contains it out
/// of the CAD reader.
fn is_dxf(text: &str) -> bool {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    lines.next() == Some("0")
        && lines.next() == Some("SECTION")
        && lines.next() == Some("2")
        && matches!(
            lines.next(),
            Some("HEADER" | "CLASSES" | "TABLES" | "BLOCKS" | "ENTITIES" | "OBJECTS")
        )
}

/// First index of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Formats with no signature, identified by shape.
///
/// CSV and JSON have no magic bytes and never will. SVG is XML, whose leading
/// bytes vary with the declaration.
fn structural(head: &[u8]) -> FormatId {
    let Ok(text) = core::str::from_utf8(head) else {
        return FormatId::Unknown;
    };
    let t = text.trim_start();
    if t.starts_with('{') || t.starts_with('[') {
        // A NOTEBOOK IS JSON, and this is where the two are separated.
        //
        // Not through `Family`: that dispatch runs over formats whose MAGIC
        // matched, and neither JSON nor a notebook has any. Signature-less
        // formats are decided here, so the structural check belongs here too.
        //
        // Two markers, not one. `"cells"` alone matches any file with a field
        // of that name, and misrouting somebody's data file to the document
        // engine would cost it its `json -> csv` route. `"cell_type"` appears
        // in the first cell and pins it: together they are a notebook or a
        // deliberate imitation of one.
        //
        // `nbformat` is deliberately NOT the marker even though the format
        // requires it — Jupyter writes it AFTER `cells`, so in any real
        // notebook it sits past the end of the sniff window.
        if t.contains("\"cells\"") && t.contains("\"cell_type\"") {
            return FormatId::Ipynb;
        }
        return FormatId::Json;
    }
    if t.starts_with("<?xml") || t.starts_with("<svg") {
        // Only claim SVG if the root element actually says so.
        if t.contains("<svg") {
            return FormatId::Svg;
        }
        return FormatId::Unknown;
    }
    // A DXF OPENS `0` / `SECTION`, two ordinary lines of text — so it is
    // decided here with the other signature-less formats, and BEFORE the CSV
    // check, which a file of short one-token lines could otherwise claim.
    //
    // All four lines are required. `SECTION` alone appears in plenty of prose;
    // the group code `0` on its own line immediately before it, followed by
    // code `2` and a section name, does not.
    if is_dxf(t) {
        return FormatId::Dxf;
    }
    if looks_like_csv(t) {
        return FormatId::Csv;
    }
    // TEXT IS THE LAST RESORT, and it is what makes `md -> pdf` reachable.
    //
    // Nothing identifies plain text — that is what "plain" means — so before
    // this it fell through to `Unknown` and every `.md`, `.rst`, `.py` and
    // `README` was refused with "the content did not match any format we
    // know". The writers that produce PDF, DOCX and ODT existed and had no
    // input that could reach them.
    //
    // The test is deliberately strict: valid UTF-8 AND no control characters
    // beyond tab, newline and carriage return. A binary file fails both, so
    // this cannot turn a truncated JPEG into a document; a source file, a
    // README or a log passes, which is the intent.
    if is_plain_text(text) {
        return FormatId::Txt;
    }
    FormatId::Unknown
}

/// Whether this looks like human-readable text rather than binary.
///
/// The classic heuristic, and it is used here for the classic reason: there is
/// no signature to check. A NUL or a stray `0x01` is the strongest single
/// signal that a file is not text, and every format with an actual signature
/// has already been tried by the time this runs.
fn is_plain_text(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
}
/// Two or more delimited fields on the first line, and no control characters.
///
/// Deliberately conservative: CSV is the format most likely to be *guessed*
/// wrongly, and a false CSV routes a binary file into a text parser.
fn looks_like_csv(text: &str) -> bool {
    let Some(first) = text.lines().next() else {
        return false;
    };
    if first.is_empty() || first.chars().any(|c| c.is_control() && c != '\t') {
        return false;
    }
    first.matches(',').count() >= 1 && first.chars().all(|c| !c.is_ascii_control())
}

/// Map a filename extension to a format — for **mismatch detection only**.
///
/// Never used to route. `SR-4` requires the mismatch be surfaced, which needs
/// the extension's claim as a value to compare against, and nothing more.
#[must_use]
pub fn from_extension(name: &str) -> Option<FormatId> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    TABLE.iter().find(|f| f.extension == ext).map(|f| f.id)
}

#[cfg(test)]
mod tests {
    use super::{from_extension, sniff_bytes};
    use crate::format::FormatId;

    #[test]
    fn a_png_signature_is_a_png() {
        let head = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(sniff_bytes(head).detected, FormatId::Png);
        assert!(!sniff_bytes(head).polyglot);
    }

    /// The head buffer reaches offset 257 because of tar, and a file shorter
    /// than a signature's reach must not match it.
    ///
    /// `b"ust"` is now `Txt` rather than `Unknown`, and that is the point of
    /// the change rather than a regression: it is three printable characters,
    /// which is text. What this test still guards is that it does not match
    /// TAR, whose `ustar` magic sits at offset 257 and which a naive reach
    /// check would have found here.
    #[test]
    fn a_short_file_matches_nothing_it_cannot_reach() {
        assert_ne!(sniff_bytes(b"ust").detected, FormatId::Tar);
        assert_eq!(sniff_bytes(b"ust").detected, FormatId::Txt);
        // Empty is not text. There is nothing to convert, and calling it a
        // document would offer routes over no content.
        assert_eq!(sniff_bytes(b"").detected, FormatId::Unknown);
    }

    /// Binary that happens to reach no signature stays `Unknown`.
    ///
    /// The guard on the text fallback: a NUL or a control byte means this is
    /// not prose, whatever else it may be. Without this, every unrecognised
    /// binary would be offered as a document.
    #[test]
    fn binary_does_not_become_text() {
        assert_eq!(
            sniff_bytes(b"\x00\x01\x02\x03rubbish").detected,
            FormatId::Unknown
        );
        assert_eq!(
            sniff_bytes(b"plain then \x07 a bell").detected,
            FormatId::Unknown
        );
        // Invalid UTF-8 is not text either.
        assert_eq!(
            sniff_bytes(&[0xff, 0xfe, 0xfd, 0xfc]).detected,
            FormatId::Unknown
        );
    }

    /// One family sharing a signature is an ordinary file, not an attack.
    #[test]
    fn ebml_family_is_decided_not_refused() {
        let mut head = vec![0x1A, 0x45, 0xDF, 0xA3];
        head.extend_from_slice(b"\x42\x82\x88matroska");
        let s = sniff_bytes(&head);
        assert_eq!(s.detected, FormatId::Mkv);
        assert!(!s.polyglot, "one family is not a polyglot");

        let mut head = vec![0x1A, 0x45, 0xDF, 0xA3];
        head.extend_from_slice(b"\x42\x82\x84webm");
        assert_eq!(sniff_bytes(&head).detected, FormatId::Webm);
    }

    /// A plain archive is a ZIP, not a DOCX. Defaulting the other way sent
    /// every `.zip` down the document path.
    #[test]
    fn a_plain_zip_is_a_zip() {
        let head = b"PK\x03\x04\x14\x00\x00\x00\x00\x00hello.txt";
        assert_eq!(sniff_bytes(head).detected, FormatId::Zip);
    }

    #[test]
    fn ooxml_is_recognised_by_its_first_member() {
        let mut head = b"PK\x03\x04\x14\x00\x00\x00\x00\x00".to_vec();
        head.extend_from_slice(b"[Content_Types].xml");
        assert_eq!(sniff_bytes(&head).detected, FormatId::Docx);
    }

    #[test]
    fn structural_formats_are_recognised_by_shape() {
        assert_eq!(sniff_bytes(b"{\"a\": 1}").detected, FormatId::Json);
        assert_eq!(sniff_bytes(b"name,amount\nx,1\n").detected, FormatId::Csv);
        assert_eq!(
            sniff_bytes(b"<?xml version=\"1.0\"?><svg xmlns=\"...\">").detected,
            FormatId::Svg
        );
    }

    /// Prose is not CSV. The guess has to stay conservative, because a false
    /// CSV routes a structured parser at something that is not a table.
    ///
    /// It IS text, which is a separate claim and a much weaker one: `Txt` says
    /// only "printable characters", and every document route from it goes
    /// through a Markdown parser that treats unmarked prose as prose.
    #[test]
    fn prose_is_not_csv() {
        assert_eq!(
            sniff_bytes(b"Hello there\nsecond line\n").detected,
            FormatId::Txt
        );
    }

    /// Markdown is text, and nothing tries to prove otherwise.
    ///
    /// This is the whole of option (a): a `.md` cannot be told from a `.txt` by
    /// content, so neither is guessed at. Both arrive as `Txt` and both are
    /// parsed as CommonMark, which plain prose trivially is. Before this,
    /// `md -> pdf` was refused with "the content did not match any format we
    /// know" while the PDF writer sat one call away.
    #[test]
    fn markdown_arrives_as_text_rather_than_being_refused() {
        let md = b"# Title\n\nSome prose.\n\n- a list item\n";
        assert_eq!(sniff_bytes(md).detected, FormatId::Txt);
    }

    #[test]
    fn the_extension_is_a_claim_and_only_a_claim() {
        assert_eq!(from_extension("photo.PNG"), Some(FormatId::Png));
        assert_eq!(from_extension("photo"), None);
        // An EPS renamed to .jpg: the extension says jpeg, the bytes say
        // otherwise, and only the bytes route.
        assert_eq!(from_extension("payload.jpg"), Some(FormatId::Jpeg));
        assert_eq!(
            sniff_bytes(b"%!PS-Adobe-3.0 EPSF-3.0").detected,
            FormatId::PostScript
        );
    }
}

#[cfg(test)]
mod family_tests {
    //! Moved here with `resolve` itself. These are byte-level tests of pure
    //! logic and they belong beside it; they lived in `openconvert-run::detect`
    //! only because the function did.
    use super::{resolve, sniff_bytes};
    use crate::format::FormatId;

    fn ebml(doctype: &str) -> Vec<u8> {
        // EBML header, then a DocType element carrying the name.
        let mut v = vec![0x1A, 0x45, 0xDF, 0xA3];
        let body_len = 3 + doctype.len();
        v.push(0x80 | body_len as u8);
        v.extend_from_slice(&[0x42, 0x82]);
        v.push(0x80 | doctype.len() as u8);
        v.extend_from_slice(doctype.as_bytes());
        v
    }

    fn zip_with(first_member: &str) -> Vec<u8> {
        let mut v = vec![b'P', b'K', 0x03, 0x04];
        v.extend_from_slice(&[0u8; 22]);
        v.extend_from_slice(first_member.as_bytes());
        v.resize(300, 0);
        v
    }

    /// **Every Matroska file was refused.**
    ///
    /// `Mkv` and `Webm` are byte-identical at the signature, so
    /// `matches.len() > 1` flagged all of them as polyglots and `route()`
    /// refused. That is `01` s154's flagship format, unconvertible.
    #[test]
    fn matroska_and_webm_are_separated_by_doctype_not_refused() {
        let (mkv, poly) = resolve(&[FormatId::Mkv, FormatId::Webm], &ebml("matroska"));
        assert_eq!(mkv, FormatId::Mkv);
        assert!(!poly, "one family is not a polyglot");

        let (webm, poly) = resolve(&[FormatId::Mkv, FormatId::Webm], &ebml("webm"));
        assert_eq!(webm, FormatId::Webm);
        assert!(!poly);
    }

    /// **A plain ZIP was reported as `docx`.**
    ///
    /// Three rows share `PK`, and the detector took the first that
    /// matched — which is `Docx`, because that is the table's order. So every
    /// archive was both misidentified and refused.
    #[test]
    fn a_plain_zip_is_a_zip_and_an_ooxml_file_is_not() {
        let all = [FormatId::Docx, FormatId::Odt, FormatId::Zip];

        let (id, poly) = resolve(&all, &zip_with("hello.txt"));
        assert_eq!(id, FormatId::Zip, "a plain archive is a Zip");
        assert!(!poly);

        let (id, _) = resolve(&all, &zip_with("[Content_Types].xml"));
        assert_eq!(id, FormatId::Docx);

        let (id, _) = resolve(
            &all,
            &zip_with("mimetypeapplication/vnd.oasis.opendocument.text"),
        );
        assert_eq!(id, FormatId::Odt);
    }

    /// **The control: a real polyglot is still refused.**
    ///
    /// Without this, the fix above is indistinguishable from deleting the flag.
    /// Two formats from different families is the case it was written for.
    #[test]
    fn matches_spanning_families_are_still_a_polyglot() {
        let (_, poly) = resolve(&[FormatId::Gif, FormatId::Zip], b"GIF89a");
        assert!(poly, "a GIF that is also a ZIP is an attack, not a family");

        let (_, poly) = resolve(&[FormatId::Mkv, FormatId::Docx], b"whatever");
        assert!(poly, "matches spanning two families are a polyglot");
    }

    /// A single match is never a polyglot, and no match falls to structure.
    #[test]
    fn one_match_and_no_match_behave_as_before() {
        let (id, poly) = resolve(&[FormatId::Png], b"");
        assert_eq!(id, FormatId::Png);
        assert!(!poly);

        let (id, poly) = resolve(&[], b"{\"a\": 1}");
        assert_eq!(id, FormatId::Json, "structural detection still runs");
        assert!(!poly);
    }

    /// An unsettleable family falls back rather than refusing.
    ///
    /// A Matroska file whose DocType sits past the head buffer is still one of
    /// two formats we support. Refusing it here would reject an ordinary file
    /// for being ambiguous; the route table's requirements are what refuse a
    /// guess that turns out unusable.
    #[test]
    fn an_unsettleable_family_falls_back_instead_of_refusing() {
        let (id, poly) = resolve(&[FormatId::Mkv, FormatId::Webm], &[0x1A, 0x45, 0xDF, 0xA3]);
        assert_eq!(id, FormatId::Mkv, "the table's first match");
        assert!(!poly, "ambiguity within a family is not an attack");
    }

    /// A little-endian TIFF with IFD0 at offset 8 and the given tags.
    #[cfg(test)]
    fn tiff_with_tags(tags: &[u16]) -> Vec<u8> {
        let mut out = Vec::from(*b"II*\x00");
        out.extend_from_slice(&8_u32.to_le_bytes());
        out.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for &tag in tags {
            out.extend_from_slice(&tag.to_le_bytes()); // tag
            out.extend_from_slice(&3_u16.to_le_bytes()); // type: SHORT
            out.extend_from_slice(&1_u32.to_le_bytes()); // count
            out.extend_from_slice(&0_u32.to_le_bytes()); // value
        }
        out.extend_from_slice(&0_u32.to_le_bytes()); // no next IFD
        out
    }

    /// The bug: every TIFF-shaped file was refused as a polyglot.
    ///
    /// `Dng`'s signature is byte-identical to `Tiff`'s, so a plain TIFF matched
    /// two rows and `route()` quarantined it — taking all five `Tiff ->` rows
    /// with it, plus both `Dng ->`, both `Cr2 ->` and both `Nef ->`. Eleven
    /// routes that could never fire, on formats this project itself writes.
    #[test]
    fn a_plain_tiff_is_a_tiff_and_not_an_attack() {
        let bytes = tiff_with_tags(&[0x0100, 0x0101, 0x0102]);
        let got = sniff_bytes(&bytes);
        assert_eq!(got.detected, FormatId::Tiff);
        assert!(
            !got.polyglot,
            "a TIFF sharing its signature with DNG is an ordinary file, not a polyglot"
        );
    }

    /// The tag is the only thing that makes a DNG a DNG.
    #[test]
    fn the_dngversion_tag_is_what_separates_dng_from_tiff() {
        // Tags sorted ascending, as TIFF requires — which is exactly why
        // 0xC612 sits at the end and why the head buffer had to grow.
        let bytes = tiff_with_tags(&[0x0100, 0x0101, 0x0102, 0xC612]);
        let got = sniff_bytes(&bytes);
        assert_eq!(got.detected, FormatId::Dng, "the tag is present");
        assert!(!got.polyglot);
    }

    /// CR2 and NEF settle themselves at offset 0, before any IFD walk.
    #[test]
    fn cr2_and_nef_are_decided_by_their_own_markers() {
        let mut cr2 = Vec::from(*b"II*\x00\x10\x00\x00\x00CR\x02\x00");
        cr2.resize(512, 0);
        assert_eq!(sniff_bytes(&cr2).detected, FormatId::Cr2);
        assert!(!sniff_bytes(&cr2).polyglot);

        let mut nef = Vec::from(*b"II*\x00\x10\x00\x00\x00NEF");
        nef.resize(512, 0);
        assert_eq!(sniff_bytes(&nef).detected, FormatId::Nef);
        assert!(!sniff_bytes(&nef).polyglot);
    }

    /// Narrowing the quarantine must not have opened it.
    ///
    /// TIFF's magic at offset 0 and tar's `ustar` at offset 257 is a file that
    /// is validly two *different* things, spanning two families. That is the
    /// case the flag was written for, and it still fires.
    #[test]
    fn a_tiff_that_is_also_a_tar_is_still_an_attack() {
        let mut bytes = tiff_with_tags(&[0x0100]);
        bytes.resize(512, 0);
        bytes[257..262].copy_from_slice(b"ustar");
        let got = sniff_bytes(&bytes);
        assert!(
            got.polyglot,
            "two families is an attack, however well one family now resolves"
        );
    }

    /// A hostile IFD cannot walk us off the end or make us read forever.
    #[test]
    fn a_hostile_ifd_header_is_bounded_not_trusted() {
        // Claims 65535 entries and supplies none.
        let mut bytes = Vec::from(*b"II*\x00");
        bytes.extend_from_slice(&8_u32.to_le_bytes());
        bytes.extend_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(sniff_bytes(&bytes).detected, FormatId::Tiff);

        // IFD0 pointing past the buffer, and pointing into its own header.
        for offset in [0_u32, 4, 0xFFFF_FFFF] {
            let mut b = Vec::from(*b"II*\x00");
            b.extend_from_slice(&offset.to_le_bytes());
            b.resize(64, 0);
            assert_eq!(
                sniff_bytes(&b).detected,
                FormatId::Tiff,
                "offset {offset} must not panic or mis-resolve"
            );
        }

        // Truncated mid-entry.
        let full = tiff_with_tags(&[0x0100, 0xC612]);
        for cut in 8..full.len() {
            let _ = sniff_bytes(&full[..cut]);
        }
    }
}
