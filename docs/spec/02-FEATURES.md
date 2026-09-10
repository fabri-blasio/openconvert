# OpenConvert — Feature Catalog

Every conversion, every AI capability, every application feature — with what ships in the core, what's a module, and what it costs.

Status: design record, **v0.4** · 2026-08-13
Part 2 of 9 — see [README](README.md). Architecture: [03-ARCHITECTURE](03-ARCHITECTURE.md).

> **Scope note.** This is the *catalogue*, not the v1 scope. v1 ships the Core (`●`) conversions with no AI, no modules, no server **and no shell extension** — see [08-EXECUTION-PLAN §3](08-EXECUTION-PLAN.md#3-weeks-544--outcomes). Everything marked as a module, an AI pack, or a server feature is post-v1 with an **observable** trigger in [03-ARCHITECTURE §15.3](03-ARCHITECTURE.md#153-what-is-deferred-and-the-trigger-that-fires).

---

## Contents

- [Legend](#legend)
- [Packaging model](#packaging-model)
- **Conversions** — [Images](#1-images) · [Audio](#2-audio) · [Video](#3-video) · [Documents & PDF](#4-documents--pdf) · [Ebooks](#5-ebooks--publishing) · [Data](#6-data--tabular) · [Archives](#7-archives) · [3D/CAD/GIS](#8-3d-cad--gis) · [Email, fonts, subtitles, misc](#9-email-fonts-subtitles--misc)
- **[AI features](#10-ai-features)** — the second master table
- [Model licensing policy](#11-model-licensing-policy)
- [Application features](#12-application-features)
- [Server features](#13-server-features)
- [Explicitly excluded](#14-explicitly-excluded)

---

## Legend

**Class** — what the operation does to your data ([Vision §4.2](01-VISION.md#42-verifiable-conversion))

| | |
|---|---|
| **A** | Lossless / deterministic — bit-exact round-trip possible |
| **B** | Lossy but deterministic and reproducible |
| **C** | Inferred — produces *new* data, source untouched, always reviewable |
| **D** | **Generative** — invents information. Off by default, and **never auto-selected**: `route()` filters every route above `Policy::max_auto_class`, so no default, preset, or prediction can arm one. Asserted by a property test. |

There are exactly four classes. A step's class is a required field, not an annotation.

**Pkg** — which package provides it

| | |
|---|---|
| **●** | **Core** — in the base install (<60 MB) |
| **V** | Video module (~30 MB) — FFmpeg |
| **O** | Office module (~410 MB) — LibreOffice |
| **E** | Ebook module (~40 MB) — Calibre |
| **A** | AI packs — per capability, see [§10](#10-ai-features) |
| **X** | Extended modules — 3D, CAD, GIS, EPS, ImageMagick long tail |

**AI** — can a local model help? `—` none · `P` parameters only · `E` enhanced · **`R` required**

**CPU** — runs on 4-core / 8 GB / no GPU: ✅ comfortably · ⚠️ slow · ❌ needs GPU

**Demand** — estimated global searches/month. ★★★★★ ≥200k · ★★★★ 50–200k · ★★★ 10–50k · ★★ 1–10k · ★ <1k. **Bold figures are measured**, everything else is a banded estimate — see [05-RESEARCH](05-RESEARCH.md).

---

## Packaging model

Two decisions shape every table below.

**No Ghostscript, anywhere in core.** It's a PostScript *interpreter* — it executes a Turing-complete language from the input file — and it escaped its own `-dSAFER` sandbox in CVE-2024-29510, exploited in the wild. Replaced by pdfium (BSD-3) and qpdf (Apache-2.0). EPS/PostScript input moves to an optional T3-only module.

**No FFmpeg in core either.** Core handles **all audio** (Symphonia + permissive encoders) and **Class-A video** (remux, stream copy, lossless trim, audio extraction) with pure-Rust container crates. Video *transcoding* needs the Video module, which prefers a system FFmpeg if present and otherwise fetches a signed LGPL-only build.

What that buys: core stays under 60 MB ⚠️ *(an estimate until [Week 0](08-EXECUTION-PLAN.md#1-week-0--before-any-code) measures the packaged worker set)*, ships **no H.264/HEVC/AAC encoder** — a far cleaner patent posture — and users who never touch video never carry FFmpeg's CVE stream.

What it costs: `convert AVI to MP4` and `compress video` — two of the largest video queries — prompt for a one-click 30 MB download on first use.

### The LGPL obligation, stated accurately

v0.3 claimed excluding FFmpeg meant core "carries no LGPL compliance burden." **That was wrong, and `engines.toml` would have proved it wrong in week 1.** Four core engines are LGPL: **libvips** (LGPL-2.1+), **libheif** (LGPL-3.0), **libraw** (LGPL-2.1 / CDDL dual) and **LAME** (LGPL-2.0+) — the last of which powers `Any → MP3`, the second-highest-demand conversion in this document.

The obligation is **bounded, not absent** — but v0.5 bounded it with the wrong reason, and the wrong reason would not have survived a licence review.

> **The correction.** v0.5 said nothing is owed because *"the engines are separate executables"*. The subprocess boundary is real, and it is **in the wrong place**. It sits between `openconvert-run` and `oc-images`. **libheif is *inside* `oc-images`, linked**, and the LGPL cares about the boundary between the library and whatever contains it — which has no wall at all. The relinking obligation attaches to the worker, one level in from where that sentence was looking.
>
> What actually discharges it is **[D19](03-ARCHITECTURE.md#16-decisions): every LGPL engine is linked dynamically**, shipped as its own `.dll` / `.so` / `.dylib`, so a user can replace it with their own build. That is the entire requirement, and satisfying it costs us nothing we were not already doing. `engines.toml` records `link_mode` per engine and **CI fails any LGPL engine that is static**, which turns a legal judgement into a build failure — the only form of compliance that survives a contributor who has never read this page.

| Owed | Not owed |
|---|---|
| Licence texts shipped in `NOTICE` | Any relinking obligation on the user — **because each LGPL engine ships as a replaceable shared library**, gated in CI. *(Not because the engines are separate executables. If we ever linked one statically, this obligation would attach and the gate is what stops that happening by accident.)* |
| An attributed notice per LGPL component | Source for our own code beyond Apache-2.0's terms (which is all of it anyway) |
| A source mirror for the exact engine versions shipped, for the life of the release | Anything at all for the permissive engines (pdfium, qpdf, libjxl, libavif, lcms2, libarchive, resvg) |

**One packaging consequence.** Shared libraries are opened by the loader **inside** the sandbox, under the container's identity rather than the installer's — so on Windows they must be readable by the AppContainer SID or the worker fails before `main` runs, with a loader error naming a missing dependency rather than a permission problem ([03 §5.3](03-ARCHITECTURE.md#53-what-the-implementer-must-not-discover-later)).

All three obligations are release-process work, scheduled in [weeks 41–44](08-EXECUTION-PLAN.md#7-parallel-non-engineering-tracks) rather than discovered at ship time. What excluding FFmpeg actually buys is the **patent** posture and the CVE stream — which is a strong enough argument that it did not need the false one.

---

## 1. Images

### 1.1 Format conversion

| Conversion | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| HEIC/HEIF → JPEG / PNG | B | ● | — | ✅ | **207,000** | The iPhone-on-Windows problem. libheif. |
| PNG ↔ JPEG | B | ● | — | ✅ | ★★★★★ | Pure Rust, in-process, no subprocess — and still limit-capped before decode |
| WEBP → JPEG / PNG | B | ● | — | ✅ | ★★★★★ | |
| JPEG / PNG → WEBP | B | ● | P | ✅ | ★★★★ | AI-P: SSIM-guided quality target |
| AVIF ↔ JPEG / PNG | B | ● | P | ✅ | ★★★ | libavif — royalty-free default |
| JPEG ↔ JPEG XL | **A** | ● | — | ✅ | ★★ | **Lossless JPEG recompression: ~20% smaller, bit-exact restore** |
| TIFF ↔ any | A/B | ● | — | ✅ | ★★★ | Multi-page supported |
| BMP / GIF / ICO ↔ any | B | ● | — | ✅ | ★★★ | |
| SVG → PNG / JPEG / PDF | B | ● | — | ✅ | ★★★★ | **resvg** — no scripting, no external refs |
| PSD / AI → PNG / JPEG | B | ● | — | ✅ | ★★★ | Flattened composite; `.ai` via pdfium where PDF-wrapped |
| RAW (CR2/CR3/NEF/ARW/RAF/DNG/ORF) → JPEG/DNG/TIFF | B | ● | E | ✅ | ★★★★ | libraw. AI-E: denoise |
| Favicon / multi-size ICO generation | A | ● | — | ✅ | ★★★ | |
| Image → PDF (single or contact sheet) | B | ● | P | ✅ | ★★★★★ | AI-P: auto-deskew, page-edge detection |
| PDF page → image | B | ● | — | ✅ | ★★★★★ | pdfium |
| Image sequence ↔ animated GIF/WebP/APNG | B | ● | — | ✅ | ★★★ | |
| EPS / PostScript → image | B | X | — | ✅ | ★★ | **Module only, and only once the microVM tier exists.** A PostScript interpreter is the one engine that has escaped its own sandbox in the wild. |
| The ImageMagick long tail (~200 formats) | B | X | — | ✅ | ★ | Opt-in module; delegates disabled |

### 1.2 Image operations

| Operation | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **Compress to target size / quality** | B | ● | **P** | ✅ | **348,000** | Metric-guided search beats fixed quality every time |
| Resize / scale / fit | B | ● | — | ✅ | ★★★★★ | |
| **Smart crop / reframe** | B | ● + A | **E** | ✅ | ★★★★★ | Saliency + face detection so it doesn't decapitate people |
| Rotate / flip / auto-orient | **A** | ● | — | ✅ | ★★★★ | Lossless JPEG rotation where dimensions allow |
| Colour space & ICC transform | B | ● | — | ✅ | ★★ | lcms2 — explicit, never reinterpreted |
| HDR gain-map handling | B | ● | — | ✅ | ★ | |
| Metadata: strip / preserve / edit | **A** | ● | — | ✅ | ★★★ | Per-profile policy; `Share` removes GPS, serial, author |
| Batch rename from template | A | ● | E | ✅ | ★★ | AI-E: fields extracted from content |
| Watermark / stamp | B | ● | — | ✅ | ★★ | |
| **Vectorize (raster → SVG)** | B | ● | E | ✅ | ★★★ | VTracer; AI picks parameters |
| Near-duplicate detection | C | ● + A | E | ✅ | ★★ | pHash + CLIP embeddings |

---

## 2. Audio

**Fully core — no FFmpeg required.** Symphonia (pure Rust, MPL-2.0) demuxes and decodes; permissive encoders on output.

| Conversion / operation | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **Any → MP3** | B | ● | — | ✅ | **263,000** | LAME |
| **Video → MP3 / M4A** (audio extraction) | A/B | ● | — | ✅ | **64,000** | Demux is Class A; transcode is B. **No FFmpeg needed.** |
| WAV ↔ FLAC ↔ ALAC | **A** | ● | — | ✅ | ★★★★ | Genuinely lossless |
| M4A / AAC / OGG / Opus / WMA / AIFF → any | B | ● | — | ✅ | ★★★★★ | |
| Any → Opus | B | ● | — | ✅ | ★★★ | Royalty-free default |
| Any → AAC | B | V | — | ✅ | ★★★ | Patent-encumbered; OS encoder or module |
| Trim / split / join | **A** | ● | E | ✅ | ★★★ | AI-E: silence detection (Silero VAD, 2 MB) |
| **Loudness normalize** (EBU R128) | B | ● | — | ✅ | ★★ | |
| Tag / chapter / cover-art editing | **A** | ● | — | ✅ | ★★ | |
| Sample-rate / channel-layout conversion | B | ● | — | ✅ | ★★ | |
| **Transcription → SRT/VTT/TXT/JSON** | **C** | A | **R** | ✅ | ★★★★★ | Whisper base int8 — 74 MB |
| **Text → speech / audiobook** | D | A | **R** | ✅ | ★★★★ | Kokoro-82M — 80 MB |
| **Noise / hum / reverb removal** | D | A | **R** | ✅ | ★★★★ | DeepFilterNet — 10 MB |
| **Stem separation / vocal removal** | D | A | **R** | ❌ | ★★★★ | HTDemucs — GPU strongly recommended |
| Speaker diarization | C | A | **R** | ⚠️ | ★★ | sherpa-onnx + CAM++ |
| BPM / key / language detection | C | A | E | ✅ | ★ | |

---

## 3. Video

The split that matters: **Class A works in core; Class B needs the Video module.**

### 3.1 Core — no FFmpeg, no codecs, no patents

| Operation | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **Remux MKV ⇄ MP4 ⇄ MOV ⇄ WebM** | **A** | ● | — | ✅ | ★★★★★ | **Stream copy — seconds, bit-identical.** The flagship. |
| Lossless trim / concat at keyframes | **A** | ● | E | ✅ | ★★★★ | AI-E: scene & silence detection |
| Rotate / re-tag / set metadata | **A** | ● | — | ✅ | ★★★ | Container flag, no re-encode |
| Add / remove / reorder tracks | **A** | ● | — | ✅ | ★★ | |
| **Extract audio track** | **A** | ● | — | ✅ | **64,000** | Copy, then optional audio transcode |
| Embed soft subtitles | **A** | ● | — | ✅ | ★★★ | |
| **Generate subtitles from video** | **C** | ● + A | **R** | ✅ | ★★★★ | Demux audio → Whisper. No video decode needed. |
| **Transcript from video** | **C** | ● + A | **R** | ✅ | ★★★ | Same path |
| Inspect: streams, codecs, bitrate, HDR metadata | — | ● | — | ✅ | ★★ | |

### 3.2 Video module — transcoding

| Operation | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **Transcode to MP4/MKV/WebM** (codecs differ) | B | V | P | ⚠️ | ★★★★★ | AV1/VP9 default; H.264 via OS encoder |
| **Compress to target size or VMAF** | B | V | **P** | ⚠️ | ★★★★★ | Per-title encoding, not a fixed bitrate |
| Resize / crop / pad / fps change | B | V | — | ⚠️ | ★★★★ | |
| **Auto-reframe to 9:16 / 1:1** | B | V + A | **E** | ⚠️ | ★★★★ | Subject tracking (RF-DETR) |
| Video → GIF / animated WebP | B | V | P | ✅ | ★★★★ | |
| **Burn-in subtitles** | B | V | — | ⚠️ | ★★★ | libass |
| **HDR → SDR tone mapping** | B | V | — | ⚠️ | ★★ | Explicit operator (BT.2390/Hable) — never silent |
| Thumbnail / contact sheet / preview clip | B | V | E | ✅ | ★★ | |
| Deinterlace, denoise, stabilize | B | V | — | ⚠️ | ★★ | Stabilization is GPL → separate module |
| **Upscale** | D | V + A | **R** | ❌ | ★★★ | Real-ESRGAN |
| **Frame interpolation 24→60 fps** | D | V + A | **R** | ❌ | ★★ | RIFE ⚠️ weight license needs verification |
| **Remove video background** | D | V + A | **R** | ❌ | ★★★ | Per-frame matting |

---

## 4. Documents & PDF

The largest and most valuable family, and where AI is most often *required* rather than decorative.

### 4.1 PDF operations

| Operation | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **Merge / split / reorder / extract pages** | **A** | ● | E | ✅ | ★★★★★ | qpdf. AI-E: *split by chapter or invoice boundary* |
| **Compress PDF** | B | ● | **P** | ✅ | ★★★★★ | Metric-guided image re-encode inside the PDF |
| Rotate / crop / resize pages | **A** | ● | — | ✅ | ★★★ | |
| **PDF → image (JPG/PNG/TIFF)** | B | ● | — | ✅ | ★★★★★ | pdfium |
| **Image → PDF** | B | ● | P | ✅ | ★★★★★ | |
| Watermark / stamp / page numbers | B | ● | — | ✅ | ★★ | |
| Encrypt / decrypt *(user-supplied password only)* | **A** | ● | — | ✅ | ★★★ | Never password cracking |
| Fill forms / extract form data | A/C | ● | E | ✅ | ★★★★ | |
| Digital signature (sign / verify) | **A** | ● | — | ✅ | ★★★★ | |
| Linearize / repair / optimize | **A** | ● | — | ✅ | ★★ | qpdf has strong repair |
| **PDF/A archival conversion** | B | O | — | ✅ | ★★ | Via LibreOffice export |
| **Paranoid rebuild** (structural or pixel) | B | X | E | ✅ | ★★ | CDR. **Class B, not A** — structural mode discards source structure and pixel mode discards the text layer; neither round-trips. Deferred with the microVM tier. |
| **True redaction** (remove objects, not draw boxes) | **A** | ● + A | **E** | ✅ | ★★★ | AI proposes PII, human confirms, qpdf removes |

### 4.2 Document conversion

| Conversion | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **PDF → DOCX** | C | ● + A | **R** for scans, E otherwise | ✅ | **220,000** | The flagship. Layout + tables is the whole game. |
| **DOCX → PDF** | B | O | — | ✅ | ★★★★★ | Fidelity is a font/layout problem, not an AI one |
| **PDF → XLSX / CSV** | C | ● + A | **R** | ✅ | ★★★★ | Table structure recognition |
| XLSX / PPTX ↔ PDF | B | O | E | ✅ | ★★★★ | |
| **Scanned PDF → searchable PDF (OCR)** | **C** | ● + A | **R** | ✅ | ★★★★ | PaddleOCR mobile — **21 MB** |
| **Image → text (OCR)** | **C** | ● + A | **R** | ✅ | ★★★★★ | Same model |
| **PDF → Markdown / JSON** | C | ● + A | **R** | ⚠️ | ★★★ *(rising)* | The RAG/AI-pipeline case; highest B2B trajectory |
| PDF → plain text | A/C | ● | — / R | ✅ | ★★★★ | Class A when a text layer exists |
| Markdown ↔ HTML ↔ DOCX ↔ PDF ↔ LaTeX ↔ RST | B | O | — | ✅ | ★★★ | Pandoc |
| **HTML / URL → PDF** | B | O | — | ✅ | ★★★★ | Big server-side demand (invoices, reports) |
| ODT / ODS / ODP ↔ Office | B | O | — | ✅ | ★★ | |
| Pages / Numbers / Keynote → Office / PDF | B | O | — | ✅ | ★★★ | Underserved |
| RTF / TXT / DJVU / WPD → any | B | O | — | ✅ | ★★ | |
| **Translate document, layout preserved** | C | A | **R** | ✅ | ★★★★ | Opus-MT per-pair — 75–300 MB each |
| **Summarize / classify / auto-route** | C | A | **R** | ⚠️ | ★★ | Batch document routing |
| **Handwriting → text** | C | A | **R** | ⚠️ | ★★★ | TrOCR / VLM |
| **Formula → LaTeX** | C | A | **R** | ⚠️ | ★★ | pix2tex |
| **Accessibility tagging (PDF/UA) + alt text** | C | A | **R** | ⚠️ | ★ | Procurement gate for public sector |

---

## 5. Ebooks & publishing

| Conversion | Class | Pkg | AI | CPU | Demand |
|---|---|---|---|---|---|
| EPUB ↔ MOBI ↔ AZW3 ↔ FB2 ↔ LIT | B | E | — | ✅ | ★★★ |
| EPUB ↔ PDF | B | E | — | ✅ | ★★★ |
| **PDF → EPUB (reflowable)** | C | A | **R** | ⚠️ | ★★★ | Fixed-layout → reflow requires understanding structure |
| **Scanned book → EPUB** | C | A | **R** | ⚠️ | ★★ |
| **Document → audiobook (M4B, chaptered)** | D | A | **R** | ✅ | ★★ | Kokoro + document IR chapters |
| Metadata & cover editing | A | E | E | ✅ | ★★ |
| CBZ / CBR → PDF / EPUB | B | ● | — | ✅ | ★★ |

---

## 6. Data & tabular

| Conversion | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| **CSV / TSV ↔ JSON** | A/B | ● | **E** | ✅ | ★★★★ | AI-E: **encoding, delimiter, header and type inference** — the actual CSV problem |
| **XLSX / XLS / ODS → CSV / JSON / Parquet** | B | ● | E | ✅ | ★★★★ | AI-E: unmerge cells, detect headers, de-pivot |
| JSON ↔ YAML ↔ TOML ↔ XML | A | ● | — | ✅ | ★★★★ | |
| CSV / JSON → Parquet / Arrow | A | ● | E | ✅ | ★★ | Data-engineering lane |
| JSONL / NDJSON handling | A | ● | — | ✅ | ★★ | |
| SQLite ↔ CSV / Parquet | A | ● | — | ✅ | ★★ | DuckDB |
| **Screenshot of a table → CSV / XLSX** | C | A | **R** | ⚠️ | ★★ | Delightful, highly shareable demo |
| Schema inference report | C | ● | E | ✅ | ★ | |
| Encoding detection & repair | B | ● | E | ✅ | ★★ | |

---

## 7. Archives

| Operation | Class | Pkg | CPU | Demand | Notes |
|---|---|---|---|---|---|
| Extract ZIP / TAR(.gz/.bz2/.xz/.zst) / 7z / RAR / CAB / ISO | A | ● | ✅ | ★★★★ | RAR **extract only** — compression is proprietary |
| Create ZIP / TAR.* / 7z / ZSTD | A | ● | ✅ | ★★★ | |
| Repack / recompress between formats | A | ● | ✅ | ★★ | |
| Inspect without extracting | — | ● | ✅ | ★★ | |
| Convert archive contents in place | varies | ● | ✅ | ★ | Applies a recipe to every member |

Archives get **three** controls, all first-class threats rather than edge cases:

**Bombs.** Ratio caps, nesting depth, entry counts, and total-uncompressed-size limits, all carried on the `Step` as `Limits` and checked against a **running total during extraction** — so a bomb is stopped before it is realised, not detected after it has filled the disk. A per-job `Budget` catches the other version of this, where forty thousand entirely legal files fill a disk without any single one exceeding anything. ([A5 / SR-5](09-THREAT-MODEL.md#3-attack-classes))

**Path traversal.** An archive member's name is not a path. Each component is parsed by `OutputName`, which rejects `..`, separators, absolute and drive-relative forms, NTFS alternate data streams, Windows reserved device names, and trailing dots. Symlink, hardlink, device and FIFO entries are refused. The check runs inside the sandbox *and* again in the trusted copy-out. ([A12 / SR-13](09-THREAT-MODEL.md#3-attack-classes))

**Destruction.** *(New in v0.4, and the one the first two do not cover.)* A member named `.bashrc`, `id_rsa` or `Brand-Guidelines.pdf` is a perfectly legal single path component — every traversal check passes it, correctly — and in most extractors it silently replaces the file that was there. **Every create is `O_EXCL`; no truncating open exists anywhere in the codebase, and a source lint proves it.** `Policy::on_conflict` (`Suffix` by default) is resolved *before* anything is opened, and the receipt records the name actually written. ([A13 / SR-15](09-THREAT-MODEL.md#3-attack-classes))

---

## 8. 3D, CAD & GIS

| Conversion | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| STL ↔ OBJ ↔ glTF/GLB ↔ PLY ↔ 3MF ↔ FBX ↔ DAE | B | X | E | ✅ | ★★★ | Assimp. AI-E: decimation targets |
| Mesh decimate / normalize units & axes | B | X | E | ✅ | ★★ | 3D-printing audience |
| 3D → preview render / turntable | B | X | — | ⚠️ | ★ | |
| DWG / DXF → PDF / SVG / DXF | B | X | — | ⚠️ | ★★★ | Hard, valuable, professional |
| STEP / IGES → mesh | B | X | — | ⚠️ | ★★ | |
| Shapefile ↔ GeoJSON ↔ KML ↔ GPKG ↔ GPX | A/B | X | — | ✅ | ★★ | GDAL. Small audience, real budgets |
| Raster GIS (GeoTIFF) ↔ formats | B | X | — | ✅ | ★ | |

---

## 9. Email, fonts, subtitles & misc

| Conversion | Class | Pkg | AI | CPU | Demand | Notes |
|---|---|---|---|---|---|---|
| EML / MSG / MBOX / PST → PDF / MD / EML | B | X | E | ✅ | ★★ | Legal e-discovery. Attachments extracted recursively. |
| SRT ↔ VTT ↔ ASS ↔ SUB ↔ TTML | A/B | ● | — | ✅ | ★★★ | ASS→SRT is lossy; warned |
| **Translate subtitles** | C | A | **R** | ✅ | ★★★ | Opus-MT |
| Subtitle resync / shift / merge | A | ● | E | ✅ | ★★ | |
| TTF ↔ OTF ↔ WOFF ↔ WOFF2, subset | B | X | — | ✅ | ★★ | Font EULA warning shown |
| IPYNB ↔ MD / HTML / PDF | B | ● | — | ✅ | ★★ | |
| DICOM → PNG / PDF *(read-only)* | B | X | — | ✅ | ★ | Regulated, high-trust, on-brand |
| vCard / iCal ↔ CSV / JSON | A | ● | — | ✅ | ★ | |
| Base64 / hex / URL encode-decode | A | ● | — | ✅ | ★★ | |

---

## 10. AI features

The second master table. **Every model listed is permissive-licensed and bundleable** unless marked 🟡 (user-fetched only) — see [§11](#11-model-licensing-policy).

### 10.1 Tier S — ships as "AI Pack v1"

The whole tier is **~500 MB quantized and runs CPU-only on 8 GB RAM.**

| # | Capability | Class | Model | License | Size ⚠️ | CPU | Enables |
|---|---|---|---|---|---|---|---|
| 1 | **Text from images / OCR** | C | PaddleOCR mobile (det+rec) | Apache-2.0 | **~21 MB** | ✅ | image→text, scanned PDF→searchable, PDF→DOCX for scans |
| 2 | **Transcription** | C | Whisper base int8 | MIT | ~74 MB | ✅ | audio/video → SRT/VTT/TXT, searchable media |
| 3 | Voice activity detection | C | Silero VAD | MIT | ~2 MB | ✅ | silence trim, auto-chapters, ASR segmentation |
| 4 | **Background removal** | D | u2netp / BiRefNet-lite | Apache-2.0 / MIT | **~5 / 90 MB** | ✅ | transparent PNG as a *target format* |
| 5 | **Upscaling** | D | Real-ESRGAN compact | BSD-3 | ~2–64 MB | ⚠️ | 2×/4× image enlargement |
| 6 | **Narration (TTS)** | D | Kokoro-82M | **Apache-2.0** | ~80 MB | ✅ | document → audiobook, narration |
| 7 | **Noise removal** | D | DeepFilterNet | Apache/MIT | ~10 MB | ✅ | clean speech audio |
| 8 | **Document structure** | C | Granite-Docling-258M | **Apache-2.0** | ~248 MB | ⚠️ | PDF→Markdown/JSON, tables, reading order |

### 10.2 Tier A — second wave

| Capability | Class | Model | License | Size ⚠️ | CPU | Enables |
|---|---|---|---|---|---|---|
| **Translation** | C | Opus-MT (per language pair) | Apache-2.0 | 75–300 MB each | ✅ | document & subtitle translation, layout preserved |
| **Table extraction** | C | TableFormer / PP-Structure | Apache-2.0 | ~100 MB | ⚠️ | PDF→XLSX, screenshot→CSV |
| **PII detection** | C | Presidio + GLiNER | MIT / Apache-2.0 | ~200 MB | ✅ | redaction proposals, share-safety warnings |
| **Object detection** | C | **RF-DETR / RT-DETR** | **Apache-2.0** | ~60–120 MB | ✅ | smart crop, auto-reframe, subject tracking |
| **Image restoration** | D | SCUNet / NAFNet | Apache/MIT ⚠️ | ~50 MB | ⚠️ | denoise, deblur, JPEG-artifact cleanup |
| **Inpainting / object removal** | D | LaMa | **Apache-2.0** | ~200 MB | ⚠️ | remove objects from photos |
| **Faster ASR** | C | Parakeet-TDT 0.6B | CC-BY-4.0 | ~300–600 MB | ⚠️ | much faster transcription, 25 languages |
| **Stem separation** | D | HTDemucs | MIT | ~300 MB | ❌ | vocals/drums/bass/other |
| Speaker diarization | C | sherpa-onnx + CAM++ | Apache-2.0 | ~80 MB | ⚠️ | "who spoke when" |
| **Image captioning / alt text** | C | Florence-2 base | **MIT** | ~460 MB | ⚠️ | accessibility, auto-tagging |
| Semantic search / dedupe | C | SigLIP / OpenCLIP | Apache / MIT | ~90 MB | ✅ | near-duplicate detection, auto-tagging |

### 10.3 Tier B — depth

| Capability | Class | Model | License | CPU | Enables |
|---|---|---|---|---|---|
| Document VLM (Q&A, extraction) | C | Qwen3-VL 4B | Apache-2.0 | ❌ | invoice/form field extraction, classification |
| **Intent engine** (natural language → plan) | C | Qwen3 4B + LoRA | Apache-2.0 | ⚠️ | "make these under 2 MB for email" |
| **Conversion advisor** (which option, and why) | C | *same model as the intent engine* | Apache-2.0 | ⚠️ | "for a client on an old Word" · "for 20-year archival" · "why is my PDF 40 MB?" |
| High-accuracy OCR | C | PaddleOCR-VL / DeepSeek-OCR | Apache-2.0 / MIT | ❌ | complex layouts, CJK, handwriting |
| Handwriting recognition | C | TrOCR | MIT | ⚠️ | scanned notes |
| Formula recognition | C | pix2tex | MIT | ⚠️ | math → LaTeX |
| Colorization | D | DDColor | Apache-2.0 ⚠️ | ⚠️ | B&W photo colorization |
| Video upscale / interpolation | D | Real-ESRGAN / RIFE ⚠️ | mixed | ❌ | video enhancement |
| Depth estimation | C | Depth Anything V2 **Small** | Apache-2.0 | ⚠️ | 2D→3D, background effects |

### 10.4 Cross-cutting AI behaviors

| Feature | Class | Where | Notes |
|---|---|---|---|
| **The prediction engine** | — | Core | **Not an LLM, and not a learned ranker in v1.** Five hand-weighted signals, <10 ms, CPU, explainable. It ranks *targets*, never plans, and cannot arm a Class D operation. See [03-ARCHITECTURE §14](03-ARCHITECTURE.md#14-prediction). A learned ranker is deferred until there is data to train it on. |
| **Format knowledge base** | — | Core | Compatibility matrices, lossy/lossless facts, patent status, platform support — **data in the format and route tables, not in model weights.** Answers most "which should I use" questions with zero download and zero hallucination risk. Prints as `openconvert formats` and generates the `/formats` site section. |
| Quality-target solver | P | Core | "under 2 MB", "VMAF ≥93" — metric loop, not a model |
| Auto-parameter selection | P | Core | Encoder settings chosen from content analysis |
| Batch auto-rename from extracted fields | C | AI pack | `{doc.invoice_no}`, `{audio.speaker_1}` |
| Auto-classify & route (watch folders) | C | AI pack | Document triage |
| Hardware backend auto-selection | — | Core | Measured per model on first use, not guessed |

---

## 11. Model licensing policy

**The project is Apache-2.0. That does not restrict which models OpenConvert can *run* — only which it can *ship*.**

Three tiers, enforced in CI:

| Tier | Rule | Examples |
|---|---|---|
| 🟢 **Bundleable** | Permissive (Apache-2.0, MIT, BSD, CC-BY), redistributable, commercial use permitted. Ships in the installer or the signed model index. | Whisper, Kokoro, BiRefNet, u2netp, Real-ESRGAN, PaddleOCR, Granite-Docling, LaMa, RF-DETR, Opus-MT, Florence-2, HTDemucs |
| 🟡 **User-fetched** | Any other license. OpenConvert ships an **adapter, never weights**. The user obtains the model from its source, sees the license before download, and a persistent "unverified/restricted models enabled" indicator appears. | RMBG-2.0 (CC-BY-NC), CodeFormer (S-Lab NC), NLLB (CC-BY-NC), Surya (RAIL-M), Marker weights (NC), Depth Anything V2 Large (CC-BY-NC), YOLO (AGPL) |
| 🔴 **Excluded** | Cannot be legally fetched or run by end users; or requires pickle-format loading. | Any pickle-only checkpoint; anything with no distributable license |

**Format allowlist:** safetensors, GGUF, ONNX only. **Pickle is rejected at the file-type layer** — it executes arbitrary code on load, and scanner bypasses are a known zero-day class. ([A9 / SR-8](09-THREAT-MODEL.md#3-attack-classes))

**How "enforced in CI" is actually enforced.** `cargo-deny` sees Rust crates and nothing else — not native engines, not model weights, which is where the real licence risk lives. So there are three tables and three gates, live from week 1 ([08-EXECUTION-PLAN §4](08-EXECUTION-PLAN.md#4-the-ci-gates)):

| Table | Covers | Gate |
|---|---|---|
| `deny.toml` | Rust crates | no GPL/AGPL/non-commercial in the dependency tree |
| `engines.toml` | native engines | every engine referenced in code is declared with its licence **and its linkage** — subprocess vs linked, because that distinction *is* the GPL boundary |
| `models.toml` | model weights | every model declared with licence, `commercial_use`, and sha256. **The build fails if any model with `commercial_use = false` is referenced from the bundled set.** |

> **On "commercial build."** v0.3 said the gate fails "a commercial build," which cannot be right for a project that ships **one artifact with no feature gating, ever** ([01-VISION §9](01-VISION.md#9-business-model-in-one-paragraph)). Apache-2.0 lets anyone redistribute what we ship, commercially, so a non-commercial model bundled in "the free build" is a problem for every downstream redistributor and therefore for us. There is one build, and **nothing with `commercial_use = false` is ever bundled into it.** Restricted models remain 🟡 user-fetched, which is what that tier is for.

**Enterprise affordance:** `openconvert doctor --licenses` renders those tables for the installed set, and a policy key can forbid non-permissive models entirely. Compliance teams ask for exactly this, and the answer is generated rather than maintained.

---

## 12. Application features

### 12.1 Getting files in

| Surface | Resident cost | Default | When |
|---|---|---|---|
| Main window (whole body is a drop target) | none | on | v1 |
| **Explorer / Finder context menu** — shows the top 3 *predictions*, not a static list | **The most expensive row in the product, and it is not in v1.** See below. | off | **v1.1** |
| Dock / taskbar icon drop (launches the app) | none | on | v1 |
| macOS Services / Quick Actions / Share sheet | none | on | v1 |
| CLI | on demand | on | v1 |
| **Edge Strip** — drag toward any screen edge, top 3 predictions expand | ~15 MB helper, 0% CPU | **off** | post-v1 |
| Tray / menu-bar icon · global hotkey → convert clipboard | same helper | off | post-v1 |
| Watch folders · REST · MCP | on demand | off | post-v1 — each has a named trigger |

### Why the shell extension is not in v1

It is **the only place in the product where our code runs outside our own process**, and v0.3 shipped it in v1, on by default, with no requirement, no test, no CI gate and no file in the schema. That combination is how a product's worst attack surface ends up being the one nobody modelled — so it is now [A15](09-THREAT-MODEL.md#3-attack-classes), it has [SR-17](09-THREAT-MODEL.md#5-requirements-to-tests), and it ships in v1.1.

The problem is not effort, it is a contradiction v0.3 wrote down twice and did not notice:

- On Windows a context-menu handler is loaded **into `explorer.exe`** — the shell process, at the user's full privilege, where a panic takes down the desktop and the COM ABI needs `unsafe` in a crate the `forbid(unsafe_code)` policy never contemplated.
- v0.3 permitted it "no file read beyond a header" in one sentence and forbade it to "parse a file" in the next. **Reading a header is parsing untrusted bytes.**
- Worse, prediction signal #1 is *detected file type*, and [SR-4](09-THREAT-MODEL.md#5-requirements-to-tests) requires type to come from content, never from an extension. So the handler had to either run our sniffer inside `explorer.exe`, or violate SR-4 in the surface most users meet first.

**The v1.1 design resolves it by giving something up.** The handler **opens no file at all**: it predicts from the extension, the sibling listing and local history only, and marks its suggestion *provisional* until the app itself has sniffed the file — so a `.jpg` that is really PostScript gets the wrong chip in the context menu for the half-second before the real answer replaces it, and that is an acceptable trade for not parsing hostile bytes in the shell. Out-of-process (`IExplorerCommand` in a packaged app) where the OS allows; every panic caught at the FFI boundary; a source lint asserts no file-open API appears in the crate. On macOS a Finder extension is already a separate sandboxed process, which is why that half is cheaper and safer.

### 12.2 The conversion experience

| Feature | Notes |
|---|---|
| **Prediction with confidence gating** | ≥85% arms the top suggestion; <60% shows a chooser instead of guessing badly |
| **Explanation on every suggestion** | "you've done this 14 times" — one line, always visible, clickable for the full breakdown |
| **Ranked alternates** | Numbered 1–9, one keypress each |
| **Fuzzy lookup** | Over every target, tool, and recipe; aliases (`jpg`/`jpeg`, `word`/`docx`) |
| **Natural language** | Optional, removable; always emits a reviewable plan, never a command |
| **Plan preview** | Per-file steps, engines, classes, **limits, isolation, and the sandbox profile that will actually be in force** — *before* committing. It isn't a feature; it's what you get by not calling `execute`. |
| **Cost estimates for this machine** | Time and output size, measured not guessed |
| **Mixed-drop grouping** | 12 HEIC + 3 MOV + 1 PDF → three predictions, one "Convert all" |
| **Receipts** | Full provenance sidecar: engines, versions, params, class, limits, isolation, sandbox profile, `content_id`, declared-vs-detected type, hashes. **Written as `<output>.receipt.json` beside the output by default** (`--receipts <dir>` to collect them, `--no-receipt` to decline one). Records the file's *name* and content hash, never its absolute source path, because a receipt often ends up in the same shared folder as the output. **A receipt that cannot be written fails the step and removes the output** — an unaccounted-for output is worse than no output. *(C2PA is post-v1; until then a receipt does not travel with a file that is emailed or uploaded, which the docs say plainly rather than implying otherwise. `openconvert verify` reads receipts, so a receipt from elsewhere is untrusted input and is parsed and fuzzed as such.)* |
| **Undo** | 60 seconds prominently, then in History. Safe because originals are never modified **and nothing is ever overwritten** — undo removes outputs, so it could not restore a file a conversion had replaced. That is why [SR-15](09-THREAT-MODEL.md#5-requirements-to-tests) is structural rather than a preference. |
| **Save as recipe** | Turns what just happened into a reusable, shareable file |
| **Resumable batches** | Crash or quit → resume from the journal |
| **Instant mode** | Opt-in zero-click for high-confidence drops, with undo |

### 12.3 Automation & customization

| Feature | Notes |
|---|---|
| **Recipes** *(weeks 31–33)* | `*.recipe.toml` — parameterized, shareable, **version-pinnable**. A recipe stores a `PlanRequest` and a plan hash, **never a `Plan`**: opening one re-routes on this machine, and a hash mismatch is shown rather than executed. That is what makes it safe to accept a recipe from a stranger, and why *signing* is deferred until a key model exists — an unsigned recipe and a signed one from an unknown key are treated identically, so the signature buys attribution, not safety. v0.3 listed recipes as a feature, gave them a fuzz target, and gave them no week; they now have one |
| **Studio** | Node canvas: branch, conditionals, live preview. Saves to the same recipe format. *(Deferred — trigger: issue demand.)* |
| **Watch folders** | Rule builder: match → plan → destination, with dry-run. *(Post-v1; also the daemon's trigger.)* |
| **Profiles** | Named default bundles (Personal / Work-Confidential / Web / Archive) |
| **Naming templates** | `{name}_{width}x{height}_{date}` plus AI-extracted fields. **Every rendered name is parsed as a `DestinationName`** before it reaches a filesystem, because `{name}` comes from a file whose name we did not choose |
| **Batch manifest** | `openconvert batch --manifest out.jsonl` — one line per file: source name, `content_id`, plan, class, engine versions, output, receipt path, outcome. The archivist's "40k files, losslessly, **with a manifest**" needs one artefact, not 40,000 sidecars |
| **Layered config** | **Three layers: defaults < user < flags.** Org-policy, profile, workspace and recipe layers are deferred — six layers of resolution with one real user is a debugging surface nobody asked for. The org-policy layer's named trigger is the first enterprise deployment. |
| `openconvert config explain <key>` | Prints the value **and which layer set it** — the feature every layered config should have and most don't |
| **Theming & keybindings** | JSON design tokens, chord support, importable presets |
| **Engine & model pinning** | Freeze versions per recipe for byte-reproducible output |

### 12.4 Safety & privacy controls

| Feature | Default |
|---|---|
| **No engine ever reaches the network** | **On, and there is no setting.** The isolation floor is three predicates and its `network: Denied` field has no config key — a machine that cannot deny an engine network access is refused rather than accommodated. [SR-1](09-THREAT-MODEL.md#5-requirements-to-tests) says "ever", so it is not a preference |
| **Isolation floor**: Standard (`Reduced`) / Elevated (`Full`) | **Standard.** Convert on any machine offering meaningful confinement, naming exactly what engaged. Elevated refuses unless the full stack is available, and says which mechanism is missing. *(A hardened Linux box with user namespaces disabled now converts at `Reduced` — Landlock needs no namespace. v0.3's ladder refused it.)* |
| **The sandbox profile is always visible** | on — in the plan preview and the receipt. If AppContainer isn't available, you are told before you commit, not after. |
| Auto-raise the floor to Elevated for files from the internet or email | on — but **this detection fails open**, and we say so: a file that arrived by sync, on a USB stick, or from an archive another tool extracted carries no zone identifier and looks local. It is a useful signal, not a guarantee ([09 §8](09-THREAT-MODEL.md#8-what-we-dont-protect-against)) |
| **Resource limits on every step, and a budget on every job** | on, and not adjustable below a floor. Enforced on the in-process path too. The job budget is what stops forty thousand entirely legal files filling a disk |
| **Path-traversal containment on archive extraction** | on, not optional, checked twice |
| **Nothing is ever overwritten** | **on, and structural.** Every create is `O_EXCL`; no truncating open exists in the codebase; conflicts resolve to `Suffix` / `Skip` / `Fail` before anything is opened |
| **One input, one read** | on — the bytes we route on are the bytes we convert and the bytes the receipt attests to. Untrusted-provenance and network-volume inputs are copied first |
| **Security Response** — engine revocation for exploited versions | **On, and it rides inside the signed update manifest.** Current list and reasons always visible; a *cached* list keeps disabling a revoked engine with the network off |
| **Update check** | **On, weekly — and it is the only network call the app makes by default.** It fetches a signed manifest carrying the revocation list; it sends no identifier, no version, no query — it downloads the whole manifest and compares locally, so it reveals only that some install exists at some IP on some day. The literal request is previewable in settings, and it is disableable. **This is a deliberate change from "no phone-home, ever":** [SR-12](09-THREAT-MODEL.md#5-requirements-to-tests) promises a <7-day kill switch for actively exploited engine CVEs, and a kill switch that only arrives if the user happens to update is a release note, not a mitigation. Turning it off is supported; the settings screen says exactly what you give up |
| **Panic** — cancel every job, kill every worker, wipe temp (global hotkey) | available; cancellation is bounded at 500 ms and asserted in CI |
| Network posture: Offline / **Sealed** (hard-fail on any egress attempt, including the update check) | Offline |
| Metadata policy | `Share` strips GPS/serial/author |
| **Diagnostics** | **Off.** Opt-in, four levels, with a literal-payload preview and a local log of everything sent |
| Prediction history | Local JSONL, viewable, exportable, wipeable, never transmitted — and **it cannot influence a security decision**: it ranks targets, and `route()` decides everything else ([03 §12](03-ARCHITECTURE.md#12-state-on-disk)) |
| Quarantine for polyglots and type mismatches | on |
| Temp cleanup | Owned by the job's worker; a startup sweep reaps anything a crash left behind — **only under our own per-user state directory, never a shared temp root**, because a sweep over a world-writable directory is a deletion primitive. Secure-delete optional. |
| Paranoid mode: Structural or Pixel | *Deferred with the microVM tier — named trigger: a customer with an untrusted-document workflow.* |

---

## 13. Server features

Same core, no GUI. **Deferred out of v1** with a named trigger — *a design partner asking for it* ([03-ARCHITECTURE §15](03-ARCHITECTURE.md#15-the-seams--how-it-grows)). It shipped to zero users in the v0.2 plan, three months before the GUI existed. The CLI is held at full `--json` parity in the meantime, so the server is a thin wrapper over `route()` and `execute()` whose OpenAPI generates from the same types.

| Feature | Notes |
|---|---|
| `POST /v1/plan` | **Dry run** — plan, classes, limits, sandbox profile, estimates, warnings, without converting. No other conversion API has this. |
| `POST /v1/convert` | Takes a `PlanRequest`, **re-routes, and executes what it routed** — it never executes a plan supplied by a client. Optionally carries the plan hash from `/v1/plan`; a mismatch is a 409 with the new plan, because a difference means the environment changed and the caller should know. Sync for small inputs, async with a job ID above a threshold. |
| `GET /v1/capabilities` | **The format and route tables as JSON** — the same data `openconvert routes` prints |
| `POST /v1/inspect` · `/v1/verify` | Type detection & polyglot warnings; SSIM/PSNR/VMAF/text-diff comparison |
| SSE progress, webhooks, idempotency keys | |
| Presigned S3/GCS/Azure I/O, streaming | |
| **MCP server** | Agents drive conversions as typed tool calls |
| `--compat gotenberg,cloudconvert` | Migration is a base-URL change |
| Per-tenant policy, quotas, audit log | API key → policy profile |
| Prometheus metrics, OpenTelemetry traces | |
| Air-gapped bundles | `openconvert bundle create` / `apply` |
| OCI images: `slim` / `full` / `ai` / `gpu` + Helm chart | Nobody pulls 3 GB for `heic→jpg` |

---

## 14. Explicitly excluded

| Excluded | Why |
|---|---|
| **YouTube / platform downloaders** | ~6.12M searches/mo — and the reason this category is associated with malware. Positioning decision, not just legal. |
| DRM removal (ebooks, video) | Illegal in many jurisdictions. DRM inputs refused with a clear message. |
| PDF password **cracking** | We open with a user-supplied password, never by brute force. |
| **Voice cloning** | Not in v1. If ever: local-only, watermarked, consent-attested. |
| **Face swap / identity manipulation** | Not shipped. |
| Watermark removal as a named feature | Generic inpainting exists; we don't market it for this. |
| A hosted OpenConvert conversion service | Recreates exactly the trust problem the project exists to solve. |
| Multi-user web UI | ConvertX's territory; a large surface to secure. |
| Cloud sync / accounts | Nothing to sync. Config is a file you can version-control. |
