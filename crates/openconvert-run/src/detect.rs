//! Phase 1 detection: **content, never extension**.
//!
//! SR-4. A PostScript file named `.jpg` routes as PostScript, the receipt
//! records both types, and the plan preview says so. The file-type icon is
//! drawn from the *detected* type, not the claimed one.
//!
//! Pure Rust, no `unsafe`, fuzzed. That is deliberate: this code reads the
//! first bytes of every hostile file the product will ever see, before any
//! decision about isolation has been made — so it is the one parser that
//! cannot itself be sandboxed.

use crate::handles::HandleTable;
use openconvert_core::facts::{FileFacts, Properties, Provenance, Sniff};
use openconvert_core::format::sniff_window;
use std::ffi::OsStr;
use std::io::{Read, Seek, SeekFrom};

/// Identify a stream by its content.
///
/// Takes `Read + Seek`, not `&[u8]`: sniffing needs the first few KB and
/// nothing else. **Nothing loads a file to decide what it is**, which is what
/// lets the multi-gigabyte demo work at all.
///
/// `hint` is the filename, used *only* to detect a mismatch — never to decide
/// identity.
pub fn sniff<R: Read + Seek>(src: &mut R, hint: Option<&OsStr>) -> std::io::Result<Sniff> {
    let mut head = vec![0_u8; sniff_window()];
    src.seek(SeekFrom::Start(0))?;
    let n = read_up_to(src, &mut head)?;
    head.truncate(n);
    src.seek(SeekFrom::Start(0))?;

    // The byte-level decision is PURE and lives in `openconvert-core::sniff`, so
    // the browser tool pages reach the same verdict from the same code. This
    // function is the I/O adapter and nothing more.
    let b = openconvert_core::sniff::sniff_bytes(&head);

    Ok(Sniff {
        detected: b.detected,
        declared: hint
            .and_then(std::ffi::OsStr::to_str)
            .and_then(openconvert_core::sniff::from_extension),
        polyglot: b.polyglot,
    })
}

