//! Format identity, and the table — **which is data, not code**.
//!
//! Adding a format is a row in `formats.toml`, including its magic signature.
//! Detection needs no code per format; that is the whole point of the design
//! decision, and it is what makes "a new format costs 1–2 files" true rather
//! than aspirational ([03 §15.1](../../../03-ARCHITECTURE.md)).
//!
//! The table is embedded at compile time. It is *our* data, not user input, so
//! a malformed row is a build failure rather than a runtime error.

use core::fmt;

/// The broad category a format belongs to.
///
/// `Properties` is an enum over this rather than one flat struct, so adding a
/// format *inside* a kind touches no shared type and adding a kind touches one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MediaKind {
    /// Raster and vector images.
    Image,
    /// Audio, with or without a container.
    Audio,
    /// Video containers and streams.
    Video,
    /// Paged and flowed documents.
    Document,
    /// Containers holding other files.
    Archive,
    /// Row-and-column data.
    Tabular,
    /// Workbooks: cells in sheets, in a container.
    ///
    /// Separate from [`Self::Tabular`], which is CSV and JSON — text this
    /// build parses in its own address space. A `.xlsx` is a ZIP of XML
    /// written by somebody else, so it is parsed in a worker like every other
    /// container, and `Tabular`'s whole point is that it is not.
    Spreadsheet,
    /// Typefaces: the SFNT container and the two wrappers around it.
    ///
    /// A kind of its own rather than a corner of `Archive`, even though the
    /// worker is the same one. What a route means here is not what it means
    /// for a zip: a font's tables are not files, the licensing metadata inside
    /// one governs what the recipient may do with it, and the outline format
    /// is a property no repack can change. Sharing a kind would have made
    /// `zip -> woff` fall out of the table for free, and it is not a
    /// conversion.
    Font,
}

impl MediaKind {
    /// Whether a confined worker binary exists for this kind.
    ///
    /// **The one source of truth**, mirrored by
    /// `openconvert_sandbox::argv::EngineBin::for_media`, which cannot be called
    /// from here: `openconvert-sandbox` depends on this crate, not the other way
    /// round. A test in the sandbox crate asserts the two agree, so the
    /// duplication cannot drift silently.
    ///
    /// `Video` and `Tabular` are `false` deliberately, not by omission. Video's
    /// Class A work is container surgery on pure-Rust crates and its Class B
    /// work goes to the separately-installed video module, which confines
    /// itself; tabular data is CSV and JSON, parsed by pure Rust, where a
    /// subprocess would buy isolation from something that does not need it.
    ///
    /// This is what makes "sandbox everything" answerable rather than
    /// aspirational: a policy can force confinement exactly where there is
    /// something to confine it in.
    #[must_use]
    pub const fn has_worker(self) -> bool {
        !matches!(self, Self::Video | Self::Tabular)
    }

    /// Whether a confined worker exists for a *conversion*, not just a kind.
    ///
    /// Mirrors `EngineBin::for_conversion`, and the difference from
    /// [`Self::has_worker`] is the whole point: the engine is chosen by the
    /// **input** kind, except that anything producing audio goes to `oc-audio`.
    /// Gating on the output kind instead looks equivalent and is not — it would
    /// confine a video-to-image thumbnail into a worker that does not exist,
    /// turning a working route into `NoWorker` at execution time.
    ///
    /// A test in the sandbox crate asserts this agrees with `for_conversion`
    /// over every route in the table.
    #[must_use]
    pub fn conversion_has_worker(from: FormatId, to: FormatId) -> bool {
        if to.kind() == Some(Self::Audio) {
            return true;
        }
        from.kind().is_some_and(Self::has_worker)
    }

    /// Every kind, for exhaustive property tests.
    pub const ALL: &'static [Self] = &[
        Self::Image,
        Self::Audio,
        Self::Video,
        Self::Document,
        Self::Archive,
        Self::Tabular,
        Self::Font,
        Self::Spreadsheet,
    ];
}

impl fmt::Display for MediaKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Document => "document",
            Self::Archive => "archive",
            Self::Tabular => "tabular",
            Self::Font => "font",
            Self::Spreadsheet => "spreadsheet",
        })
    }
}

