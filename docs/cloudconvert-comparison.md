# CloudConvert's catalogue, and where OpenConvert stands against it

Transcribed from CloudConvert's format catalogue on 2026-09-07, then checked
against `RouteTable::v1()` by running `openconvert routes <from> <to>` for every
conversion CloudConvert calls common. Nothing below is read off the route table
by eye.

---

## 1. The catalogue

**212 formats across 11 categories.**

### Documents (23)

`ABW` `DJVU` `DOC` `DOCM` `DOCX` `DOT` `DOTX` `HTML` `HWP` `HWPX` `LWP` `MD`
`ODT` `PAGES` `PDF` `RST` `RTF` `SDW` `TEX` `TXT` `WPD` `WPS` `ZABW`

Common: `PDF → DOCX` (editable Word document) · `DOCX → PDF` (print-ready PDF) ·
`HTML → TXT` (plain text export)

### Images (42)

`3FR` `ARW` `AVIF` `BMP` `CR2` `CR3` `CRW` `DCR` `DNG` `EPS` `ERF` `GIF` `HEIC`
`HEIF` `ICNS` `ICO` `JFIF` `JPEG` `JPG` `MOS` `MRW` `NEF` `ODD` `ODG` `ORF`
`PEF` `PNG` `PPM` `PS` `PSB` `PSD` `PUB` `RAF` `RAW` `RW2` `TGA` `TIF` `TIFF`
`WEBP` `X3F` `XCF` `XPS`

Common: `HEIC → JPG` (camera roll compatibility) · `SVG → PNG` (raster image
output) · `TIFF → WEBP` (smaller web image)

### Video (28)

`3G2` `3GP` `3GPP` `AVI` `CAVS` `DV` `DVR` `FLV` `M2TS` `M4V` `MKV` `MOD` `MOV`
`MP4` `MPEG` `MPG` `MTS` `MXF` `OGG` `OGV` `RM` `RMVB` `SWF` `TS` `VOB` `WEBM`
`WMV` `WTV`

Common: `MOV → MP4` (browser-friendly video) · `MKV → WEBM` (web playback) ·
`AVI → MP4` (device compatibility)

### Audio (21)

`AAC` `AC3` `AIF` `AIFC` `AIFF` `AMR` `AU` `CAF` `DSS` `FLAC` `M4A` `M4B` `MP3`
`OGA` `OPUS` `SF2` `SFARK` `VOC` `WAV` `WEBA` `WMA`

Common: `WAV → MP3` (compressed audio) · `FLAC → M4A` (portable lossless
library) · `OGG → AAC` (mobile playback)

### Spreadsheets (8)

`CSV` `ET` `NUMBERS` `ODS` `SDC` `XLS` `XLSM` `XLSX`

Common: `XLSX → CSV` (data export) · `ODS → XLSX` (Excel workflow) ·
`NUMBERS → PDF` (shareable report)

### Slides (11)

`DPS` `KEY` `ODP` `POT` `POTX` `PPS` `PPSX` `PPT` `PPTM` `PPTX` `SDA`

Common: `PPTX → PDF` (deck handoff) · `KEY → PPTX` (PowerPoint editing) ·
`ODP → PDF` (review copy)

### E-books (22)

`AZW` `AZW3` `AZW4` `CBC` `CBR` `CBZ` `CHM` `EPUB` `FB2` `HTM` `HTMLZ` `LIT`
`LRF` `MOBI` `OEB` `PDB` `PML` `PRC` `RB` `SNB` `TCR` `TXTZ`

Common: `EPUB → PDF` (fixed-layout reading) · `MOBI → EPUB` (modern ereader
format) · `AZW → PDF` (document archive)

### Archives (39)

`7Z` `ACE` `ALZ` `ARC` `ARJ` `BZ` `BZ2` `CAB` `CPIO` `DEB` `DMG` `EML` `GZ`
`IMG` `ISO` `JAR` `LHA` `LZ` `LZMA` `LZO` `RAR` `RPM` `RZ` `TAR` `TAR.7Z`
`TAR.BZ` `TAR.BZ2` `TAR.GZ` `TAR.LZO` `TAR.XZ` `TAR.Z` `TBZ` `TBZ2` `TGZ` `TZ`
`TZO` `XZ` `Z` `ZIP`

Common: `RAR → ZIP` (standard archive) · `7Z → TAR` (server workflow) ·
`TAR.GZ → ZIP` (desktop sharing)