/// Read until the buffer is full or the stream ends. A short file is not an
/// error — it is a short file.
fn read_up_to<R: Read>(src: &mut R, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

/// Open a file once, identify it, and hash it — **through the same handle**.
///
/// This is the function I1 names: nothing else may construct a [`FileFacts`].
/// It is also where I13/SR-16 becomes real rather than approximated. Three
/// things happen against **one open descriptor**:
///
/// 1. the content is sniffed,
/// 2. the bytes are hashed into `content_id`,
/// 3. the handle is parked in the [`HandleTable`] under a token,
///
/// so every later read goes to the same inode. That is what defeats
/// replace-by-rename — the realistic attack on a sync folder or a network
/// share, where the file that was routed on need not be the file converted.
///
/// # Errors
///
/// Any I/O failure opening or reading the file.
pub fn detect(
    path: &std::path::Path,
    table: &mut crate::handles::HandleTable,
) -> std::io::Result<FileFacts> {
    let mut file = std::fs::File::open(path)?;
    let s = sniff(&mut file, path.file_name())?;

    // Hash through the handle we already hold, not by re-opening the path.
    file.rewind()?;
    let mut hasher = blake3::Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    let content_id: [u8; 32] = *hasher.finalize().as_bytes();

    let provenance = provenance_of(path);
    file.rewind()?;
    let token = table.insert(file);

    Ok(FileFacts::new(
        token,
        content_id,
        s,
        Properties::None,
        provenance,
    ))
}

/// Where a file came from.
///
/// **Fails open, and that is stated rather than implied.** A file that reached
/// the disk by sync, from a non-NTFS volume, a USB stick, or an archive another
/// tool extracted looks `Trusted`, because the OS has told us nothing else. The
/// automatic floor raise is a useful signal, not a guarantee (`09` §8).
///
/// Captured here, **before any copy exists** — the mandated `create_new` copy
/// path strips `Zone.Identifier` (spike S26), so re-deriving this later would
/// report a downloaded file as local.
fn provenance_of(path: &std::path::Path) -> Provenance {
    #[cfg(windows)]
    {
        // Mark-of-the-web lives in an NTFS alternate data stream. If it is
        // there at all, the file came from outside.
        let ads = format!("{}:Zone.Identifier", path.display());
        if std::fs::metadata(&ads).is_ok() {
            return Provenance::Untrusted;
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
    Provenance::Trusted
}

/// Read the bytes `detect()` parked under this file's token.
///
/// **The only way to get a file's content**, and the reason `execute` and the
/// phase-2 probe cannot disagree about what they are looking at. There is no
/// path here to re-open: the handle is the one that produced `content_id`, so
/// "the bytes routed on are the bytes converted" is a property of the types
/// rather than a rule someone has to remember (I13/SR-16).
///
/// A rename-swap between detection and execution therefore changes nothing —
/// which is what SR-16 asks for, and what a second `File::open` would quietly
/// give up.
///
/// # Errors
///
/// [`std::io::ErrorKind::NotFound`] if the table does not hold the token, which
/// is structurally unreachable through `detect()` and an error rather than a
/// panic because the alternative is a crash on a forged token. Any underlying
/// read failure otherwise.
pub fn bytes_of(facts: &FileFacts, table: &mut HandleTable) -> std::io::Result<Vec<u8>> {
    let src = table.get_mut(facts.token()).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "internal: the input handle for this file is not open",
        )
    })?;
    src.rewind()?;
    let mut bytes = Vec::new();
    src.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    // The byte-level verdicts are tested in openconvert_core::sniff. What these
    // test is the ADAPTER: that seeking, short reads and the extension hint
    // behave, which is the half that stayed here.
    use openconvert_core::format::FormatId;

    fn sniff_bytes(b: &[u8], hint: Option<&str>) -> Sniff {
        sniff(&mut Cursor::new(b.to_vec()), hint.map(OsStr::new)).expect("sniff")
    }

    /// SR-4, the headline case: content wins over the name.
    #[test]
    fn postscript_named_jpg_routes_as_postscript() {
        let s = sniff_bytes(b"%!PS-Adobe-3.0\n", Some("invoice.jpg"));
        assert_eq!(s.detected, FormatId::PostScript);
        assert_eq!(s.declared, Some(FormatId::Jpeg));
        assert!(s.mismatched(), "the mismatch was not flagged");
    }

    #[test]
    fn png_is_png() {
        let s = sniff_bytes(b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0DIHDR", Some("a.png"));
        assert_eq!(s.detected, FormatId::Png);
        assert!(!s.mismatched());
    }

    /// tar's signature sits at offset 257 — the reason `max_magic_reach` is
    /// derived from the table rather than assumed. A sniffer reading 16 bytes
    /// would silently miss every tar file.
    #[test]
    fn deep_signature_is_reached() {
        let mut buf = vec![0_u8; 300];
        buf[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff_bytes(&buf, None).detected, FormatId::Tar);
    }

    /// A file shorter than the deepest signature must not panic or over-read.
    #[test]
    fn short_files_do_not_panic() {
        for len in 0..12 {
            let s = sniff_bytes(&vec![0x42; len], None);
            let _ = s.detected;
        }
        assert_eq!(sniff_bytes(b"BM", None).detected, FormatId::Bmp);
    }

    /// Structural formats have no magic and are recognised by shape.
    #[test]
    fn structural_formats_are_recognised() {
        assert_eq!(sniff_bytes(b"{\"a\":1}", None).detected, FormatId::Json);
        assert_eq!(sniff_bytes(b"a,b,c\n1,2,3\n", None).detected, FormatId::Csv);
        assert_eq!(
            sniff_bytes(br#"<?xml version="1.0"?><svg xmlns="x"/>"#, None).detected,
            FormatId::Svg
        );
    }

    /// The control for CSV: binary data must not be guessed as text.
    ///
    /// A false CSV routes a binary file into a text parser, which is exactly
    /// the confusion SR-4 exists to prevent.
    #[test]
    fn binary_is_not_guessed_as_csv() {
        assert_eq!(
            sniff_bytes(&[0x00, 0x01, 0x02, 0xFF, 0xFE], None).detected,
            FormatId::Unknown
        );
        assert_eq!(
            sniff_bytes(b"\xFF\xFE,\x00,\x01", None).detected,
            FormatId::Unknown
        );
    }

    /// **This test used to assert the bug.**
    ///
    /// It read *"A ZIP header is also how DOCX and ODT start"* and required
    /// `polyglot` to be true - pinning the behaviour that made every archive
    /// unconvertible and reported a plain `.zip` as `docx`. A green test was
    /// protecting the defect, which is why the defect survived until someone
    /// ran the binary against an actual zip file.
    ///
    /// `Zip`, `Docx` and `Odt` share a signature **by design**, and the table's
    /// own rows say so. That is a family, not a polyglot. The real polyglot
    /// case - matches spanning two families - is asserted in `family_tests`.
    #[test]
    fn a_shared_family_signature_is_not_a_polyglot() {
        // PK 03 04, then the version and flag bytes a real archive carries.
        let s = sniff_bytes(&[0x50, 0x4B, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00], None);
        assert!(
            !s.polyglot,
            "one family matching several rows is not an attack"
        );
        assert_eq!(
            s.detected,
            FormatId::Zip,
            "a bare ZIP header with no OOXML or ODF member is a Zip"
        );
    }

    /// An unnamed input has nothing to disagree with.
    #[test]
    fn no_hint_means_no_mismatch() {
        let s = sniff_bytes(b"\x89PNG\r\n\x1a\n", None);
        assert_eq!(s.declared, None);
        assert!(!s.mismatched());
    }

    /// Never panics, whatever the bytes. This function reads every hostile
    /// file the product will ever see, before isolation has been decided.
    #[test]
    fn arbitrary_bytes_never_panic() {
        let mut seed = 0x1234_5678_u32;
        for _ in 0..2000 {
            let len = (seed % 400) as usize;
            let buf: Vec<u8> = (0..len)
                .map(|i| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> (i % 24)) as u8
                })
                .collect();
            let _ = sniff_bytes(&buf, Some("x.bin"));
        }
    }
}