/// A format we know about.
///
/// A closed set. `Unknown` is a variant rather than an `Option` because "we
/// looked and could not tell" is a real answer that must survive into the
/// receipt and the failure table, not a missing value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FormatId {
    /// Sniffed, and no signature matched.
    Unknown,
    // -- image --
    /// Portable Network Graphics.
    Png,
    /// JPEG. The subject of the week-3 metadata strip, because its segment
    /// structure lets metadata be removed without decoding a pixel.
    Jpeg,
    /// WebP. Signature sits at offset 8, inside a RIFF container.
    Webp,
    /// GIF, both 87a and 89a.
    Gif,
    /// Windows bitmap. Two-byte signature - the weakest in the table, and the
    /// reason sniffing alone is not identity.
    Bmp,
    /// TIFF, little- and big-endian.
    Tiff,
    /// HEIC. **Shares ISO-BMFF box structure with AVIF and MP4** and differs
    /// only by brand at offset 4, so the three are distinguished by brand and
    /// never by container shape.
    Heic,
    /// AVIF. See [`FormatId::Heic`] on the shared box structure.
    Avif,
    /// JPEG XL. Two container forms: a bare codestream and an ISO-BMFF box.
    Jxl,
    /// SVG. XML, so **no fixed signature** - the leading bytes vary with the
    /// declaration, and it is identified structurally.
    Svg,
    // -- audio --
    /// RIFF WAVE. Signature at offset 8, like WebP.
    Wav,
    /// FLAC.
    Flac,
    /// MP3, with an ID3 tag or a bare frame sync.
    Mp3,
    /// Ogg container.
    Ogg,
    /// AAC/ALAC in an ISO-BMFF container — the "M4A" family. See the table
    /// row for how it shares box structure with `Mp4` yet stays distinct.
    M4a,
    /// Matroska audio — the extraction target. See the table row for why it
    /// never wins a content-sniff against `Mkv`.
    Mka,
    // -- video --
    /// MP4. See [`FormatId::Heic`] - same box structure, different brand.
    Mp4,
    /// AVI. A RIFF container, and the only one in this table.
    ///
    /// Its signature is `RIFF` at 0 with `AVI ` at 8 — the same shape WAV uses
    /// with a different form, which is why both are read at an offset rather
    /// than by the first four bytes alone.
    Avi,
    /// QuickTime. **The same ISO base media container as MP4**, under the
    /// brand `qt  `, and separated from it by that brand rather than by
    /// structure.
    ///
    /// Not a Family member: `ftypqt  ` and `ftypisom` are different byte
    /// strings at the same offset, so one signature match settles it. The
    /// families exist for formats that are byte-identical at the signature and
    /// need a second read; this needs none.
    ///
    /// SPIKED BEFORE THE ROW WAS WRITTEN, on real exports and on synthesised
    /// files: `mp4demux` parses them unchanged, carryable tracks come out
    /// byte-identical, and a track whose codec MP4 cannot hold is dropped
    /// rather than invented. That is why the routes are stream copies and why
    /// there is no `.mov` demuxer.
    Mov,
    /// Matroska. **Byte-identical EBML signature to WebM**; they are separated
    /// by the DocType element, not by magic.
    Mkv,
    /// WebM. See [`FormatId::Mkv`].
    Webm,
    // -- camera raw --
    /// Canon RAW v3 (CR3). Shares ISO-BMFF container with HEIC/MP4.
    Cr3,
    /// Canon RAW v2 (CR2). TIFF-based with CR signature at offset 8.
    Cr2,
    /// Nikon Electronic Format. TIFF-based with NEF signature.
    Nef,
    /// Sony Alpha Raw. Starts with a Sony header block.
    Arw,
    /// Adobe Digital Negative. TIFF-based DNG specification.
    Dng,
    // -- document --
    /// PDF.
    Pdf,
    /// Word (OOXML). **A ZIP file** - identical magic to `Zip` and `Odt`,
    /// resolved by inspecting the archive's contents.
    Docx,
    /// OpenDocument text. Also a ZIP; see [`FormatId::Docx`].
    Odt,
    /// PowerPoint (OOXML). **A ZIP**, like every other OOXML format; the part
    /// names separate them. See [`FormatId::Docx`].
    Pptx,
    /// OpenDocument presentation. A ZIP whose first member states its type.
    Odp,
    /// EPUB. A ZIP of XHTML with a spine that states the reading order.
    Epub,
    /// DXF, AutoCAD's interchange format — and the only CAD format here.
    ///
    /// **ASCII only.** A DXF is a flat stream of `group code` / `value` line
    /// pairs, which is what makes it readable without a CAD kernel. The binary
    /// variant is a different encoding of the same model and is refused by name
    /// rather than misread.
    ///
    /// Not `Vector` — there is no such kind — but a `Document`, which is what
    /// it converts to: a drawing on a page.
    Dxf,
    /// PostScript. Interpreted, and Ghostscript is excluded from core
    /// entirely - it escaped its own `-dSAFER` sandbox in the wild.
    ///
    /// **Detected, and deliberately routed nowhere.** Converting it means an
    /// interpreter, which is the sentence above. Detecting it is separate and
    /// is worth keeping on its own: A6/SR-4 says the extension is a claim, and
    /// a PostScript file wearing a `.jpg` name has to be NAMED as PostScript in
    /// the mismatch warning and in the refusal. Drop the row and that file
    /// becomes "the content did not match any format we know", which is a worse
    /// answer to a more suspicious file.
    ///
    /// `every_format_routes_or_is_declared_detect_only` is what keeps this a
    /// decision rather than an oversight.
    PostScript,
    // -- spreadsheet --
    /// Excel (OOXML). A ZIP; see [`FormatId::Docx`].
    Xlsx,
    /// OpenDocument spreadsheet. A ZIP whose first member states its type.
    Ods,
    // -- font --
    /// TrueType. The SFNT container with `glyf` outlines, version `0x00010000`.
    Ttf,
    /// OpenType with **CFF** outlines, signature `OTTO`.
    ///
    /// The same SFNT container as [`FormatId::Ttf`] holding a different kind of
    /// outline, which is why there is no `otf -> ttf` route: see the comment on
    /// the font rows in `route.rs`.
    Otf,
    /// WOFF: an SFNT whose tables are individually zlib-compressed.
    Woff,
    /// WOFF2: an SFNT whose tables are Brotli-compressed as one stream.
    ///
    /// A TARGET ONLY. Writing one needs a Brotli encoder and the null
    /// transform; reading an arbitrary one needs the `glyf` transform, which is
    /// a different and much larger job. See the font rows in `route.rs`.
    Woff2,
    // -- archive --
    /// ZIP. See [`FormatId::Docx`]: several document formats are ZIPs, so a
    /// magic match alone does not settle identity.
    Zip,
    /// TAR. Signature at **offset 257**, which is what sets
    /// [`max_magic_reach`] and why a sniffer reading 16 bytes would miss it.
    Tar,
    /// gzip.
    Gzip,
    /// 7-Zip.
    SevenZip,
    // -- tabular --
    /// CSV. **No signature and never will have one**; identified structurally.
    Csv,
    /// JSON. No signature; identified structurally.
    Json,
    /// Plain text — what OCR and transcription produce.
    ///
    /// It was declared once before and removed, because nothing produced it:
    /// the routes that named it were gated on models whose adapters did not
    /// exist, so the format advertised a capability the build did not have.
    /// It is back because that stopped being true.
    Txt,
    /// A Jupyter notebook.
    ///
    /// JSON, and therefore signature-identical to [`Self::Json`] — separated
    /// by the members the format requires (`nbformat` and `cells`), the same
    /// way a `.docx` is separated from a plain ZIP. See [`Family::Json`].
    Ipynb,
    /// HTML.
    ///
    /// Both a source and a destination. As a source it is identified by
    /// content like everything else — `<!doctype html` or `<html` — and a
    /// fragment carrying neither is detected as text, which under this
    /// build's text-is-Markdown rule still converts. See `mdread`.
    Html,
    /// Markdown.
    ///
    /// A DESTINATION, like `Txt` and for the same reason: nothing here parses
    /// it. What makes it worth having beside plain text is that the document
    /// pipeline already recovers structure — `text::layout` infers headings
    /// from glyph size, and a `.docx` names its own — and plain text is where
    /// that structure goes to die. Markdown is the cheapest format that keeps
    /// it.
    Markdown,
}