### Vector (10)

`AI` `CDR` `CGM` `EMF` `SK` `SK1` `SVG` `SVGZ` `VSD` `WMF`

Common: `SVG → PNG` (web preview) · `AI → SVG` (vector handoff) · `EPS → PDF`
(print proof)

### CAD (3)

`DWF` `DWG` `DXF`

Common: `DWG → DXF` (CAD interchange) · `DXF → PDF` (review drawing) ·
`STP → OBJ` (3D workflow)

> `STP` and `OBJ` are advertised as a common conversion and appear in **no**
> category list on the page. Either the catalogue or the example is wrong.

### Fonts (5)

`EOT` `OTF` `TTF` `WOFF` `WOFF2`

Common: `TTF → WOFF2` (web font delivery) · `OTF → TTF` (desktop compatibility)
· `WOFF → WOFF2` (smaller web font)

---

## 2. The comparison that is not a scoreboard

**212 formats against 50 is the wrong headline**, and reading it as a gap of 174
would lead to the wrong work.

CloudConvert is a server that shells out to LibreOffice, Calibre, ImageMagick,
FFmpeg and Ghostscript. Nearly its whole Documents, Slides, Spreadsheets and
E-books surface — roughly 64 formats — is *one* dependency each: LibreOffice
gives it `DOC DOCM DOT DOTX HWP LWP PAGES RTF SDW WPD WPS` and every slide and
spreadsheet format; Calibre gives it the 22 e-books.

OpenConvert writes its own engines and runs each in a sandboxed worker with pinned
native libraries, a fidelity class per route and a receipt per conversion.
Bundling LibreOffice would add ~60 formats and a 400 MB dependency that executes
documents, into a product whose premise is that a hostile file is confined.

So the meaningful comparison is not the count. It is: **of the conversions
people actually perform, which work?**

| | CloudConvert | OpenConvert |
|---|---|---|
| Formats | 212 | 50 |
| Routes | not published | 268, all enumerated in `docs/ROUTES.md` |
| Runs | on their servers | on this machine |
| Says what was lost | no | fidelity class A–D + a receipt per file |
| Refuses a route it cannot run | n/a | yes, by name |

---

## 3. Coverage of the 33 common conversions

Measured, one `openconvert routes` call each, re-run after the missing-routes
plan's phases 1–4 and the AVI, DXF and workbook work that followed.
`SVG → PNG` is listed twice (Images and Vector), so there are 32 distinct pairs.

| Category | Covered | Detail |
|---|---|---|
| **Documents** | **3 / 3** | `PDF→DOCX` C · `DOCX→PDF` C · `HTML→TXT` C |
| **Images** | **3 / 3** | `HEIC→JPG` B · `SVG→PNG` B · `TIFF→WEBP` B |
| **Video** | **3 / 3** | `MOV→MP4` **A** · `MKV→WEBM` **A** · `AVI→MP4` **A** |
| **Slides** | 2 / 3 | `PPTX→PDF` C · `ODP→PDF` C · KEY is a proprietary Apple bundle |
| **Archives** | 2 / 3 | `7Z→TAR` B · `GZIP→ZIP` **A** · RAR is licence-blocked |
| **Spreadsheets** | 2 / 3 | `XLSX→CSV` C · `ODS→XLSX` C · NUMBERS is Apple's |
| **Fonts** | 2 / 3 | `TTF→WOFF2` **A** · `WOFF→WOFF2` **A** · `OTF→TTF` declined |
| **CAD** | 1 / 3 | `DXF→PDF` C · DWG is closed and GPL-only · STP needs a kernel |
| E-books | 1 / 3 | `EPUB→PDF` C · MOBI and AZW are Amazon binaries, often DRM |
| Audio | 1 / 3 | `WAV→MP3` B · no permissively-licensed AAC encoder |
| Vector | 1 / 3 | `SVG→PNG` B · AI and EPS need a PostScript interpreter |

**21 of 33 listed; 20 of 32 distinct.** Up from 10 and 9.

Three categories are complete and five more are one short.

### What changed, and what it cost

| | Before | After | What it took |
|---|---|---|---|
| Documents | 3/3 | 3/3 | `htmlread`: `HTML→TXT` was **writing an empty file** |
| Video | 1/3 | 3/3 | `Mov` was one `Magic` entry; `Avi` was a RIFF walk and Annex-B→AVCC |
| Fonts | 0/3 | 2/3 | One module in oc-archive; no new engine |
| Slides | 0/3 | 2/3 | A slide-text scanner shaped like `office.rs` |
| Spreadsheets | 0/3 | 2/3 | `calamine` to read, `rust_xlsxwriter` to write |
| Archives | 1/3 | 2/3 | One row, once `StepKind::Extract` could name its ends |
| CAD | 0/3 | 1/3 | A DXF pair-scanner, bulges and de Boor, and a vector emitter |
| E-books | 0/3 | 1/3 | A spine reader, feeding `htmlread` |

### The twelve that remain, and why each does

Four kinds of reason, and **not one of them is effort**:

**A licence, or a closed format** — `RAR→ZIP` (the unrar licence forbids
reimplementing the decoder), `FLAC→M4A` and `OGG→AAC` (no AAC encoder clears
`LICENCE_ALLOWLIST`; `m4a` is read and never written), `DWG→DXF` (LibreDWG is
GPL-3.0, and DWG is undocumented and versioned).

**Somebody else's secret** — `NUMBERS→PDF`, `KEY→PPTX` (proprietary Apple
bundles that change between releases), `MOBI→EPUB`, `AZW→PDF` (Amazon binaries,
frequently DRM-protected, and stripping DRM is not something this product does).

**A decision this product already made** — `EPS→PDF` and `AI→SVG` need a
PostScript interpreter, and Ghostscript is excluded from this build *because it
escaped its own `-dSAFER` sandbox in the wild*. Writing a less-audited
interpreter for a Turing-complete language and pointing it at untrusted input
would be worse than the thing already refused. `.ai` files nonetheless convert
today through every PDF route, because a modern one **is** a PDF.

**A different product** — `STP→OBJ` means tessellating trimmed NURBS surfaces,
which is the core of a CAD kernel. The entry point to 3D, if it is ever wanted,
is `STL→OBJ`: both are meshes.

**And one declined on honesty grounds.** `OTF→TTF` would mean converting cubic
Bézier outlines to quadratic — an approximation of every curve in the font — or
renaming the container, which ships CFF outlines in a file called `.ttf` that
half the world will not render. Every converter on the web offers one of the
two under this name. `woff → ttf` on a CFF font is refused **by name**, pointing
at the route that works.

The full reasoning for the last three is in
[EPS, DWG and STEP: why these three are not being built](plans/2026-09-07-eps-dwg-step-declined.md).

### The findings that came out of the sweeps

`postscript` was a declared format with **zero routes in either direction** —
detectable, then refused for everything. It is still detected and still routes
nowhere, and that is now a **decision with a test behind it**:
`every_format_routes_or_is_declared_detect_only` fails the build on any format
with no routes that is not written down with its reason. Detection earns its
place separately, because A6/SR-4 says the extension is a claim and a PostScript
file wearing a `.jpg` name has to be named as PostScript in the warning and the
refusal.

`openconvert formats` now prints how many routes start and end at each format, so
a 0/0 row says "detected only; no conversions" on the line where somebody would
otherwise read it as capability.

## 4. What is easy, and what only looks easy

Ranked by conversions gained per unit of work, using what the repo already has.

### Tier 1 — hours, no new dependency

**1. `gzip → zip`, and the `TAR.GZ` family.** One route row chaining two legs
that both exist and are Class A. Gains a common conversion outright.

**2. `MOV → MP4`.** A `.mov` is ISO-BMFF — the same container family as MP4,
differing mainly in brand — and `mp4demux.rs`, `mp4mux.rs` and `remux.rs` are
already here doing exactly this work for MP4/MKV. Likely a detection entry and a
brand check rather than a demuxer. **Highest-demand single item on the list**,
and worth spiking before promising: if the sample files are plain ISO-BMFF it is
nearly free, and if they carry QuickTime-only atoms it is not.

**3. Fonts: `TTF ↔ WOFF ↔ WOFF2`.** Wins the whole Fonts category — all three
common conversions — for one of the smallest jobs on this page. WOFF and WOFF2
are container wrappers around the same SFNT tables, zlib and Brotli
respectively, and pure-Rust crates exist for both. No rendering, no layout, no
parsing of hostile glyph programs beyond a table walk. Squarely the audience
already asked about.

### Tier 2 — days, reusing machinery that exists