/// One row of the format table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// The identity this row describes.
    pub id: FormatId,
    /// Its category.
    pub kind: MediaKind,
    /// Short lowercase name, as it appears in the CLI and the receipt.
    pub name: &'static str,
    /// Canonical extension, without the dot.
    pub extension: &'static str,
    /// IANA media type where one exists.
    pub media_type: &'static str,
    /// Byte signatures that identify this format, with the offset each is
    /// found at. Empty for formats identified structurally rather than by
    /// magic — CSV has no signature and never will.
    pub magic: &'static [Magic],
    /// Whether a pure-Rust parser handles this format.
    ///
    /// Read by `route()` to decide `InProcess` vs `Sandboxed` (I10) — and by a
    /// property test asserting no memory-unsafe engine is ever `InProcess`.
    /// **This is a data claim**, gated by a review rule on this one table,
    /// which is stated in `03` §7.4 rather than left to be discovered.
    pub pure_rust_parser: bool,
}

/// A magic-byte signature at a known offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Magic {
    /// Offset from the start of the file.
    pub offset: usize,
    /// Bytes that must match exactly.
    pub bytes: &'static [u8],
}

impl Magic {
    const fn at(offset: usize, bytes: &'static [u8]) -> Self {
        Self { offset, bytes }
    }
}

macro_rules! fmt_row {
    ($id:ident, $kind:ident, $name:literal, $ext:literal, $mt:literal, $pure:literal, [$($m:expr),* $(,)?]) => {
        Format {
            id: FormatId::$id,
            kind: MediaKind::$kind,
            name: $name,
            extension: $ext,
            media_type: $mt,
            magic: &[$($m),*],
            pure_rust_parser: $pure,
        }
    };
}

/// The format table.
///
/// Ordered by kind then by name, so a diff adding a format is one line in a
/// predictable place. `Unknown` is deliberately absent — it is the *absence*
/// of a match, not a row.
pub const TABLE: &[Format] = &[
    // ---- image ----
    fmt_row!(
        Png,
        Image,
        "png",
        "png",
        "image/png",
        true,
        [Magic::at(0, b"\x89PNG\r\n\x1a\n")]
    ),
    fmt_row!(
        Jpeg,
        Image,
        "jpeg",
        "jpg",
        "image/jpeg",
        true,
        [Magic::at(0, b"\xFF\xD8\xFF")]
    ),
    fmt_row!(
        Webp,
        Image,
        "webp",
        "webp",
        "image/webp",
        true,
        [Magic::at(8, b"WEBP")]
    ),
    fmt_row!(
        Gif,
        Image,
        "gif",
        "gif",
        "image/gif",
        true,
        [Magic::at(0, b"GIF87a"), Magic::at(0, b"GIF89a")]
    ),
    fmt_row!(
        Bmp,
        Image,
        "bmp",
        "bmp",
        "image/bmp",
        true,
        [Magic::at(0, b"BM")]
    ),
    fmt_row!(
        Tiff,
        Image,
        "tiff",
        "tiff",
        "image/tiff",
        true,
        [Magic::at(0, b"II*\x00"), Magic::at(0, b"MM\x00*")]
    ),
    // HEIC and AVIF share the ISO-BMFF box structure and differ only in brand.
    fmt_row!(
        Heic,
        Image,
        "heic",
        "heic",
        "image/heic",
        false,
        [
            Magic::at(4, b"ftypheic"),
            Magic::at(4, b"ftypheix"),
            Magic::at(4, b"ftypmif1")
        ]
    ),
    fmt_row!(
        Avif,
        Image,
        "avif",
        "avif",
        "image/avif",
        false,
        [Magic::at(4, b"ftypavif")]
    ),
    fmt_row!(
        Jxl,
        Image,
        "jxl",
        "jxl",
        "image/jxl",
        false,
        [
            Magic::at(0, b"\xFF\x0A"),
            Magic::at(0, b"\x00\x00\x00\x0CJXL \r\n\x87\n")
        ]
    ),
    // SVG is XML: no fixed signature, and the leading bytes vary with the
    // declaration. Detected structurally.
    fmt_row!(Svg, Image, "svg", "svg", "image/svg+xml", true, []),
    // ---- audio ----
    // Every audio format parses through the confined worker, deliberately.
    // The flag answers "should this run in-process", and for attacker-supplied
    // media the answer is no regardless of how safe today's decoder is: the
    // decoders change, the boundary does not. This is the oc-archive rule --
    // memory_safe does not by itself grant InProcess -- applied to a whole
    // media kind.
    fmt_row!(
        Wav,
        Audio,
        "wav",
        "wav",
        "audio/wav",
        false,
        [Magic::at(8, b"WAVE")]
    ),
    fmt_row!(
        Flac,
        Audio,
        "flac",
        "flac",
        "audio/flac",
        false,
        [Magic::at(0, b"fLaC")]
    ),
    fmt_row!(
        Mp3,
        Audio,
        "mp3",
        "mp3",
        "audio/mpeg",
        false,
        [
            Magic::at(0, b"ID3"),
            Magic::at(0, b"\xFF\xFB"),
            Magic::at(0, b"\xFF\xF3")
        ]
    ),
    fmt_row!(
        Ogg,
        Audio,
        "ogg",
        "ogg",
        "audio/ogg",
        false,
        [Magic::at(0, b"OggS")]
    ),
    // M4A is ISO-BMFF: the same box structure HEIC, AVIF and MP4 use. As with
    // those three, the brand at offset 4 settles identity — `ftypM4A` cannot
    // also be `ftypisom`, so no signature overlaps and no polyglot question
    // arises from the container alone.
    //
    // `ftypqt  ` WAS ON THIS ROW and has moved to `Mov`, where it belongs. The
    // comment above already said the brand settles identity, and that brand
    // says QuickTime, not audio. What it produced: every `.mov` on this
    // machine -- H.264 video, LPCM audio and a timecode track -- was detected
    // as an AUDIO file, offered only the audio routes, and had its video
    // silently discarded by whichever one the user picked. An audio-only `.mov`
    // is the case that entry was presumably for, and it is now served by
    // `Mov -> Mka` and by the audio-extraction rows rather than by calling a
    // video container an audio format.
    fmt_row!(
        M4a,
        Audio,
        "m4a",
        "m4a",
        "audio/mp4",
        false,
        [Magic::at(4, b"ftypM4A"), Magic::at(4, b"ftypM4B ")]
    ),
    // ---- video ----
    // Class A video is pure-Rust container surgery (`02` §3.1): remux, stream
    // copy, extraction. The parsers are THIS project's, bounded like the
    // sniffer — depth, element count, advancing cursor, byte budget — which
    // is what admits them in-process (I10). No third-party parser is
    // involved until a real transcode needs FFmpeg, and that is a module.
    fmt_row!(
        Mp4,
        Video,
        "mp4",
        "mp4",
        "video/mp4",
        true,
        [
            Magic::at(4, b"ftypisom"),
            Magic::at(4, b"ftypmp42"),
            Magic::at(4, b"ftypM4V ")
        ]
    ),
    // QUICKTIME, BY BRAND. A `.mov` from a camera, a phone or an NLE opens
    // `ftypqt  ` -- the same `ftyp` box MP4 uses, with a different brand -- so
    // this is one more magic string rather than a second container reader.
    //
    // What varies between `.mov` files is the CODECS, not the boxes. ProRes
    // and QuickTime RLE have no MP4 representation and are refused by name
    // ("no audio or video track this build can carry"); LPCM audio is dropped
    // and disclosed. Both behaviours are the demuxer's and predate this row.
    fmt_row!(
        Mov,
        Video,
        "mov",
        "mov",
        "video/quicktime",
        true,
        [Magic::at(4, b"ftypqt  ")]
    ),
    // AVI. RIFF, like WAV, so the FORM at offset 8 is what separates them —
    // `Magic` takes an offset for exactly this.
    fmt_row!(
        Avi,
        Video,
        "avi",
        "avi",
        "video/x-msvideo",
        true,
        [Magic::at(8, b"AVI ")]
    ),
    fmt_row!(
        Mkv,
        Video,
        "mkv",
        "mkv",
        "video/x-matroska",
        true,
        [Magic::at(0, b"\x1A\x45\xDF\xA3")]
    ),
    fmt_row!(
        Webm,
        Video,
        "webm",
        "webm",
        "video/webm",
        true,
        [Magic::at(0, b"\x1A\x45\xDF\xA3")]
    ),
    // Matroska audio-only shares its DocType with MKV ("matroska"), so a file
    // sniffed from CONTENT always reports Mkv here; Mka exists as an OUTPUT
    // target (-t mka) and for extension-based hints. That asymmetry is
    // honest: the containers are identical at the header level, differing
    // only in whether video tracks ride along.
    fmt_row!(
        Mka,
        Audio,
        "mka",
        "mka",
        "audio/x-matroska",
        true,
        [Magic::at(0, b"\x1A\x45\xDF\xA3")]
    ),
    // ---- document ----
    fmt_row!(
        Pdf,
        Document,
        "pdf",
        "pdf",
        "application/pdf",
        false,
        [Magic::at(0, b"%PDF-")]
    ),
    fmt_row!(
        Docx,
        Document,
        "docx",
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Odt,
        Document,
        "odt",
        "odt",
        "application/vnd.oasis.opendocument.text",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    // THE OTHER ZIPS. Everything true of `Docx` above is true of these: they
    // share `PK\x03\x04` with `Zip` itself and are separated by what is inside,
    // which is `Family::Zip`'s whole reason for existing. `sniff::zip_member`
    // does the separating and its comment says how.
    fmt_row!(
        Pptx,
        Document,
        "pptx",
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Odp,
        Document,
        "odp",
        "odp",
        "application/vnd.oasis.opendocument.presentation",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Epub,
        Document,
        "epub",
        "epub",
        "application/epub+zip",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Xlsx,
        Spreadsheet,
        "xlsx",
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Ods,
        Spreadsheet,
        "ods",
        "ods",
        "application/vnd.oasis.opendocument.spreadsheet",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    // DXF HAS NO SIGNATURE, and never will: it opens with `0 SECTION`, which
    // is two ordinary lines of text. So it is separated structurally, in
    // `sniff::structural`, exactly as CSV and JSON are — and the marker used
    // there is the SECTION/HEADER pair, which every DXF has and no CSV does.
    fmt_row!(Dxf, Document, "dxf", "dxf", "image/vnd.dxf", true, []),
    fmt_row!(
        PostScript,
        Document,
        "postscript",
        "ps",
        "application/postscript",
        false,
        [Magic::at(0, b"%!PS")]
    ),
    // ---- fonts ----
    //
    // FOUR FORMATS, ONE CONTAINER. A TTF and an OTF are both SFNT: a table
    // directory and a pile of tables, differing in which outline table they
    // carry (`glyf` versus `CFF `) and in the version at offset 0. WOFF and
    // WOFF2 wrap that same container -- zlib per table, Brotli over all of
    // them -- so every route among them is a repack, and the routes that would
    // be an outline CONVERSION do not exist. See `route.rs`.
    //
    // `0x00010000` is a weaker signature than the others here: four bytes that
    // could open something else. It is the version field the specification
    // defines and what every reader uses, and a file that matches it and is not
    // a font fails at the table directory rather than being converted into
    // nonsense.
    //
    // `false` FOR ALL FOUR, exactly as the archive rows are and for the same
    // reason. The flag reads "can this be decoded without a C library", and for
    // a font the honest answer is yes — but the flag is what `isolation_for`
    // consults to decide whether a step runs in OUR address space, and the only
    // implementation of a font repack is `font.rs` inside oc-archive. Marked
    // `true`, `openconvert plan` said "in-process" for a conversion that then ran
    // in a worker, and a plan the receipt contradicts is the one thing a plan
    // must never be.
    fmt_row!(
        Ttf,
        Font,
        "ttf",
        "ttf",
        "font/ttf",
        false,
        [Magic::at(0, b"\x00\x01\x00\x00"), Magic::at(0, b"true")]
    ),
    fmt_row!(
        Otf,
        Font,
        "otf",
        "otf",
        "font/otf",
        false,
        [Magic::at(0, b"OTTO")]
    ),
    fmt_row!(
        Woff,
        Font,
        "woff",
        "woff",
        "font/woff",
        false,
        [Magic::at(0, b"wOFF")]
    ),
    fmt_row!(
        Woff2,
        Font,
        "woff2",
        "woff2",
        "font/woff2",
        false,
        [Magic::at(0, b"wOF2")]
    ),
    // ---- archive ----
    fmt_row!(
        Zip,
        Archive,
        "zip",
        "zip",
        "application/zip",
        false,
        [Magic::at(0, b"PK\x03\x04")]
    ),
    fmt_row!(
        Tar,
        Archive,
        "tar",
        "tar",
        "application/x-tar",
        false,
        [Magic::at(257, b"ustar")]
    ),
    fmt_row!(
        Gzip,
        Archive,
        "gzip",
        "gz",
        "application/gzip",
        false,
        [Magic::at(0, b"\x1F\x8B")]
    ),
    fmt_row!(
        SevenZip,
        Archive,
        "7z",
        "7z",
        "application/x-7z-compressed",
        false,
        [Magic::at(0, b"7z\xBC\xAF\x27\x1C")]
    ),
    // ---- camera raw ----
    fmt_row!(
        Cr3,
        Image,
        "cr3",
        "cr3",
        "image/x-canon-cr3",
        false,
        [Magic::at(4, b"ftypcrx")]
    ),
    fmt_row!(
        Cr2,
        Image,
        "cr2",
        "cr2",
        "image/x-canon-cr2",
        false,
        [
            Magic::at(0, b"II*\x00\x10\x00\x00\x00CR"),
            Magic::at(0, b"MM\x00\x2a\x10\x00\x00\x00CR")
        ]
    ),
    fmt_row!(
        Nef,
        Image,
        "nef",
        "nef",
        "image/x-nikon-nef",
        false,
        [Magic::at(0, b"II*\x00\x10\x00\x00\x00NEF")]
    ),
    fmt_row!(
        Arw,
        Image,
        "arw",
        "arw",
        "image/x-sony-arw",
        false,
        [Magic::at(0, b"\x00\x00\x00\x08\x53\x4f\x4e\x59")]
    ),
    fmt_row!(
        Dng,
        Image,
        "dng",
        "dng",
        "image/x-adobe-dng",
        false,
        [Magic::at(0, b"II*\x00"), Magic::at(0, b"MM\x00*")]
    ),
    // ---- tabular ----
    fmt_row!(Csv, Tabular, "csv", "csv", "text/csv", true, []),
    fmt_row!(Json, Tabular, "json", "json", "application/json", true, []),
    // A notebook is JSON with required members, so it has no signature of its
    // own and is separated structurally. `Document`, not `Tabular`: what it
    // holds is prose, code and output, and it converts to documents.
    fmt_row!(
        Ipynb,
        Document,
        "notebook",
        "ipynb",
        "application/x-ipynb+json",
        true,
        []
    ),
    // Plain text: no signature, like CSV and JSON.
    //
    // It is a SOURCE now as well as a destination. Nothing detects it -- there
    // is nothing to detect -- so anything textual that matches no other
    // signature arrives here, and `mdread` parses it as CommonMark. Plain text
    // is CommonMark containing no markup, so that is correct for a `.txt` and
    // an enhancement for a `.md`, which cannot be told apart by content.
    fmt_row!(Txt, Document, "txt", "txt", "text/plain", true, []),
    // Markdown: no signature either. `text/markdown` is registered (RFC 7763).
    fmt_row!(
        Markdown,
        Document,
        "markdown",
        "md",
        "text/markdown",
        true,
        []
    ),
    // HTML has no registered magic number, but the openings below are how
    // browsers and file(1) recognise one in practice. A fragment with none of
    // them is detected as text, which still converts.
    fmt_row!(
        Html,
        Document,
        "html",
        "html",
        "text/html",
        true,
        [
            Magic::at(0, b"<!DOCTYPE html"),
            Magic::at(0, b"<!doctype html"),
            Magic::at(0, b"<html")
        ]
    ),
];

/// A set of formats that **share a signature by design**.
///
/// # Why this is not the polyglot flag
///
/// `sniff` flagged any file matching two signatures as a polyglot and `route()`
/// refused it. Two families in this table share magic deliberately — the rows
/// say so in their own comments — so the flag fired on every member of both:
///
/// - **EBML**: `Mkv` and `Webm` are byte-identical at the signature and are
///   separated by the DocType element. Every Matroska file was refused, which
///   is `01` §154's flagship format.
/// - **ZIP**: `Zip`, `Docx` and `Odt` all begin `PK`. Every archive was
///   refused *and* misidentified as `docx`, because the detector took the first
///   table row that matched.
///
/// A polyglot is a file that is validly two **different** things — a GIF that
/// is also a JAR — and it is an attack. A file matching two members of one
/// family is an ordinary file whose exact identity needs one more read. Treating
/// the second as the first is how five of twenty-seven formats became
/// unconvertible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// Matroska, WebM and MKA. Separated by the DocType element — except
    /// Mka, which shares MKV's DocType exactly and is distinguished (when it
    /// matters) by having no video tracks. See the table row.
    Ebml,
    /// ZIP and the formats that are ZIP archives with required members.
    Zip,
    /// TIFF and the camera RAW formats built on it.
    ///
    /// `Dng`'s signature is **byte-identical** to `Tiff`'s -- a DNG *is* a
    /// TIFF, by specification -- and `Cr2` and `Nef` are TIFFs whose first
    /// eight bytes happen to continue into something more specific. So every
    /// file in this family matched at least two rows and every one of them was
    /// refused as a polyglot: all five `Tiff ->` routes, both `Dng ->`, both
    /// `Cr2 ->` and both `Nef ->`.
    ///
    /// This is the third time the same shape has appeared, after EBML and ZIP,
    /// and it was missed because TIFF's collision is with rows added later and
    /// far away in the table. `Arw` is deliberately NOT a member: its magic is
    /// a Sony header, not a TIFF one, so it was never ambiguous.
    Tiff,
}

impl FormatId {
    /// This format's row, or `None` for [`FormatId::Unknown`].
    #[must_use]
    pub fn row(self) -> Option<&'static Format> {
        TABLE.iter().find(|f| f.id == self)
    }

    /// The family whose members share this format's signature, if any.
    ///
    /// `None` means the signature identifies the format on its own, which is
    /// true of every row not listed here — including `Mp4`, `Heic` and `Avif`,
    /// which share the ISO-BMFF box structure but carry **distinct brands** in
    /// their magic and so were never ambiguous.
    #[must_use]
    pub const fn family(self) -> Option<Family> {
        match self {
            Self::Mkv | Self::Webm | Self::Mka => Some(Family::Ebml),
            // EVERY ZIP-SHAPED FORMAT, and the list grows with the table.
            // Leaving one out is not a missing feature: `resolve` only calls
            // `zip_member` when every match belongs to ONE family, so a format
            // that has ZIP's magic and no family membership makes every ZIP
            // look like a polyglot and be refused.
            Self::Zip
            | Self::Docx
            | Self::Odt
            | Self::Pptx
            | Self::Odp
            | Self::Epub
            | Self::Xlsx
            | Self::Ods => Some(Family::Zip),
            Self::Tiff | Self::Dng | Self::Cr2 | Self::Nef => Some(Family::Tiff),
            _ => None,
        }
    }

    /// The kind this format belongs to.
    #[must_use]
    pub fn kind(self) -> Option<MediaKind> {
        self.row().map(|f| f.kind)
    }

    /// Whether a pure-Rust parser handles it.
    ///
    /// `Unknown` returns `false`: something we could not identify is exactly
    /// what should not run in our own address space.
    #[must_use]
    pub fn has_pure_rust_parser(self) -> bool {
        self.row().is_some_and(|f| f.pure_rust_parser)
    }

    /// Short lowercase name for display.
    #[must_use]
    pub fn name(self) -> &'static str {
        self.row().map_or("unknown", |f| f.name)
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// How many bytes a sniffer should be handed.
///
/// [`max_magic_reach`] answers a narrower question -- how far the deepest
/// SIGNATURE reaches -- and for a decade of formats that was the same number.
/// It stopped being the same number when `Tiff` and `Dng` turned out to share
/// a signature exactly: separating them means reading IFD0 for the
/// `DNGVersion` tag, and TIFF sorts its entries by tag, so a tag as high as
/// `0xC612` sits at the END of an IFD that can be hundreds of bytes long.
/// 265 bytes cannot see it.
///
/// 4 KiB is a bounded read that covers IFD0 for every real DNG while staying
/// far below any allocation worth worrying about, and a short file is still
/// just a short file.
#[must_use]
pub fn sniff_window() -> usize {
    const IFD_WINDOW: usize = 4096;
    let reach = max_magic_reach();
    if reach > IFD_WINDOW {
        reach
    } else {
        IFD_WINDOW
    }
}

/// How many bytes a sniffer must read to evaluate every signature.
///
/// Derived from the table rather than guessed, so adding a format with a deeper
/// signature cannot leave the sniffer reading too little. `tar`'s `ustar` at
/// offset 257 is why this is not the 8 or 16 bytes one might assume.
#[must_use]
pub fn max_magic_reach() -> usize {
    TABLE
        .iter()
        .flat_map(|f| f.magic.iter())
        .map(|m| m.offset + m.bytes.len())
        .max()
        .unwrap_or(0)
}