**4. `PPTX → PDF/TXT/MD` and `ODP → PDF`.** PPTX and ODP are ZIP-plus-XML in
exactly the shape `office.rs` already scans for DOCX and ODT, and `typeset.rs`
already writes the PDF. The work is a slide-text extractor and a route table
entry; the paragraph model and every writer downstream are done. Gains 2 of 3
Slides.

**5. `EPUB → PDF/TXT/MD`.** ZIP plus XHTML, the same shape again, and the HTML
text path already exists. Gains 1 of 3 E-books.

**6. `XLSX/ODS → CSV`.** `calamine` (MIT, pure Rust) reads xlsx, ods and xls.
Clears `LICENCE_ALLOWLIST`, links nothing native, so `depcheck` is unaffected.
`ODS → XLSX` additionally needs a writer (`rust_xlsxwriter`, MIT). Gains 1–2 of
3 Spreadsheets.

**7. `AI → PDF`.** Modern `.ai` files *are* PDF with a private data stream, so
this may be a detection entry and a passthrough. Verify on real files first —
older AI is genuine PostScript and is a different job entirely.

### Tier 3 — real work, do only on demand

`AVI → MP4` needs a RIFF demuxer. `DXF → PDF` needs a CAD interpreter. `EPS →
PDF` needs a PostScript interpreter — which would also give `postscript` its
missing routes.

### Deliberately not

| | Why |
|---|---|
| `RAR → ZIP` | The unrar licence forbids reimplementing the decoder and does not clear `LICENCE_ALLOWLIST`. |
| `FLAC → M4A`, `OGG → AAC` | No AAC encoder with a permissive licence. `fdk-aac` is not on the allowlist. |
| `NUMBERS`, `KEY`, `PAGES` | Proprietary Apple bundles, undocumented, and they change. |
| `MOBI`, `AZW` | Amazon binary formats; `AZW` is frequently DRM-protected, and stripping DRM is not something this product should do. |
| `DOC`, `PPT`, `XLS` (pre-2007) | Compound binary formats. LibreOffice is the only real reader, and bundling it is the decision this design already declined. |

---

## 5. What this suggests

**Done, and then some.** The four phases plus the AVI, DXF and workbook work
took the common-conversion score from **10/33 to 21/33**. The original version
of this section predicted 15/33 after Tier 1 and roughly 20/33 after Tier 2, and
was wrong in both directions in the same interesting way.

It was **too pessimistic about the work**. MOV, the whole font family and the
archive chain each cost less than a day. The ZIP-of-XML families cost one reader
module apiece because `Paragraph` and the six writers already existed. Even AVI
and DXF — both filed under "real engineering, do only on demand" — came in at a
module each, because the spikes narrowed them: an AVI's hard part turned out to
be one bitstream reframing, and a DXF's turned out to be five entity types.

It was **too optimistic about the count**, because two of the gaps are decisions
rather than work: `OTF → TTF` is an outline conversion this build declines to
pretend at, and the PostScript family is excluded by a security decision the
project made years before this list existed.

What all of it actually produced was not mostly routes. It was **eight defects
in shipped code**, each found by running a real file rather than by reading:

- `HTML → *` wrote an **empty file** and reported success, for every target.
- Three `* → HTML` routes were declared in the table and refused by the engine.
- Every `.mov` was detected as an **audio** file and had its video discarded.
- Every `mp4 → mkv` this build wrote was **unreadable by any player**.
- `ctts` was read by nothing, so B-frame video came out **in the wrong order**
  under a Class A receipt.
- A real `.xlsx` was detected as a plain zip, because Excel writes
  `[Content_Types].xml` last and the sniffer looked for nothing else.
- A font conversion planned "in-process" and then ran in a worker, so the plan
  contradicted the receipt.
- Six receipt and assertion strings had lost a line-continuation backslash and
  read with runs of spaces mid-sentence.

Each now has a gate. `every_declared_document_route_runs` drives the shipped CLI
through all 49 declared document pairs and checks the input's own text comes out
the far end; `every_format_routes_or_is_declared_detect_only` fails on a format
that converts nowhere; and the QuickTime, AVI and Matroska tests assert the
BYTES rather than a round trip, because a round trip through our own reader was
exactly what hid three of these.

The twelve that remain are the ones a local-first, sandboxed converter should be
expected to lack. Each has a reason worth stating rather than a gap worth
apologising for, and none of the reasons is effort.
