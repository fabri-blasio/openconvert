# OpenConvert — Research Foundation

**Status:** research record · 2026-08-11, banner updated 2026-08-13
Part 5 of 9 — see [README](README.md).

> ## ⚠️ Parts 5–11 are superseded. Do not implement from this document.
>
> **This is the evidence base, not the plan.** Parts 0–4 and the appendices are live: the competitive landscape, the CVE record, the engine and format survey, the model inventory, and the licensing analysis. **Parts 5–11 describe a first-pass architecture that no longer exists** — a daemon with a JSON-RPC protocol, eight IR hubs, A\* over a capability graph, four isolation tiers, a module system with a manifest ABI, and six layers of config resolution. All of it was deliberately cut.
>
> **Why this banner is emphatic.** An adversarial review found that keeping the security requirements in this document was itself a defect: five other documents cited it as authoritative, [03-ARCHITECTURE](03-ARCHITECTURE.md) never referenced it, and when the architecture was simplified, controls that had been specified here — universal resource limits, the degraded-sandbox fallbacks — were dropped without anyone noticing. One attack class had never been written down at all.
>
> So the security content has moved. **§2.3 (attack classes) and §2.5 (requirements) now live in [09-THREAT-MODEL](09-THREAT-MODEL.md)**, where the architecture points at them and every requirement has a test that fails the build. Both sections below are stubs pointing there. §8 is retained as historical detail, superseded by the same document.
>
> Where Parts 5–11 conflict with the later documents, **the later documents win** — notably: the license is Apache-2.0 (not source-available), Ghostscript and FFmpeg are both excluded from core, there is no daemon, and the enterprise positioning is CDR and sovereign document AI rather than conversion.
>
> Current design: [03-ARCHITECTURE](03-ARCHITECTURE.md) · [09-THREAT-MODEL](09-THREAT-MODEL.md) · [08-EXECUTION-PLAN](08-EXECUTION-PLAN.md) · [01-VISION](01-VISION.md) · [02-FEATURES](02-FEATURES.md) · [04-GROWTH](04-GROWTH.md)

**Scope:** Competitive/technical research, threat analysis, engine & model landscape, and the first-pass product and architecture design.

---

## How to read this document

| Part | What it answers |
|---|---|
| [0. Executive summary](#0-executive-summary) | Is there a real gap, and what exactly is the wedge? |
| [1. Market & competitive research](#1-market--competitive-research) | What already exists and where each one stops |
| [2. Threat landscape](#2-threat-landscape-why-safe-is-a-feature-not-a-slogan) | Why "safe & secure" is an engineering problem, not marketing |
| [3. Engines, formats & fidelity](#3-engines-formats--fidelity) | What can actually be converted, by what, at what legal cost |
| [4. Local AI landscape](#4-local-ai-landscape) | Which models are real, on what hardware, under what licenses |
| [5. Product design](#5-product-design) | What the app *is*: surfaces, tools, mental model |
| [6. Architecture](#6-architecture) | How it's built: IR hubs, planner, modules, sandboxes |
| [7. Customization & configuration](#7-customization--configuration) | "Fully customizable" made concrete |
| [8. Security architecture](#8-security-architecture) | The specific controls, per OS |
| [9. Roadmap & MVP](#9-roadmap--mvp-scope) | What to build first, what to cut |
| [10. Business model](#10-business-model-options) | How it sustains itself |
| [11. Risks & open questions](#11-risk-register--open-questions) | What kills this project |
| [Appendices](#appendix-a--target-format-coverage) | Manifests, recipes, CLI, sources |

**Confidence marking.** Claims sourced from primary documentation (project READMEs, vendor docs, CVE writeups, license texts) are stated plainly. Claims sourced from secondary/SEO-grade blog content are marked ⚠️ and should be re-verified before any decision depends on them. See [§12 Research gaps](#12-research-gaps--what-to-verify-next).

---

## 0. Executive summary

### 0.1 The thesis

File conversion is a solved problem *technically* and an unsolved problem *as a product*. The engines exist and are free (FFmpeg, libvips, LibreOffice, Pandoc, Calibre, Assimp, pdfium). What doesn't exist is a single artifact that is simultaneously:

1. **Local by default** — nothing leaves the machine, verifiably
2. **Actually safe** — because "local" means *you* now run the hostile parser on *your* CPU
3. **Broad** — one tool instead of eleven
4. **Intelligent** — on-device models that do the things conversion alone can't
5. **Modular** — you install only what you need, and third parties can extend it
6. **Honest** — every output is accounted for: what changed, what was lost, what was inferred

Every existing product picks 2–3 of those six. The gap is the combination.

### 0.2 The gap, stated precisely

| Existing archetype | Example | Where it stops |
|---|---|---|
| Cloud SaaS converter | CloudConvert, Convertio, Zamzar | Files leave your machine. Retention policies vary and are usually unverifiable. |
| Browser-local converter | VERT | Excellent privacy story, but WASM ceiling: video conversion on the public instance is offloaded to a server daemon. |
| Self-hosted web converter | ConvertX | 1000+ formats via bundled engines, but it's a *web app you must operate*, with no sandboxing story per-engine and no AI layer. |
| Domain toolkit | Stirling PDF, HandBrake, ImageMagick | Deep in one domain, absent elsewhere. |
| Security-first converter | Dangerzone | Best-in-class isolation model, but it does one thing: dangerous doc → safe PDF. |
| Local AI point tool | Upscayl, chaiNNer, Video2X, Buzz | One AI capability each, no conversion substrate, no shared model management. |

Nobody ships **the safe substrate + the breadth + the AI layer + a real module system** in one installable app.

### 0.3 The wedge (what to lead with)

Not "converts 1000 formats" — that's table stakes and unprovable in marketing. Lead with the two things competitors structurally cannot copy quickly:

1. **Verifiable conversion.** Every output ships with a *Conversion Receipt*: engines + versions, exact parameters, an operation class (deterministic / lossy / inferred / generative), input+output hashes, and a measured fidelity delta where computable (VMAF, SSIM, text diff, checksum-identical stream copies). No other converter tells you what it did to your file.
2. **Hostile-input isolation as a product feature.** Untrusted files are parsed inside per-engine sandboxes, with an optional "Paranoid mode" that reconstructs documents through a pixel/AST bottleneck (the Dangerzone technique) so no original parser structure survives. This is a *checkbox in the UI*, not a whitepaper.

AI is the third pillar, but it must be framed as **capability**, not as **magic that mutates your files**. See the determinism boundary in [§5.9](#59-the-determinism-boundary-the-single-most-important-design-rule).

### 0.4 The three biggest risks (details in §11)

1. **Codec patents and engine licensing** — FFmpeg's LGPL/GPL split and H.264/HEVC/AAC patent pools constrain what a *commercial* binary may ship. This shapes packaging on day one, not later.
2. **The CVE treadmill** — you are shipping ImageMagick/Ghostscript/LibreOffice-class parsers to end users. That is a permanent security-response obligation.
3. **Scope explosion** — "all-in-one" is an unbounded promise. The module system is the mechanism that makes the promise bounded; without it this project never ships.

---

## 1. Market & competitive research

### 1.1 Market shape

Market-research vendors put file-conversion software at roughly **USD 1.0–1.5B (2024–25)**, growing to **USD 2.3–3.2B by 2033**, with CAGR estimates spanning **5.5%–11.1%** depending on the firm, and online/web-based tools estimated at ~60% of the market. ⚠️ These are syndicated-report figures with wide methodological spread; treat as order-of-magnitude only. The useful signal is directional: it's a real, growing, fragmented category dominated by web tools — which is exactly the segment a privacy-and-safety product attacks.

The more interesting market signal is a trust event: in **March 2025 the FBI Denver Field Office publicly warned that free online file-converter sites were being used to distribute malware** (ransomware droppers, credential stealers, miners). The category's incumbents are ad-supported websites of unknown provenance. That is a soft underbelly, and it is the marketing story for a signed, local, offline binary.

### 1.2 Category map

```
                        │ Runs locally │ Broad formats │ AI tools │ Modular │ Sandboxed
────────────────────────┼──────────────┼───────────────┼──────────┼─────────┼───────────
CloudConvert / Zamzar   │      no      │     yes       │  partial │   no    │  n/a (theirs)
Convertio / FreeConvert │      no      │     yes       │  partial │   no    │  n/a
VERT.sh                 │   mostly*    │     yes       │    no    │   no    │  browser
ConvertX (self-host)    │   yes (svr)  │     yes       │    no    │   no    │  container only
Stirling PDF            │   yes (svr)  │   PDF only    │  OCR     │   no    │  container only
Dangerzone              │     yes      │   docs only   │  OCR     │   no    │  gVisor+ctr ✅
HandBrake / LosslessCut │     yes      │  video only   │    no    │   no    │   no
Upscayl / chaiNNer      │     yes      │  images only  │   yes    │ partial │   no
Video2X                 │     yes      │  video only   │   yes    │ partial │   no
Pandoc / FFmpeg / vips  │     yes      │  per-domain   │    no    │  n/a    │   no
────────────────────────┴──────────────┴───────────────┴──────────┴─────────┴───────────
OpenConvert (proposed)              yes            yes            yes       yes       yes
* VERT: images/audio/docs local via WASM; the public instance offloads video to `vertd`.
```

### 1.3 Deep profiles

#### CloudConvert (cloud, commercial)
- ~212 formats across 11 categories (audio, video, document, ebook, archive, image, spreadsheet, presentation, CAD, RAW, production media).
- **Notable design idea worth stealing:** *pinnable engines and engine versions*. For DOCX you can choose the `office` engine or the `libreoffice` engine, and pin a specific LibreOffice version so output doesn't drift when they upgrade. Reproducibility as a first-class feature — almost nobody else does this.
- Weakness for our purposes: it's a server. Your file is on their infrastructure.

#### VERT.sh (browser-local, open source)
- Svelte + TypeScript front end; conversion via **WebAssembly** in the browser. Claims 250+ formats. Uses established engines compiled to WASM (FFmpeg for media, libvips for images, Pandoc for documents ⚠️ — the README itself doesn't enumerate engines; this attribution comes from secondary coverage).
- **The honest limitation, straight from the project:** video is the exception. The official instance uploads video to `vertd`, a self-hostable Rust+FFmpeg daemon, because in-browser video transcode is too slow. Self-hosting `vertd` restores full locality.
- **Lesson:** WASM is a great *isolation* answer and a mediocre *throughput* answer for video. Our architecture must not bet the video pipeline on WASM. (See tiering in [§6.3](#63-isolation-tiers).)

#### ConvertX (self-hosted, AGPL-3.0)
The most direct prior art for "breadth." Bundled engine set and rough format counts, from the project README:

| Engine | Domain | in → out |
|---|---|---|
| FFmpeg | video/audio | ~472 → ~199 |
| ImageMagick | images | 245 → 183 |
| GraphicsMagick | images | 167 → 130 |
| Assimp | 3D assets | 77 → 23 |
| Vips | images | 45 → 23 |
| Pandoc | documents | 43 → 65 |
| LibreOffice | documents | 41 → 22 |
| Calibre | ebooks | 26 → 19 |
| libjxl | JPEG XL | 11 → 11 |
| VTracer | raster→vector | 8 → 1 |
| Inkscape | vector | 7 → 17 |
| Markitdown | documents | 6 → 1 |
| Dasel | data files | 5 → 4 |
| Potrace | raster→vector | 4 → 11 |
| dvisvgm | vector | 4 → 2 |
| libheif | HEIF | 2 → 4 |
| resvg, XeLaTeX, msgconvert | misc | 1 → 1 each |

- Stack: TypeScript + Bun + Elysia, Docker deployment, JWT auth, auto-delete after N hours, optional VAAPI hardware accel.
- **Where it stops:** it's a multi-user web service you must run; engines execute in one container with no per-engine confinement; there is no AI layer, no module system, no fidelity accounting.
- **Lesson:** this is the coverage bar to match. The engine table above is effectively our Phase-1 shopping list.

#### Stirling PDF (self-hosted, PDF)
- 50–60+ PDF operations, Tesseract OCR in many languages, compression, signing, watermarking, metadata editing, redaction, and a **no-code pipeline builder** where a sequence of operations is saved as a named, re-runnable workflow. v2.9.0 added a viewer for non-PDF types (CSV, JSON, Markdown, images).
- **Lesson:** the saved-pipeline concept is exactly right and generalizes far past PDF. We call it a **Recipe** ([§5.4](#54-the-core-mental-model)).

#### Dangerzone (local, security-first) — the most important prior art
Maintained by Freedom of the Press Foundation. Architecture:

1. **Inside the sandbox:** convert the untrusted document to PDF (LibreOffice or PyMuPDF), split into pages, render each page to **raw RGB pixel data**.
2. **Outside the sandbox:** take only the pixel arrays and build a fresh PDF. Optional OCR adds a searchable text layer. Output is `*-safe.pdf`; the original is archived.
3. **Isolation:** gVisor sandbox inside a Linux container (Podman on Windows/macOS, native on Linux), **networking disabled**, no host filesystem access. gVisor is written in Go and reinterprets syscalls rather than passing them to the host kernel.
4. Handles 19+ types: Office (docx/xlsx/pptx), ODF, PDF, images (JPG/PNG/GIF/SVG/TIFF), EPUB.
5. The project is explicit about residual risk: a chain of exploits across LibreOffice → gVisor → kernel (→ Podman's VM on macOS/Windows) is theoretically possible.

**Lesson:** the "narrow, non-executable bottleneck between untrusted parse and trusted rebuild" is the single best security primitive in this domain. We generalize it beyond pixels: *every* conversion in our system passes through a canonical IR that cannot carry executable content ([§6.2](#62-canonical-intermediate-representations-ir-hubs)).

#### Local AI point tools
- **Upscayl** — Real-ESRGAN family via NCNN/Vulkan, cross-platform desktop, model picker (UltraSharp/Remacri/etc.), no account, no watermark. Proof that "download an app, pick a model, drag a file" is a viable consumer AI UX.
- **chaiNNer** — node-based image processing, framework-agnostic across PyTorch/NCNN/ONNX, supports user-trained models. Proof that a node graph is usable by non-programmers for media work.
- **Video2X** — GPL, 100% local, pluggable backends (Real-ESRGAN, Anime4K, RIFE, libplacebo), ~20k GitHub stars ⚠️. Proof of demand for local video enhancement.
- **Buzz / whisper.cpp front-ends** — local transcription as a standalone product.

**Lesson:** each of these is a *module* in our design. The fact that they each exist as separate apps, each with its own model downloader and its own GPU setup, is the redundancy we eliminate.

### 1.4 What nobody currently does

| Missing capability | Why it matters |
|---|---|
| Per-engine sandboxing in a consumer converter | The engines are the attack surface; everyone runs them unconfined |
| Conversion receipts / fidelity accounting | Users cannot tell a lossless remux from a quality-destroying re-encode |
| Lossless-first path planning | Most tools decode+re-encode when a container remux would do |
| One shared model manager across AI capabilities | Users currently keep 4 copies of similar models across 4 apps |
| A real third-party module ABI with permissions | Extensibility today means "fork the repo" |
| Enterprise policy layer on a *local* tool | Orgs want the privacy story but need locked-down config + audit |
| Agent/API access (MCP, JSON CLI) | Conversion is a natural tool call; nobody exposes it well |

---

## 2. Threat landscape (why "safe" is a feature, not a slogan)

### 2.1 The core problem

A converter's job description *is* the attack: **take a file from an untrusted source and run a complex, memory-unsafe parser over it.** Format parsers are the highest-density source of memory-safety bugs in the open-source ecosystem. Moving conversion from a cloud service to the user's laptop *transfers the blast radius to the user*. So "local" without "sandboxed" is a downgrade in safety, not an upgrade.

This is the single most important framing in this document. Our privacy pitch obligates us to a security architecture.

### 2.2 Evidence base (a sample, not a survey)

**ImageMagick** — the canonical example of "conversion library as RCE vector":

| CVE | Nature | Note |
|---|---|---|
| CVE-2025-57803 | 32-bit integer overflow in the BMP encoder (`WriteBMPImage`) scanline-stride computation → heap corruption | CVSS 9.8 ⚠️; explicitly called out as dangerous in **auto-convert pipelines** — i.e. exactly what we are building |
| CVE-2025-55298 | Format-string flaw: `InterpretImageFilename` passes user-controlled input to `FormatLocaleString` | **Crafted filenames** are an input vector, not just file contents |
| CVE-2025-68618 | Uncontrolled recursion in the SVG parser via deeply nested elements → DoS | Any service ingesting user SVG |
| CVE-2025-54418 | Command injection via an ImageMagick handler passing filenames/params into a shell | The *integration* is the bug, not the library |

**Ghostscript** — CVE-2024-29510: format-string injection in the `uniprint` device that **escapes the `-dSAFER` sandbox**, enabling command execution and file I/O. Exploited in the wild using **EPS files disguised as JPGs**. It shipped alongside CVE-2024-29506/29507/29509 (buffer overflows), CVE-2024-29508 (pointer leak), CVE-2024-29511 (arbitrary file read/write). Ghostscript sits underneath ImageMagick, LibreOffice and GIMP, so "I don't use Ghostscript" is usually false.

**FFmpeg** — CVE-2025-1373 (use-after-free in the MOV parser, `mov_read_trak`), plus 2025 issues in the ALS decoder, the Firequalizer filter, the HLS implementation, and a heap overflow in the JPEG 2000 decoder. There is **no official Rust rewrite** of FFmpeg; third-party reconstructions exist but are not production substitutes. Note also: Rust *bindings* to FFmpeg do not confer memory safety — safe-looking Rust can still trigger UAF in the C below.

### 2.3 Attack-class taxonomy — **moved**

> **This section now lives in [09-THREAT-MODEL §3](09-THREAT-MODEL.md#3-attack-classes).**
>
> The taxonomy there is the current one and carries a twelfth class that this version lacked — **A12, path traversal and arbitrary file write** — which is the class the v0.3 architecture was actually exploitable to. Its absence here is instructive: the model was written entirely around *inbound* confinement, because the mental picture was one file in, one file out. Archives are where the output namespace becomes attacker-controlled, and nothing covered it.
>
> Each class there names the control, the requirement ID, and the test that gates it.

### 2.4 The module-ecosystem risk (learn from others' pain)

We are proposing an extension ecosystem. The prior art is discouraging and instructive:

- **VS Code / Open VSX:** malicious extension detections went from 27 (2024) to 105 in the first 10 months of 2025 ⚠️. A fake Prettier extension delivered a multi-stage loader + RAT in Nov 2025. The GlassWorm campaign hit **72+ Open VSX extensions** (linters, formatters, AI assistants) discovered since Jan 2026. "Verified publisher" badges often verify only *domain ownership*, and valid SLSA provenance still can't save you if the build pipeline itself is compromised.
- **Obsidian:** community plugins get an initial review, but the team can't review every release; plugins run with **full application privileges** — filesystem, network, subprocess. The community has repeatedly asked for a browser-extension-style permission manifest and a tiered vetting model.

**Design consequence:** a plugin system without an enforced capability boundary is a malware distribution channel with our signature on it. Permissions must be *enforced by the runtime*, not *declared in a README*. This is why modules default to WASM ([§6.4](#64-the-module-system)).

### 2.5 Security requirements — **moved**

> **This section now lives in [09-THREAT-MODEL §5](09-THREAT-MODEL.md#5-requirements-to-tests).**
>
> The version there adds **SR-13** (an engine or archive member cannot cause a write outside the job directory) and **SR-14** (a killed, crashed or cancelled job leaves no orphan and no partial output), and — the change that matters most — **every requirement now names the test that gates it and the week that test starts failing the build.**
>
> A requirement with no test is a requirement that gets dropped in the next simplification. That is not hypothetical: SR-5's universal resource limits were specified here, had no test, and silently vanished from the architecture between v0.2 and v0.3. Traceability is the fix.

---

## 3. Engines, formats & fidelity

### 3.1 Engine inventory

Risk = likelihood of hostile-input compromise. Tier = minimum isolation tier ([§6.3](#63-isolation-tiers)).

| Engine | Domain | License | Risk | Tier | Notes |
|---|---|---|---|---|---|
| **FFmpeg / libav*** | audio+video, everything | LGPL-2.1+ (GPL if `--enable-gpl`) | High | T2 | The backbone. Build LGPL-only. See §3.4. |
| **libvips** | images, huge/streaming | LGPL-2.1+ | Med | T2 | Faster & far lower memory than IM; prefer as default image engine |
| **ImageMagick / GraphicsMagick** | image breadth | ImageMagick License (Apache-ish) | **Very high** | T2/T3 | Only for formats vips can't do. Delegates & risky coders **disabled**. |
| **libheif + libde265/x265** | HEIC/HEIF/AVIF | LGPL/GPL mix | Med | T2 | HEVC patent exposure |
| **libavif / libaom / SVT-AV1 / dav1d** | AVIF/AV1 | BSD/ISC-ish | Med | T2 | **Royalty-free stack — preferred default** |
| **libjxl** | JPEG XL | BSD-3 | Med | T2 | Lossless JPEG recompression (~20%) is a killer feature |
| **resvg** | SVG render | MPL-2.0, **pure Rust** | Low | T1 | No scripting, no external refs. Use instead of IM for SVG. |
| **Little-CMS (lcms2)** | ICC color | MIT | Low | T1 | Mandatory for color-correct conversion |
| **pdfium** | PDF render/parse | BSD-3 (Google/Chromium) | Med | T2 | Battle-tested via Chrome; prefer over Ghostscript |
| **MuPDF** | PDF | AGPL / commercial | Med | T2 | License makes it awkward for closed distribution |
| **Ghostscript** | PostScript/EPS/PDF ops | AGPL / commercial | **Very high** | **T3** | History of `-dSAFER` escapes; isolate hardest, consider omitting |
| **LibreOffice (headless)** | Office ↔ everything | MPL-2.0 | High | T2/T3 | Huge; ~400 MB. Optional module, not core. |
| **Pandoc** | markup documents | GPL-2+ | Low–Med | T1/T2 | GPL — ship as a separate process, never linked |
| **Calibre / ebook-convert** | ebooks | GPL-3 | Med | T2 | Same GPL boundary rule |
| **Assimp** | 3D assets | BSD-3 | Med | T2 | 77 in / 23 out |
| **Potrace / VTracer** | raster → vector | GPL-2 / MIT | Low | T1 | VTracer (Rust, MIT) preferred |
| **libarchive / 7-Zip** | archives | BSD-2 / LGPL | High | T2 | Bomb-prone; strict limits mandatory |
| **Apache Arrow / DuckDB** | tabular data | Apache-2.0 / MIT | Low | T0 | CSV/Parquet/JSON/Excel done right |
| **Symphonia** | audio demux/decode | MPL-2.0, **pure Rust** | Low | **T0** | AAC, ALAC, FLAC, MP3, MP4, OGG, Vorbis, WAV, WebM — use for *probing* everything |
| **image-rs** | common images | MIT/Apache-2.0, **pure Rust** | Low | **T0** | Safe fast path for PNG/JPEG/GIF/WebP/BMP/TIFF |
| **Tesseract** | OCR (classic) | Apache-2.0 | Med | T2 | Superseded by neural OCR for quality, kept for breadth/languages |
| **infer / tree_magic_mini** | type detection | MIT | Low | T0 | Content sniffing, with `tree_magic_mini` as fallback |

**Architectural rule:** the fast, common paths (PNG↔JPEG↔WebP, WAV↔FLAC, CSV↔Parquet, MD↔HTML) should run through **pure-Rust T0 code** with no subprocess at all. Subprocess + sandbox cost is then only paid for the long tail. This is both a performance and a security win.

### 3.2 Format families and the genuinely hard ones

| Family | Easy | Hard | Why hard |
|---|---|---|---|
| Raster images | PNG/JPEG/WebP/GIF/BMP/TIFF | **Camera RAW** (CR3, NEF, ARW, RAF…) | Per-vendor, per-body; needs libraw + color science + demosaic choices |
| | AVIF, HEIC | HEIC ↔ anything on Windows | Patent + platform codec availability |
| | JPEG XL | JXL adoption | Chrome 145 re-added decode behind a flag in early 2026; off by default in Chrome/Firefox; Safari default-on; ~14% global support ⚠️. **AVIF is the safe web default (~93% ⚠️); JXL's real value is archival + lossless JPEG recompression.** |
| Vector | SVG | AI, CDR, EMF/WMF | Proprietary/undocumented; SVG itself is an active-content hazard |
| Documents | MD, HTML, DOCX, ODT, TXT | **DOCX → PDF fidelity**, PPTX animations, legacy .doc/.wpd | Layout engines disagree; fonts must be embedded or substituted |
| PDF | render, merge, split | **PDF → editable DOCX**, tagged/PDF-UA output, true redaction | Reconstructing semantics from a print format is inference, not conversion |
| Ebooks | EPUB, MOBI/AZW3 | **DRM-protected** | Legally off-limits. Explicit non-goal; state it in the docs. |
| Audio | WAV/FLAC/MP3/AAC/Opus | Multichannel layouts, DSD, ReplayGain/loudness, chapter/tag survival | Metadata models differ per container |
| Video | MP4/MKV/WebM remux | **HDR→SDR**, subtitles/fonts, VFR, interlacing, multi-track | See §3.3 |
| Archives | ZIP, TAR, 7z, RAR (extract) | Encrypted, solid, split, RAR *creation* | RAR compression is proprietary |
| 3D | glTF, OBJ, STL, FBX | Materials/PBR, rigging, units/axes | Assimp loses material fidelity across some pairs |
| Data | CSV, JSON, Parquet, XLSX | Type inference, encodings, merged cells, formulas | "CSV" is not a format, it's a rumor |
| Email | EML, MBOX | PST, MSG | Proprietary containers; attachment recursion |
| Fonts | TTF/OTF/WOFF2 | Hinting, subsetting, licensing | Font EULAs often forbid conversion |
| Subtitles | SRT, VTT, ASS | ASS styling → SRT | Lossy by nature; must warn |
| Notebooks/code | IPYNB, MD | outputs, widgets | Executable content risk |

### 3.3 Fidelity: the quiet failure mode

Most converters produce output that *opens*, which users read as *correct*. Common silent damage:

1. **Color.** Dropping the ICC profile, or converting sRGB→Display P3 without transform, shifts every color. Fix: always carry an explicit color state (ICC profile *or* CICP primaries/transfer/matrix) through the IR, and transform through lcms2 rather than reinterpreting.
2. **HDR → SDR.** `libswscale` converts pixel *formats* but does not tone-map; the result is washed-out or clipped color. Different players tone-map differently anyway (GPU shaders vs. ICC vs. OS compositor), so "correct" is target-dependent. Fix: an explicit tone-mapping operator (BT.2390 / Reinhard / Hable) chosen by preset, plus gain-map support for photos, and never a silent conversion.
3. **Chroma subsampling & bit depth.** 4:4:4 10-bit → 4:2:0 8-bit is common and irreversible; must be disclosed.
4. **Re-encode when a remux would do.** Changing MKV→MP4 with compatible codecs should be a **stream copy**: bit-identical, seconds not minutes. Most tools re-encode. Our planner does not.
5. **Metadata.** Tags, chapters, cover art, timecode, GPS, and color metadata get dropped by naive pipelines — sometimes a privacy *win*, often a data loss. Must be a policy, not an accident.
6. **Fonts & text.** DOCX→PDF without embedded fonts silently substitutes and reflows.
7. **Generation loss.** Repeated lossy round-trips. The app should detect "you are re-encoding an already-lossy source" and say so.

**Design consequence:** fidelity is a first-class output of a conversion, reported in the receipt, not a hope.

### 3.4 The licensing & patent minefield

This constrains packaging on day one.

| Issue | Fact | Consequence for us |
|---|---|---|
| FFmpeg base license | LGPL-2.1+ | Closed-source use is possible **if** you dynamically link and satisfy LGPL |
| FFmpeg GPL components | Building with `--enable-gpl` (e.g. **libx264**, some filters) makes **all of FFmpeg GPL** | Ship an **LGPL-only** default build. x264/x265 go in an optional, clearly-labeled GPL module the user opts into. |
| FFmpeg `--enable-nonfree` | Produces an undistributable binary | Never. |
| LGPL compliance checklist | Dynamic linking (DLL/dylib/so), ship the (modified) source on the same server, display "This software uses code of FFmpeg licensed under the LGPLv2.1" | Build a compliance page + source mirror into the release process from day one |
| Codec patents | H.264/HEVC/AAC are patent-encumbered independent of FFmpeg's license; FFmpeg's own legal page notes companies have faced MPEG-LA demands | Default encoders: **AV1 + Opus + FLAC + AVIF (royalty-free)**. H.264/HEVC/AAC encoding via OS/hardware encoders (Media Foundation, VideoToolbox, VA-API/NVENC) where the platform already licenses them, or as an explicit user-provided/optional module. Decode-only is the lower-risk posture. |
| GPL tools (Pandoc, Calibre, Potrace, Video2X models/code) | GPL | Only ever invoked as **separate processes over a documented interface**, never linked. Keep an explicit legal note on the aggregation argument. |
| AGPL tools (Ghostscript, MuPDF) | AGPL or commercial | Avoid in closed distribution. Prefer pdfium (BSD). If needed, ship as a user-installed optional module. |
| Model licenses | e.g. BRIA **RMBG-2.0 is CC BY-NC 4.0** — non-commercial only; commercial requires an agreement with BRIA | Every bundled model needs a license column and a commercial-use gate. See §4.4. |
| Font/DRM | Font EULAs may forbid format conversion; ebook/video DRM circumvention is illegal in many jurisdictions | Explicit non-goals, enforced in code (refuse DRM-protected inputs with a clear message) |

---

## 4. Local AI landscape

### 4.1 Runtimes

| Runtime | Strength | Where we use it |
|---|---|---|
| **ONNX Runtime** | One model file, many execution providers: CUDA, ROCm, **DirectML**, **CoreML**, **OpenVINO** (CPU/GPU/**NPU**), **QNN** (Snapdragon NPU), XNNPACK, WebGPU | **Primary vision/audio runtime.** Note: DirectML is in sustained engineering; new Windows work is moving to **Windows ML**. |
| **Windows ML** (Win 11 / Copilot+) | OS-managed EPs for NPU/GPU/CPU; app doesn't ship the whole stack | Windows NPU path; reduces installer size |
| **NCNN (Vulkan)** | What Upscayl/Video2X use; works on almost any GPU incl. old Intel/AMD | Fallback GPU path for upscalers where ONNX EP coverage is poor |
| **CoreML / MLX** | Apple Neural Engine + unified memory | macOS path. whisper.cpp's CoreML encoder is reported >3× faster than CPU-only ⚠️ |
| **llama.cpp / GGUF** | Quantized LLM/VLM inference everywhere, Metal/CUDA/Vulkan | The Intent Engine + document/VLM tools |
| **Apple Foundation Models framework** | As of WWDC 2026, a Swift protocol layer that fronts Apple's on-device model *and* third-party/local models; on-device inference is free per request ⚠️ | Optional macOS-native path for text tasks; avoids shipping our own LLM on Mac |
| **whisper.cpp / faster-whisper (CTranslate2)** | ASR. Rough split: whisper.cpp wins on Apple Silicon (Metal + ANE), faster-whisper wins on NVIDIA ⚠️ | Ship both behind one ASR interface, pick by hardware |

**Design consequence:** we need a **hardware abstraction layer** that probes the machine once, ranks available backends, and per-model picks (runtime × execution provider × precision). Users see "Fast / Balanced / Best" and an override; not "which execution provider do you want."

### 4.2 Hardware reality (what to actually promise)

| Tier | Typical machine | What must work |
|---|---|---|
| **T-Min** | Any 4-core CPU, 8 GB RAM, no GPU | All deterministic conversion. Small AI: Whisper tiny/base, Kokoro TTS (runs on a Raspberry Pi ⚠️), background removal at seconds-per-image, classic OCR, 2× upscale (slow) |
| **T-Mid** | Integrated GPU / NPU (Copilot+, Apple M-series, Intel Core Ultra) | Real-time-ish ASR, fast matting, neural OCR, 4× image upscale, small VLM (Qwen3-VL-4B ≈ 3–8 GB ⚠️) |
| **T-High** | Discrete GPU ≥ 8 GB VRAM | Video upscaling, frame interpolation, stem separation (HTDemucs on CPU is 10–15 min/track ⚠️), diffusion-based restoration, 8B+ VLMs |

Publish these tiers in-app: every AI tool shows an estimated runtime for *this machine* before you run it, and refuses-with-explanation rather than freezing a laptop for 40 minutes.

### 4.3 Model inventory by capability

| Capability | Candidate models | License posture | Notes |
|---|---|---|---|
| Image upscale (general) | **Real-ESRGAN**, ESRGAN variants, **SwinIR**, **HAT**, **BSRGAN** | Mostly BSD/Apache/MIT — **verify each checkpoint** | Real-ESRGAN = practical default; SwinIR/HAT = quality-at-cost; BSRGAN better on noisy/compressed sources ⚠️ |
| Image restore (severe) | **SUPIR**, diffusion upscalers | Often restrictive; check base-model license | *Generative* — Class D, opt-in only |
| Face restore | GFPGAN / CodeFormer family | Mixed; CodeFormer has non-commercial terms — **verify** | Class D |
| Background removal / matting | **BiRefNet** (general / portrait / matting variants) via rembg; U²-Net | BiRefNet widely used in open tooling; **BRIA RMBG-2.0 is CC BY-NC 4.0 — not usable commercially without a BRIA agreement** | ~17 FPS @1024² FP16 on RTX 4090, ~3.5 GB VRAM; several seconds/image on CPU ⚠️ |
| OCR / doc → structure | **Marker** (Surya OCR), **Docling** (IBM, MIT, CPU-friendly), **MinerU** (84 languages, strong CJK) | Docling MIT; verify Marker/MinerU model licenses separately from code licenses | Reported olmOCR-bench: Marker 2 ≈76.0, MinerU pipeline ≈72.7, Docling ≈50.3 ⚠️. Ship **Docling as the default** (MIT + CPU), Marker/MinerU as optional modules. |
| Speech → text | **Whisper large-v3 / turbo**, distil-whisper, WhisperX (word timestamps + diarization) | MIT (OpenAI Whisper weights) | "turbo" reported as the accuracy/compute sweet spot; no Whisper v4 as of mid-2026 ⚠️ |
| Text → speech | **Kokoro-82M** (Apache-2.0, 54 voices/8 langs, faster than real-time, CPU-capable), **Piper** (ONNX, 900+ voices, 47 langs), XTTS/Chatterbox | **Kokoro's Apache-2.0 is the standout** for commercial safety ⚠️ | Voice cloning is a policy minefield — see §5.7 |
| Audio stem separation | **HTDemucs / htdemucs_ft** (Meta), Open-Unmix | MIT (Demucs) | GPU strongly recommended |
| Video frame interpolation | **RIFE** (IFNet) | Check checkpoint license; RIFE code MIT-ish, some weights restricted | Class D |
| Video upscale | Real-ESRGAN, Anime4K, libplacebo, SeedVR2/FlashVSR | Mixed | Very expensive; T-High only |
| Vision-language (doc Q&A, alt text, classification) | **Qwen3-VL 4B/8B** (strong DocVQA ⚠️), **Moondream 3** (2B active MoE, purpose-built for detection/grounding/structured output) | Apache-2.0 for Qwen3 line — **verify per checkpoint** | Qwen3-VL for documents/tables/multilingual; Moondream when you need coordinates back |
| Text LLM (intent engine, summarize, rename) | Small instruct models via GGUF (3–8B) | Apache/MIT preferred | Must run on T-Min in a degraded mode or be skippable |

### 4.4 Model licensing is the trap

The RMBG-2.0 case is the cautionary tale: excellent quality, open weights, **CC BY-NC 4.0**, commercial use requires a paid agreement. Shipping it in a paid app is an infringement.

**Required control:** the model registry entry is not just a URL + hash. It carries:

```toml
[model.birefnet-general]
license      = "MIT"              # SPDX where possible
commercial   = true               # gate
redistribute = true               # may we mirror the weights?
attribution  = "required"         # surfaced in About > Acknowledgements
source       = "https://..."
sha256       = "..."
format       = "onnx"             # onnx | gguf | safetensors — never pickle
size_mb      = 178
```

The build fails if a model with `commercial = false` is referenced from a paid tier. Non-commercial models may still be *offered* to users of a free/personal edition via user-initiated download, with the license shown before download — but they are never bundled.

### 4.5 Model supply-chain security

- **Never load pickle-derived formats.** Pickle deserialization executes arbitrary code; `picklescan`-style scanners have had their own zero-day bypasses. safetensors was independently security-audited and is tensor-data-only. Our loader accepts **safetensors, GGUF, ONNX** and rejects everything else at the file-type layer, before any library touches it.
- **Malicious model uploads to public hubs are rising** (reported ~5× YoY ⚠️). Therefore: models come from **our signed index**, not from arbitrary URLs, by default. Advanced users may add a custom source, which flips a persistent "unverified models enabled" indicator.
- **ONNX graphs can reference custom operators** that load external native libraries. Validate the op set against an allowlist; refuse graphs with custom domains.
- Hash-pin everything. Verify on every load, not just on download (defends against post-install tampering).

### 4.6 Where AI must *not* go

Stated here because it belongs to the research conclusions: **AI is a poor substitute for a codec.** No model should be in the path of a JPEG→PNG conversion. AI belongs in (a) understanding content, (b) enhancing content when the user asks, (c) choosing parameters, (d) driving the UI. Never in silently re-rendering the user's data. Formalized in §5.9.

---

## 5. Product design

### 5.1 Product principles

1. **Local by default, offline-capable always.** The app must be fully functional with the NIC disabled.
2. **Never surprise the file.** No silent re-encode, no silent metadata strip, no silent AI touch-up.
3. **The original is sacred.** Non-destructive by default; originals are never overwritten without an explicit, separate confirmation.
4. **Safety is a mode you can turn *up*, never *off* below a floor.** Sandboxing is not optional; Paranoid is an addition.
5. **Small core, everything else is a module.** Installer target < 60 MB. Users pull the rest.
6. **Everything the GUI does, the CLI does, and the config file describes.** No GUI-only capability.
7. **Explain the cost before doing the work.** Estimated time, output size, quality delta, and whether it's lossless.
8. **AI is labeled, opt-in, and reversible.** Generative operations are marked in the receipt and in the file's provenance.
9. **No accounts, no telemetry, no phone-home.** The product is the binary.
10. **Boring where it matters.** Deterministic conversions must be byte-reproducible given the same engine versions and parameters.

### 5.2 Personas & jobs

| Persona | Job | What they need that others don't give |
|---|---|---|
| **Privacy-constrained professional** (lawyer, doctor, journalist, HR) | "Convert this contract/scan without it leaving my laptop" | Verifiable locality + Paranoid mode + redaction + audit log |
| **Creator** (video/podcast/photo) | "Get this into the format the platform wants, at the smallest size that still looks right" | Platform presets, per-title quality targeting, stems, subtitles, batch |
| **Developer / power user** | "Script it, pipe it, put it in CI, let my agent call it" | CLI, `--json`, exit codes, watch folders, MCP server, reproducible engine pinning |
| **Knowledge worker** | "PDF → editable, deck → doc, 200 scans → searchable" | Doc IR, OCR, table extraction, batch rename/route |
| **IT / security admin** | "Let staff convert attachments safely, under policy" | Locked policy layer, offline module/model bundles, audit log, no egress |
| **Archivist / researcher** | "Normalize 40k heterogeneous files, losslessly, with a manifest" | Lossless-first planner, receipts, dry-run plans, resumable batch |

### 5.3 The five surfaces

1. **Drop** — the zero-config surface. Drop files, get a ranked list of suggested targets based on detected type + recent history + presets. One click.
2. **Studio** — the pipeline canvas. Nodes = sources, operations, branches, targets. Save as a Recipe. (chaiNNer's node model, Stirling's saved-pipeline concept, generalized.)
3. **Watch** — folder automation. Rules: `if PDF and pages>1 and has_no_text_layer → OCR → move to /Archive`. Runs as a background service (opt-in).
4. **Command line + API** — `openconvert` binary, `--json` everywhere, plus a local **MCP server** so an agent can convert files as a tool call, and an optional loopback HTTP API for scripts.
5. **OS integration** — right-click / Services menu / Quick Actions, Share sheet, and a global command palette with fuzzy search over every tool and recipe.

### 5.4 The core mental model

```
        ┌──────────┐    ┌────────────┐    ┌──────────┐    ┌────────┐
 file → │  DETECT  │ →  │  INGEST    │ →  │   IR     │ →  │ EMIT   │ → output
        │ (sniff)  │    │ (parse in  │    │ (canon.  │    │(encode)│    + receipt
        └──────────┘    │  sandbox)  │    │ hub fmt) │    └────────┘
                        └────────────┘    └────┬─────┘
                                               │
                                          ┌────▼─────┐
                                          │  TOOLS   │  ← deterministic ops
                                          │ (magic + │    and AI ops operate
                                          │ classic) │    ON THE IR, not on
                                          └──────────┘    file formats
```

Three user-facing nouns:

- **Job** — one input set → one output set, with a plan.
- **Recipe** — a saved, parameterized Job graph. A file (`*.recipe.toml`), shareable, versionable, diffable, signed if published.
- **Profile** — a bundle of defaults: quality targets, metadata policy, naming scheme, safety level, output locations. (`Personal`, `Work-Confidential`, `Web-Publishing`, `Archive-Master`.)

The killer interaction: **every job shows its plan before running.**

```
Plan for 42 files → MP4                                       [Dry run ✓]

  1. Detect                                   42 files, 3 distinct types
  2. 31 × MKV(h264,aac) → MP4     STREAM COPY  lossless · ~8s   · 0 re-encode
  3.  9 × AVI(mpeg4,mp3) → MP4    RE-ENCODE    lossy    · ~4m12s · AV1 CRF 30
  4.  2 × MOV(prores,pcm) → MP4   RE-ENCODE    lossy    · ~2m40s · AV1 CRF 26
     ⚠ 2 files carry HDR10 metadata → tone-map BT.2390 (SDR output)
     ⚠ 9 files already lossy → generation loss expected
     ⓘ Metadata policy "Work": GPS + author removed, chapters kept

  Estimated: 7m10s · 1.4 GB in → ~610 MB out · 0 network calls
                                                     [Adjust]  [Run]
```

No other converter shows this. It is cheap to build and it *is* the trust story.

### 5.5 The Magic Tools catalog

Grouped by domain. Class column: **A** = deterministic/lossless, **B** = deterministic/lossy, **C** = inferred (analysis, reviewable), **D** = generative (alters pixels/audio). See §5.9.

#### Images
| Tool | Class | Engine/model |
|---|---|---|
| Format convert / re-encode | A/B | vips, image-rs, libjxl, libavif |
| Lossless JPEG recompression (≈20% smaller, bit-exact restore) | **A** | libjxl |
| Smart compress to target size / target quality (SSIM-guided search) | B | vips + metric loop |
| Resize / crop / rotate / flatten / ICC transform | B | vips + lcms2 |
| **Upscale** 2×/4×/8× | D | Real-ESRGAN / SwinIR / HAT |
| **Restore** (denoise, deblur, JPEG-artifact cleanup, descreen) | D | Real-ESRGAN variants, BSRGAN |
| **Face restore** | D | CodeFormer/GFPGAN (license-gated) |
| **Background remove / replace** | D | BiRefNet (general/portrait/matting) |
| **Object removal / inpaint** | D | LaMa-class |
| **Smart crop / reframe** to aspect | C→B | saliency + face detection, then deterministic crop |
| **Auto color / exposure / white balance** | D | classic + learned |
| **Colorize** B&W | D | opt-in, clearly generative |
| **Deskew / dewarp** scans | B/D | classic CV + learned dewarp |
| **Vectorize** raster → SVG | B | VTracer / Potrace, AI-assisted parameter choice |
| **Alt-text / caption generation** | C | Qwen3-VL / Moondream |
| **Auto-tag & near-duplicate detection** across a folder | C | CLIP-class embeddings + pHash |
| **Local PII/NSFW flagging** (faces, IDs, plates) before sharing | C | on-device detector; flags only, never auto-deletes |
| Metadata: strip / preserve / edit / normalize | A | exiv2-class |

#### Documents
| Tool | Class | Engine/model |
|---|---|---|
| PDF ops: merge/split/rotate/reorder/extract/stamp/watermark/compress | A/B | pdfium + qpdf |
| Office ↔ PDF ↔ ODF ↔ MD ↔ HTML ↔ TeX | B | LibreOffice, Pandoc |
| **OCR with layout** (searchable text layer) | C | Docling / Marker / Surya / Tesseract |
| **PDF → structured Markdown / JSON** (headings, lists, tables, formulas) | C | Docling (default), Marker, MinerU |
| **Table extraction** → CSV/XLSX/Parquet | C | doc models + Arrow |
| **Formula → LaTeX**, handwriting recognition | C | specialist models |
| **True redaction** (remove objects, not draw boxes) + PII auto-detect | C→A | detector proposes, user confirms, qpdf removes |
| **Summarize / translate** document | C | local LLM/VLM |
| **Classify + auto-rename + route** ("Invoice_ACME_2026-03-14.pdf") | C | VLM field extraction → naming template |
| **Reflow scanned book → EPUB** | C | doc IR + segmentation |
| **Accessibility**: tag structure, alt text for embedded images (PDF/UA) | C | doc IR + VLM |
| **Paranoid rebuild** (pixel bottleneck + OCR text layer) | A* | Dangerzone technique |

#### Audio
| Tool | Class |
|---|---|
| Format convert, lossless transcode (FLAC↔WAV↔ALAC), tag/chapter preservation | A |
| Lossy encode with Opus/AAC, bitrate/VBR targeting | B |
| **Loudness normalize** (EBU R128 / -14 LUFS etc.) | B |
| **Transcribe** → SRT/VTT/JSON with word timestamps | C |
| **Speaker diarization** ("who spoke when") | C |
| **Translate subtitles** | C |
| **Stem separation** (vocals/drums/bass/other) | D |
| **Denoise / dereverb / de-hum** | D |
| **Silence trim / auto-chapters from transcript** | C→A |
| **Text → speech narration** (Kokoro/Piper) | D |
| BPM / key / language detection | C |

#### Video
| Tool | Class |
|---|---|
| Remux / stream copy (container change with **zero** re-encode) | **A** |
| Transcode with codec/bitrate/CRF control, hardware encode | B |
| **Per-title quality targeting** (encode to a VMAF target, not a bitrate) | B+C |
| **HDR → SDR** with an explicit tone-mapping operator | B |
| **Subtitle generate + burn-in or soft-embed** | C→B |
| **Frame interpolation** 24→60 fps | D (RIFE) |
| **Upscale** | D |
| **Stabilize / denoise** | D |
| **Scene detect + auto-split**, silence/filler removal | C→A |
| **Auto-reframe** to 9:16 / 1:1 with subject tracking | C→B |
| Thumbnail / contact sheet / animated preview, GIF/WebP optimizer | B |
| Trim/concat losslessly at keyframes | **A** |

#### Data, 3D, archives, misc
| Tool | Class |
|---|---|
| CSV/TSV/JSON/YAML/TOML/XML/Parquet/XLSX interconversion with **schema inference + type report** | A/B |
| Encoding & delimiter detection and repair (the actual CSV problem) | C→A |
| Spreadsheet → clean tabular (unmerge, header detect, de-pivot) | C |
| 3D: glTF/OBJ/STL/FBX/PLY convert, decimate, unit/axis normalize | B |
| Archives: repack, recompress (zip→zstd/7z), inspect, safely extract with limits | A |
| Email: MBOX/EML/MSG → PDF/MD with attachments extracted | B |
| Fonts: TTF/OTF/WOFF2, subset (with an EULA warning) | B |
| Images ↔ video ↔ GIF sequences, contact sheets, sprite sheets | B |
| **Batch rename** from AI-extracted fields + template DSL | C |
| **"Make it fit"** — target size / dimensions / platform preset, solver picks parameters | B |
| **"Match this reference"** — analyze a reference file, replicate its encoding parameters | C→B |

#### Platform presets (ship ~40)
Web (AVIF ≤200 KB, WebP fallback, responsive srcset), YouTube 4K/1080p, Instagram Reel/Post/Story, TikTok, X, Discord (≤10 MB), WhatsApp, email (≤10/25 MB), Kindle, Kobo, ePub3, print CMYK 300 dpi, arXiv-safe PDF, PDF/A-2b archival, ProRes proxy, DaVinci-friendly, Plex/Jellyfin-optimal, iOS/Android device-safe, PowerPoint-safe images, Slack, Notion, WordPress.

### 5.6 The Intent Engine (natural language → plan)

A small local LLM (3–8B GGUF, or the OS-provided model on macOS/Windows) that turns a phrase into a **plan object**, never into shell commands.

```
> "make these 40 podcast episodes into transcripts with speaker names,
   plus a 1-minute teaser clip of the loudest bit"

Proposed plan (nothing has run yet):
  A. 40 × MP3 → ASR(whisper-turbo) → diarize(pyannote-class) → *.srt + *.md
  B. 40 × loudness analysis → pick peak 60s window → trim (lossless) → *_teaser.mp3
  Estimated 22m on your GPU · 0 network calls · originals untouched
                                          [Edit as pipeline] [Run] [Cancel]
```

Rules:
- The LLM emits **only** a constrained JSON plan validated against the tool schema. It cannot invent operations, paths outside the selection, or shell strings.
- The plan is always shown and editable. Nothing runs unreviewed.
- The engine is **optional and removable** — the app is fully usable without it.
- It runs against the tool registry, so installing a module automatically extends what the intent engine can propose (no retraining, just schema).

### 5.7 Policy-sensitive capabilities

Some AI tools carry real misuse potential. Ship them with friction, or not at all:

| Capability | Stance |
|---|---|
| Voice cloning from a sample | **Not in v1.** If ever: local-only, watermarked output, explicit consent attestation. |
| Face swap / identity manipulation | **Not shipped.** |
| Watermark removal | **Not shipped** as a named feature. Generic inpainting exists; we don't market it for this. |
| DRM stripping (ebooks, video) | **Not shipped.** Refuse DRM inputs with a clear message. |
| Upscaling as "enhancement" of evidence/faces | Shipped, but generative outputs are labeled in the receipt and (optionally) C2PA-marked, because invented detail must never be mistaken for recovered detail. |
| PII detection | Flags only; never silently deletes or transmits. |

### 5.8 Conversion Receipts & provenance

Every output gets a sidecar (`file.ext.receipt.json`, optional, on by default for AI ops):

```json
{
  "openconvert_version": "1.4.2",
  "job_id": "01J...",
  "input":  { "name": "IMG_4021.HEIC", "sha256": "9f2c…", "bytes": 3841022,
              "detected_type": "image/heic", "declared_type": "image/heic" },
  "output": { "name": "IMG_4021.avif",  "sha256": "b71a…", "bytes": 412998 },
  "operations": [
    { "op": "decode",  "class": "A", "engine": "libheif 1.20.2" },
    { "op": "color",   "class": "A", "engine": "lcms2 2.16",
      "from": "Display P3", "to": "sRGB", "intent": "relative-colorimetric" },
    { "op": "upscale", "class": "D", "model": "real-esrgan-x4plus",
      "model_sha256": "4c1e…", "runtime": "onnxruntime 1.23 / DirectML",
      "generative": true },
    { "op": "encode",  "class": "B", "engine": "libavif 1.3.0",
      "params": { "quality": 62, "speed": 6, "chroma": "4:2:0", "depth": 8 } }
  ],
  "fidelity": { "ssim": 0.9971, "psnr_db": 44.2, "lossless": false },
  "metadata_policy": "share-safe",
  "metadata_removed": ["GPS", "SerialNumber", "OwnerName"],
  "network_calls": 0,
  "isolation": { "max_tier": "T2", "engines_sandboxed": true }
}
```

Optionally emit a **C2PA** manifest for images/video so generative edits are cryptographically attributable. This is a differentiator for journalism/legal customers and costs little.

### 5.9 The determinism boundary (the single most important design rule)

Every operation is one of four classes, and the class is visible in the UI, the plan, the receipt, and the CLI output:

| Class | Meaning | Reversible? | Default | UI treatment |
|---|---|---|---|---|
| **A — Deterministic, lossless** | Remux, stream copy, lossless transcode, container change, metadata edit | Yes (bit-exact round-trip possible) | On | Green "lossless" chip |
| **B — Deterministic, lossy** | Re-encode, resize, quantize, tone-map | No, but reproducible | On, with disclosure | Amber chip + measured delta |
| **C — Inferred** | OCR, ASR, classification, table extraction, detection | Output is *new data*, source untouched | On | "Reviewable" — always presented for confirmation on write-back |
| **D — Generative** | Upscale, inpaint, colorize, interpolate, restore, TTS | No — invents information | **Off by default** | Red/violet "AI-generated content" chip; receipt + optional C2PA; original always retained |

**Hard rule:** a Class D operation can never be introduced into a job by defaults, presets, or the intent engine without an explicit user action on that job. "Convert" never secretly means "enhance."

---

## 6. Architecture

> ⚠️ **Superseded in full by [03-ARCHITECTURE](03-ARCHITECTURE.md).** Everything in this part was cut: the daemon and its IPC protocol, seven of the eight IR hubs, A\* over a capability graph, two of the four isolation tiers, the module system and its manifest ABI, the content-addressed cache, and six-layer config resolution. Retained as the record of what was considered and why it was rejected — [03-ARCHITECTURE §2](03-ARCHITECTURE.md#2-what-earlier-revisions-cut-and-why-that-still-stands) gives the reason for each cut.

### 6.1 System overview

```
┌───────────────────────────────────────────────────────────────────────────┐
│  SHELLS   Desktop GUI (Tauri)   CLI (openconvert)   MCP server   OS integrations  │
└──────────────────────────────┬────────────────────────────────────────────┘
                               │  IPC (typed, versioned, local socket)
┌──────────────────────────────▼────────────────────────────────────────────┐
│                        CORE DAEMON  (Rust, no unsafe in core)              │
│                                                                            │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌──────────┐ │
│  │  Detector  │ │  Planner   │ │ Scheduler  │ │  Governor  │ │ Receipts │ │
│  │ sniff+     │ │ graph      │ │ DAG exec,  │ │ limits,    │ │ hashes,  │ │
│  │ polyglot   │ │ search on  │ │ resume,    │ │ quotas,    │ │ metrics, │ │
│  │ detection  │ │ cost fn    │ │ cache      │ │ kill       │ │ C2PA     │ │
│  └────────────┘ └────────────┘ └────────────┘ └────────────┘ └──────────┘ │
│  ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌────────────┐ ┌──────────┐ │
│  │  IR hubs   │ │  Registry  │ │  Modules   │ │  Models    │ │  Policy  │ │
│  │ raster/doc │ │ tools,     │ │ install,   │ │ resolve,   │ │ layered  │ │
│  │ audio/av/  │ │ formats,   │ │ verify,    │ │ verify,    │ │ config,  │ │
│  │ tabular/3d │ │ capability │ │ sandbox    │ │ HW select  │ │ org lock │ │
│  └────────────┘ └────────────┘ └────────────┘ └────────────┘ └──────────┘ │
└──────────────────────────────┬────────────────────────────────────────────┘
                               │  Capability-brokered handles only
   ┌───────────────┬───────────┴───────────┬─────────────────┬─────────────┐
   ▼               ▼                       ▼                 ▼             ▼
┌────────┐  ┌──────────────┐   ┌────────────────────┐  ┌──────────┐  ┌─────────┐
│  T0    │  │      T1      │   │        T2          │  │   T3     │  │ Model   │
│ in-proc│  │ WASM         │   │ OS-sandboxed       │  │ microVM /│  │ workers │
│ pure   │  │ (wasmtime,   │   │ subprocess         │  │ container│  │ (ORT,   │
│ Rust   │  │  fuel, no    │   │ (ffmpeg, vips, LO, │  │ gVisor/  │  │ llama,  │
│ safe   │  │  WASI net)   │   │  pdfium, archive)  │  │ WSL2/VZ  │  │ ncnn)   │
└────────┘  └──────────────┘   └────────────────────┘  └──────────┘  └─────────┘
```

**Why a daemon + thin shells:** the GUI, CLI, watch service, and MCP server all need the same scheduler, cache, and sandbox supervisor. Duplicating that is how you get inconsistent behavior between "the app" and "the CLI." One core, four faces.

**Framework choice: Tauri v2** over Electron. Rationale: capability-based security *by default* (Electron is permissive-by-default and must be locked down), dramatically smaller binaries and RAM, native Rust core with no IPC-to-Node hop, and first-class sidecar support for shipping engine binaries. Known caveats to plan for: sidecar `rpath` fixups are manual on some platforms ⚠️, and WebView differences (WebKitGTK / WKWebView / WebView2) mean the UI must be tested on all three. If the GUI later needs a Chromium guarantee, the daemon split means the shell is replaceable without touching the engine.

### 6.2 Canonical Intermediate Representations (IR hubs)

**The problem:** M input formats × N output formats = M×N converters. ConvertX-class tools solve this by having each engine own a matrix. That's why coverage is uneven and why quality varies by path.

**The solution:** route through canonical hubs. M+N adapters instead of M×N. This is also a *security* mechanism: the IR is a plain data structure with no executable content, so it acts as a Dangerzone-style bottleneck on every path, not just Paranoid mode.

| Hub | Representation | Carries |
|---|---|---|
| **RasterIR** | frame sequence of linear-light RGBA (f16/u16), tiled/streamable | explicit color state (ICC blob *or* CICP), alpha mode, orientation, DPI, HDR gain map, metadata map |
| **VectorIR** | normalized path/paint tree (no script, no external refs) | fonts referenced + embedded, units |
| **DocIR** | structured document tree: blocks, headings, lists, tables, figures, footnotes, math, reading order, page geometry, provenance spans | semantics *and* layout; converts to MD/HTML/DOCX/EPUB/PDF/JSON |
| **AudioIR** | planar f32 PCM + sample rate + channel layout | tags, chapters, loudness measurements, cue points |
| **AVGraph** | demuxed elementary streams + edit/filter graph | **codec-preserving**: enables stream copy without ever decoding |
| **TabularIR** | Apache Arrow record batches | schema, inferred types, null policy, source encoding |
| **SceneIR** | glTF 2.0-shaped scene graph | meshes, materials, transforms, units, axis convention |
| **ArchiveIR** | virtual filesystem tree, lazy | per-entry type, size, ratio, depth |

**Critical property of AVGraph:** it represents streams *without decoding them*. `MKV(h264,aac) → MP4` never instantiates RasterIR; it's a copy. This is what makes lossless-first routing possible.

### 6.3 Isolation tiers

| Tier | Mechanism | Startup | Throughput | Used for |
|---|---|---|---|---|
| **T0** | In-process, pure-safe Rust, no `unsafe` in the parse path | ~0 | Native | image-rs, Symphonia (probing/decoding), Arrow, text/markup, hashing, tabular |
| **T1** | **wasmtime**: capability-based WASI, **no network capability granted**, preopened dir = one brokered temp dir, fuel metering for CPU, hard memory cap, epoch-based interruption | ~1–5 ms | 1.2–3× slower | Third-party modules (default), resvg, small codecs, custom scripts |
| **T2** | OS-sandboxed subprocess. **Linux:** user namespace + seccomp-bpf allowlist + **Landlock** path restriction + cgroup v2 memory/CPU/pids. **macOS:** App Sandbox + `sandbox_init` profile, no network entitlement. **Windows:** **AppContainer** + restricted token + **Job Object** (memory/CPU/process caps) + no network firewall rule | ~10–50 ms | Native | FFmpeg, libvips, pdfium, libarchive, ImageMagick, LibreOffice, model workers |
| **T3** | MicroVM/container. **Linux:** gVisor or Firecracker. **macOS:** Virtualization.framework. **Windows:** WSL2/Hyper-V container. No network, no host FS. | 200 ms–2 s | Native-ish | Ghostscript, Paranoid mode, anything the user marks "untrusted source", enterprise "all conversions" policy |

**Assignment rules**
- An engine's tier is declared in its manifest and can only be *raised* by policy, never lowered by a module.
- The user's "safety level" (Standard / Elevated / Paranoid) sets a floor: Paranoid pushes everything ≥ T2 and documents/images to T3.
- **Files with a `mark-of-the-web` / quarantine attribute, or from a `Downloads`/`Mail` path, automatically get +1 tier.** Nice touch: the app knows the file is from the internet because the OS already told it.
- T3 is an *optional module* (it needs a VM/container runtime); if unavailable, Paranoid mode degrades to a hardened T2 and **says so explicitly** rather than silently weakening.

**Capability brokering:** no sandboxed process ever receives a user path. The core creates a per-job temp dir, copies/links inputs in under opaque names, passes pre-opened file descriptors/handles where the OS allows, and copies results out after validation. A compromised engine sees one directory containing the file it was already given.

### 6.4 The Module System

#### What a module can be
| Kind | Contains | Default tier |
|---|---|---|
| **Converter** | Format adapters (`from → to` edges for the planner) | T1 (WASM) or T2 (native sidecar) |
| **Tool** | An operation over an IR (a Magic Tool) | T1 / T2 / model-worker |
| **Model pack** | Weights + metadata + a runtime binding | Model worker (T2) |
| **Preset pack** | Presets, recipes, naming templates | Data only (T0) |
| **Theme** | Design tokens, icons | Data only |
| **Integration** | Watch-folder sources, destinations (S3, WebDAV, SMB) — the only module kind allowed network, and only with an explicit allowlist | T1 + net capability |

#### Manifest (the contract)

```toml
schema = 1
id            = "dev.openconvert.mod.heif"
name          = "HEIF / HEIC"
version       = "2.1.0"
publisher     = "OpenConvert Core"
license       = "LGPL-2.1-or-later"
min_app       = "1.2.0"
description   = "Decode and encode HEIF/HEIC images."

[runtime]
kind          = "native-sidecar"      # wasm-component | native-sidecar | script | model
tier          = "T2"                  # minimum isolation tier; policy may raise
entry         = "bin/${target}/heif-worker${exe}"
targets       = ["x86_64-pc-windows-msvc","aarch64-apple-darwin","x86_64-unknown-linux-gnu"]

[capabilities]                        # ENFORCED, not documentation
fs            = "brokered"            # brokered | none   (never "host")
network       = "none"                # none | allowlist  (allowlist requires `network.hosts`)
subprocess    = false
gpu           = false
env           = []                    # explicit allowlist of env vars
clipboard     = false

[limits]
memory_mb     = 2048
wall_time_s   = 120
cpu_time_s    = 90
output_bytes  = 2_000_000_000
expansion_max = 200                   # output/input ratio guard
pixels_max    = 500_000_000

[[provides.converter]]                # planner edges
from     = ["image/heic","image/heif"]
to       = ["raster-ir"]
fidelity = "lossless-decode"
cost     = 1.0
[[provides.converter]]
from     = ["raster-ir"]
to       = ["image/heic"]
fidelity = "lossy"
cost     = 2.0
notes    = "HEVC patent exposure; disabled in the royalty-free profile"

[integrity]
artifacts = [
  { path = "bin/x86_64-pc-windows-msvc/heif-worker.exe", sha256 = "…" },
]
sbom      = "sbom.spdx.json"
signature = "module.sig"              # signed by publisher key; countersigned by registry
```

#### Lifecycle
`discover → verify signature → verify hashes → check license/policy gate → check capability set against policy → install (content-addressed, versioned dir) → register in planner graph → enable`

- **Atomic + rollback.** Modules install to `modules/<id>/<version>/`; enabling flips a symlink/pointer. Downgrade is instant.
- **Pinning.** A Recipe can pin exact module versions, making conversions reproducible over time (the CloudConvert engine-pinning idea, done better because it's local).
- **Hot reload** for data-only and WASM modules; native sidecars require a worker restart, not an app restart.
- **Kill switch.** A signed revocation list ships with updates. If an engine gets an exploited-in-the-wild CVE, we can disable that module version on every install within one update cycle, with a clear in-app explanation. This is SR-12 made real.

#### Trust tiers in the registry
| Tier | Meaning | Requirements | UI |
|---|---|---|---|
| **Core** | Built and signed by us | In-repo, reviewed, fuzzed in CI | No warning |
| **Verified** | Third party, reviewed by us | Source available, reproducible build, no `network`/`subprocess` capability, signed, SBOM | Blue check + capability list |
| **Community** | Third party, unreviewed | Signed by publisher; **WASM-only (T1)**; no network, no subprocess | Yellow banner, capability list shown at install with a hold-to-confirm |
| **Local** | User-built / sideloaded | Nothing | Persistent "developer mode" indicator |

Deliberate constraint learned from VS Code/Obsidian: **unreviewed modules cannot run native code and cannot touch the network.** If a capability can't be enforced, it isn't offered. That closes the class of attack that has repeatedly hit other ecosystems.

### 6.5 The Planner (the technical heart)

Model the system as a **directed graph**: nodes are format/IR states, edges are converter capabilities contributed by modules. Each edge is annotated:

```
edge = (from, to, module, engine_version,
        fidelity_class ∈ {A,B,C,D},
        est_cost(bytes, pixels, duration),
        risk_tier ∈ {T0..T3},
        license_flags,          # patent-encumbered? GPL? non-commercial model?
        quality_loss_estimate)
```

Path selection is **A\* over a user-selectable cost function**:

```
cost = w_fidelity · Σ quality_loss
     + w_time     · Σ est_time
     + w_risk     · Σ tier_penalty
     + w_size     · predicted_output_bytes
     + ∞ if any edge violates active policy (license gate, capability gate, safety floor)
```

Modes map to weight presets:

| Mode | Behavior |
|---|---|
| **Lossless-first** (default) | `w_fidelity` dominates; refuses to re-encode when a copy path exists; will happily pick a slower path to stay lossless |
| **Fast** | `w_time` dominates; prefers hardware encoders |
| **Smallest** | `w_size` dominates with a quality floor (e.g. "min VMAF 93") |
| **Safest** | `w_risk` dominates; prefers memory-safe T0/T1 engines even at quality/speed cost |
| **Reproducible** | Only pinned engine versions; no hardware encoders (they're non-deterministic across drivers) |

**Why this matters:** it's the difference between "we have 1000 conversions" and "we pick the *right* one." It's also how the app stays coherent as modules are added — a new module contributes edges, and everything downstream (planner, intent engine, CLI help, UI suggestions) updates automatically with no code changes.

### 6.6 Job engine

- **DAG execution** with content-addressed caching: node output keyed by `hash(inputs) + hash(op) + hash(params) + module version`. Re-running a 500-file batch after changing one parameter only recomputes the affected subtree.
- **Resumable batches.** Crash/quit/power loss → resume from the journal. Non-negotiable for 10k-file archival jobs.
- **Resource governor.** Global caps (max concurrent jobs, max total RAM, GPU serialization, thermal/battery awareness — on battery, default to CPU-light paths and warn before a 40-minute GPU job).
- **Priority lanes.** Interactive (user is watching) preempts background (watch folder).
- **Streaming where possible.** libvips-style streaming for large images; FFmpeg pipe-through for video; never load a 4 GB file into RAM to change its container.
- **Idempotence & collision policy.** Explicit: skip / rename / overwrite / version — never guess.

### 6.7 Model Manager (a package manager for models)

```
openconvert model list                       # installed, with size, license, backend
openconvert model search upscale
openconvert model pull real-esrgan-x4plus    # verifies signature + sha256 before writing
openconvert model bench                      # measures THIS machine, records the results
openconvert model gc                         # remove models unused for N days
```

- Shared store: one copy of Whisper serves the audio module, the video subtitle tool, and the intent engine. (Directly addresses the "four apps, four copies" waste in the current ecosystem.)
- Per-model, per-machine **backend selection** is measured, not guessed: on first use, run a 5-second benchmark across available runtimes/EPs and cache the winner.
- Quantization variants offered by hardware tier (fp16 / int8 / Q4_K_M) with quality notes.
- License gate enforced at pull time; non-commercial models show their terms and are blocked in commercial/Team editions.
- Format allowlist: safetensors / GGUF / ONNX only (SR-8).

### 6.8 Storage layout

```
~/.openconvert/
  config.toml                # user config
  policy.toml                # org policy (read-only, may be system-managed)
  profiles/                  # Personal.toml, Work-Confidential.toml, …
  recipes/                   # *.recipe.toml (git-friendly)
  presets/
  modules/<id>/<version>/    # content-addressed, atomic
  models/<id>/<version>/     # shared store
  keys/                      # trust roots, revocation list
  cache/                     # content-addressed intermediate results (purgeable)
  work/<job-id>/             # per-job sandbox temp (auto-wiped, secure-delete option)
  logs/                      # local only; audit.jsonl when policy enables it
  receipts/                  # optional central receipt store
```

Everything is a plain text file except the caches. You can version-control `~/.openconvert` minus `cache/`, `work/`, `models/`. That is what "fully customizable" should actually mean.

### 6.9 Network posture

The daemon has **no outbound network capability at all** in the conversion path. A separate, small `updater` component handles three explicitly user-initiated flows:

1. App updates (signed, staged, transparency-logged)
2. Module registry (signed index; user-triggered)
3. Model downloads (signed index; hash-pinned)

Each is a distinct process with its own allowlist. The UI shows a persistent, honest indicator: **"Offline — 0 network calls this session."** Optionally, a "Sealed mode" that hard-fails if any component attempts egress, for genuinely sensitive environments.

---

## 7. Customization & configuration

### 7.1 Layered configuration

```
built-in defaults
   └─◄ system policy      (org-managed; can LOCK keys — user cannot override)
         └─◄ user config  (~/.openconvert/config.toml)
               └─◄ profile        (Work-Confidential, Web-Publishing, …)
                     └─◄ workspace (.openconvert.toml next to the files)
                           └─◄ recipe
                                 └─◄ CLI flags / UI overrides
```

`openconvert config explain output.metadata_policy` prints the resolved value **and which layer set it** — the debuggability feature every layered config system should have and most don't.

Locked keys render as disabled-with-a-lock-icon in the GUI, with the policy source named. That's what makes enterprise deployment viable.

### 7.2 What's customizable

| Area | Mechanism |
|---|---|
| **Conversion defaults** | Per-format target defaults, quality targets, codec preferences, metadata policy, color policy, naming templates |
| **Naming** | Template DSL: `{name}_{width}x{height}_{date:%Y-%m-%d}{ext}`, plus AI-extracted fields: `{doc.invoice_no}`, `{audio.speaker_1}` |
| **Recipes** | Full pipeline as a file; parameterizable; shareable; signable; pinnable to module versions |
| **Presets** | Named parameter bundles; user-authored; exportable packs |
| **UI layout** | Dockable panels, saved workspaces, density (compact/comfortable), sidebar composition, per-surface hiding ("I only ever convert images — hide everything else") |
| **Theming** | JSON design tokens (color, radius, spacing, type scale) + optional custom CSS in a restricted subset; light/dark/system; high-contrast; icon sets |
| **Keybindings** | JSON, chord support, per-surface, importable presets (default / vim-ish / Photoshop-ish) |
| **Command palette** | Every tool, recipe, preset, and setting is a command; user-defined aliases |
| **Scripting** | Sandboxed JS or Lua in T1 for recipe steps, conditionals, custom naming, and simple transforms. Real work goes in a module. |
| **Watch rules** | Declarative match → plan → destination, with dry-run |
| **Modules** | Install/remove/pin/downgrade; enable per-profile |
| **Models** | Choose model + quantization + backend per tool; bring-your-own-model with a manifest |
| **Engine pinning** | Freeze engine versions per recipe for reproducibility |
| **Policy (org)** | Lock any key; force safety tier; disable Class D; block network integration modules; require signed modules only; enable audit log |
| **Localization** | Standard i18n; RTL support; user-contributable translation packs as data modules |
| **Accessibility** | Full keyboard operation, screen-reader labels on every control, reduced motion, no color-only status encoding, adjustable font scaling |

### 7.3 A recipe, concretely

```toml
schema = 1
name = "Scanned contracts → searchable, redacted, archival PDF"
version = "3"
description = "Paranoid-mode ingest for client documents."

[requires]
app     = ">=1.4"
modules = ["dev.openconvert.mod.docling@^1.2", "dev.openconvert.mod.qpdf@^1.0"]
models  = ["docling-layout@1.3.0", "surya-ocr@0.9.1"]

[settings]
safety      = "paranoid"        # forces T3 isolation
profile     = "Work-Confidential"
on_conflict = "version"

[[steps]]
op = "ingest"
accept = ["application/pdf", "image/*"]
reject_if = { encrypted = true, drm = true }

[[steps]]
op = "paranoid_rebuild"          # pixel bottleneck; kills all active content
dpi = 300

[[steps]]
op = "ocr"
engine = "docling"
languages = ["en", "de"]
layout = true
output = "text_layer"

[[steps]]
op = "detect_pii"
kinds = ["person","address","iban","email","phone","national_id"]
action = "propose"               # never auto-redact; human confirms

[[steps]]
op = "redact"
source = "confirmed_pii"
method = "remove_objects"        # true removal, not black rectangles

[[steps]]
op = "emit"
format = "application/pdf"
pdf_profile = "PDF/A-2b"
metadata_policy = "strip_all_except_title"
name_template = "{doc.counterparty|slug}_{doc.date:%Y-%m-%d}_{name}.pdf"

[[steps]]
op = "receipt"
c2pa = false
audit_log = true
```

This one file demonstrates every claim in the pitch: local, sandboxed, AI-assisted, reproducible, policy-aware, human-in-the-loop, and readable by the person whose job depends on it.

---

## 8. Security architecture

> ⚠️ **Superseded by [09-THREAT-MODEL](09-THREAT-MODEL.md)**, which carries the current controls table, the per-OS implementation *with its fallback profiles*, and the requirement→test map. The per-OS mechanisms below are still accurate; the tier scheme wrapping them is not.

### 8.1 Controls mapped to threats

| Threat (§2.3) | Controls |
|---|---|
| A1 memory corruption | Tier ≥T1 for all third-party parsers; pure-Rust T0 fast paths for common formats; continuous fuzzing (see §8.4); ASLR/CFG/CET enabled on all shipped binaries; per-engine crash quarantine (3 crashes on a format → auto-disable that path + report locally) |
| A2 sandbox escape | Never rely on in-engine sandboxes (`-dSAFER` taught us); Ghostscript at T3 or omitted; defense in depth (WASM ⊕ OS sandbox ⊕ VM) |
| A3 command injection | No shell anywhere; `argv` arrays; inputs renamed to `[a-f0-9]{16}.bin` before engines see them (kills the `InterpretImageFilename` class); parameters type-checked against a schema, never string-concatenated |
| A4 external refs/SSRF | Sandbox has **no network capability** — so even a successful SSRF has nowhere to go; ImageMagick `delegates.xml`/`policy.xml` locked down; XML entity resolution disabled; SVG via resvg (no script, no remote refs); PDF remote resource fetching off |
| A5 resource exhaustion | Governor caps: wall/CPU time, RSS (cgroup / Job Object / rlimit), output bytes, expansion ratio, decoded pixel count, archive depth + entry count + total uncompressed size; WASM fuel metering; streaming decode so a bomb is detected before it's realized |
| A6 polyglot | `infer` + `tree_magic_mini` content sniffing; multi-signature detection → **quarantine and ask**; route strictly by detected type; declared-vs-detected mismatch is surfaced in the receipt |
| A7 active content | CDR on ingest (macros, JS, embedded objects, OpenActions stripped); the IR cannot represent executable content, so rebuild is inherently disarming; Paranoid mode adds the pixel bottleneck |
| A8 metadata leakage | Metadata policy per profile; `Share` strips GPS/serial/author/history/thumbnails; redaction removes objects and re-linearizes; the receipt lists exactly what was removed |
| A9 model supply chain | safetensors/GGUF/ONNX only; pickle rejected at the type layer; signed model index; sha256 verified on every load; ONNX custom-op allowlist |
| A10 module supply chain | Enforced capability manifests; unreviewed = WASM-only, no net, no subprocess; signing + countersigning; SBOM required for Verified tier; reproducible builds; version pinning; signed revocation/kill list |
| A11 update channel | Signed updates + transparency log; staged rollout; **capability-diff prompt**: if an update wants a new capability, the user must approve it |

### 8.2 Per-OS sandbox implementation

| | Linux | macOS | Windows |
|---|---|---|---|
| **T2 process** | user namespace, **seccomp-bpf** allowlist, **Landlock** path rules, cgroup v2 (mem/cpu/pids), no-new-privs, empty netns | App Sandbox + `sandbox_init` profile, no `com.apple.security.network.*` entitlement, hardened runtime, notarized | **AppContainer** (low-privilege SID), restricted token, **Job Object** limits, no network capability SID, mitigation policies (ACG/CIG/DEP/CFG) |
| **T3** | gVisor (preferred) or Firecracker microVM | Virtualization.framework microVM | WSL2 / Hyper-V isolated container |
| **Deps** | none (kernel ≥5.13 for Landlock) | none | optional feature for T3 |
| **Fallback** | Landlock unavailable → seccomp + bind-mount namespace, and **say so in the UI** | — | AppContainer unavailable → Job Object + restricted token, and say so |

**Principle:** never silently degrade. If the strongest available isolation isn't the one requested, the plan preview shows it.

### 8.3 Paranoid mode (CDR), generalized

```
untrusted file
   │
   ├─[T3]─► parse with the risky engine ──► narrow, non-executable bottleneck ──┐
   │           (LibreOffice/pdfium/vips)     (RGB pixels, or a validated IR)    │
   │                                                                            │
   └──────────────────────────────────────────────────────────────────────────► rebuild
                                                                          (trusted, T0,
                                                                       memory-safe Rust)
                                                                                │
                                                              + optional OCR text layer
                                                              + receipt: "no source
                                                                structure survived"
```

Two strengths:
- **Paranoid-Structural** — rebuild from a *validated* IR (DocIR/RasterIR). Preserves text, structure, and selectability. Kills active content and parser-structure exploits. Fast.
- **Paranoid-Pixel** — the Dangerzone approach: rasterize to RGB, rebuild, re-OCR. Maximum assurance, loses vector text and increases file size. Slow.

Offer both, explain the trade-off in one sentence each, and default untrusted-origin documents to Paranoid-Structural.

### 8.4 Security engineering practices (non-negotiable)

- **Continuous fuzzing** of every IR adapter and every parser wrapper (cargo-fuzz / libFuzzer / AFL++), corpora seeded from public format test suites, run in CI, integrated with OSS-Fuzz where the upstream is already there.
- **Dependency inventory + CVE watch** on every bundled engine, with a per-engine "days since upstream release" dashboard. This is a standing operational cost; budget for it.
- **Reproducible builds** so a third party can verify the shipped binary matches the source.
- **Published threat model + security.txt + coordinated disclosure policy + bug bounty** (even a small one).
- **Third-party audit** before 1.0 of: the sandbox implementations, the module capability enforcement, and the update/signing chain. Publish it. (Tauri's own published audit is a useful precedent for why this matters commercially.)
- **A "what we don't protect against" page.** Dangerzone does this and it earns more trust than any marketing claim: exploit chains through the sandbox stack, hardware side channels, a compromised OS, and malicious *content* that's semantically harmful but structurally valid.

---

## 9. Roadmap & MVP scope

### Phase 0 — Spike (4–6 weeks)
**Goal: prove the two hard bets before committing.**
- Bet 1: the IR-hub + planner model actually produces better paths than hardcoded matrices. Build RasterIR + AVGraph, 8 formats, and show `MKV→MP4` choosing stream copy while `AVI→MP4` re-encodes.
- Bet 2: T2 sandboxing works on all three OSes with acceptable overhead. Measure startup cost and throughput for FFmpeg and libvips under seccomp+Landlock / App Sandbox / AppContainer.
- **Kill criteria:** if T2 overhead exceeds ~15% on real workloads, or Windows AppContainer + sidecar proves unworkable, the security pitch needs rethinking before any UI exists.

### Phase 1 — Core converter, no AI (3 months)
- Rust core daemon: detector, planner, scheduler, governor, receipts
- IR hubs: Raster, Audio, AVGraph, Tabular, DocIR (basic)
- Engines: image-rs + libvips + libavif + libjxl (images), FFmpeg LGPL (A/V), Symphonia (probe), Arrow/DuckDB (data), pdfium (PDF basics)
- Tiers T0/T1/T2 on all three OSes
- Tauri GUI: **Drop surface only** + plan preview + receipts
- `openconvert` CLI at parity
- Signed installers, notarized macOS build, Windows signing via Azure Artifact Signing (~$10/mo, GA since April 2026 ⚠️ — far cheaper than a $400+/yr EV cert, and EV no longer buys instant SmartScreen reputation ⚠️)
- **Ship it.** A fast, safe, honest, offline converter with no AI is already a product people want.

### Phase 2 — Modules + first AI (3 months)
- Module system v1: manifests, capability enforcement, signing, registry client, install/pin/rollback, kill switch
- Model Manager + ONNX Runtime with EP auto-selection + benchmark-on-first-use
- First AI tools: **OCR/doc-structure (Docling), ASR (Whisper), background removal (BiRefNet), image upscale (Real-ESRGAN)**
- Studio (pipeline canvas) + Recipes + Presets
- Watch folders

### Phase 3 — Depth + trust (4 months)
- Video pipeline: hardware encoders, per-title VMAF targeting, HDR tone mapping, subtitle generate/burn, interpolation, upscale
- Paranoid mode (T3 + both CDR strengths)
- Verification suite (SSIM/PSNR/VMAF/text-diff), C2PA
- Intent engine
- Third-party module SDK + docs + example modules
- Security audit + published threat model

### Phase 4 — Scale (ongoing)
- Team/enterprise: policy layer, audit log, offline module/model bundles, MSI/PKG deployment
- MCP server + local API
- Community registry with the trust tiers
- Long-tail modules: 3D, CAD, GIS, email, fonts, RAW, ebooks

### Explicit non-goals (write these down and defend them)
- Cloud conversion of user files (ever)
- DRM circumvention
- Being a full editor (Photoshop/Premiere/Acrobat) — we convert and enhance, we don't author
- Mobile apps before 1.0
- Multi-user server mode before enterprise demand is proven
- Any capability whose sandbox we can't enforce

---

## 10. Business model options

Given a local, offline, no-telemetry product, subscriptions are a hard sell and unenforceable offline. Recommended shape:

| Tier | Price shape | Contents |
|---|---|---|
| **Core** | Free, source-available | Full deterministic converter, all Class A/B ops, sandboxing, CLI, recipes. This *is* the trust anchor and the distribution engine. |
| **AI Pack** | One-time, per major version (e.g. $39, includes 12 months of updates, keeps working forever) | Class C/D modules: OCR/doc-structure, ASR/subtitles, upscale/restore, matting, stems, intent engine |
| **Pro** | One-time (e.g. $69) | Watch folders, unlimited batch automation, verification suite, C2PA, engine pinning, MCP/API |
| **Team** | Per-seat annual | Policy layer, locked config, audit log, offline bundles, MSI/PKG + SSO-less deployment, priority security SLA, commercial model licenses |
| **Module marketplace** | Rev-share | Third-party modules; we take a cut and provide signing + review |

**Licensing of the app itself.** Two viable options:
- **Open core (AGPL core + proprietary modules)** — maximum trust, matches ConvertX/Stirling precedent, but AGPL complicates commercial module linking (keep modules as separate processes, which our architecture already does).
- **Source-available (BSL/FSL, converting to Apache after 4 years)** — allows verification (the actual user need) while preventing an immediate SaaS clone.

Recommendation: **source-available for the core, permissive for the module SDK.** The user's need is *verifiability*, not *forkability*; the SDK must be permissive or nobody builds modules.

**Non-negotiable marketing constraints:** no telemetry, no accounts required for the free tier, no bundled offers, no "free trial that expires mid-conversion." The entire pitch is trustworthiness; one dark pattern destroys it.

---

## 11. Risk register & open questions

| # | Risk | Sev | Likelihood | Mitigation |
|---|---|---|---|---|
| R1 | Codec patent exposure (H.264/HEVC/AAC) on a commercial binary | High | Med | Royalty-free defaults (AV1/Opus/FLAC/AVIF); patented *encoding* only via OS/hardware encoders or opt-in modules; decode-only posture; get counsel before charging money |
| R2 | LGPL/GPL contamination of a proprietary build | High | Med | LGPL-only FFmpeg, dynamic linking, GPL tools only as separate processes, automated license-scan gate in CI, compliance page + source mirror in the release checklist |
| R3 | Engine CVE treadmill overwhelms a small team | High | **High** | Minimize the engine set; prefer memory-safe Rust; sandbox everything; automated upstream-CVE monitoring; module kill switch; publish an SLA and staff for it |
| R4 | Module ecosystem becomes a malware channel (the VS Code/Open VSX outcome) | High | Med | WASM-only for unreviewed modules; enforced capabilities; no net/subprocess without review; signing + revocation; capability-diff on update |
| R5 | Non-commercial model licenses shipped by accident (the RMBG-2.0 trap) | High | Med | License field in the model registry; CI gate blocking `commercial=false` in paid tiers; legal review of every bundled checkpoint |
| R6 | GPU/NPU heterogeneity → "works on my machine" support hell | Med | High | ONNX Runtime EP abstraction + measured backend selection + always-available CPU fallback + in-app hardware report to attach to bug reports |
| R7 | Installer bloat kills the "small core" promise | Med | High | Hard budget: core < 60 MB. LibreOffice (~400 MB), ImageMagick, and all models are downloadable modules. Enforce in CI. |
| R8 | Scope explosion; "all-in-one" never ships | High | **High** | Phase 1 ships with zero AI. The module system is the scope-control mechanism. Non-goals list is defended in writing. |
| R9 | Users perceive AI features as unwanted "AI slop" | Med | Med | Class D off by default, clearly labeled, never in the conversion path; the app is fully usable with AI uninstalled |
| R10 | Tauri sidecar/WebView friction across platforms | Med | Med | Daemon/shell split means the shell is replaceable; test matrix across WebKitGTK/WKWebView/WebView2 from week one; budget for the known rpath issue ⚠️ |
| R11 | Performance expectations vs. sandbox overhead | Med | Med | T0 fast paths for common formats; measure and publish overhead; make tier visible so users understand the trade |
| R12 | Platform store policies (Mac App Store sandbox vs. subprocess engines) | Med | Med | Direct distribution is primary; MAS build (if any) is a reduced-capability variant, explicitly labeled |
| R13 | Someone big ships this first | Med | Low | The security architecture is the moat; it's the part that's slow to copy and impossible to bolt on |

### Open questions requiring a decision

1. **License of the core** — AGPL vs. BSL/FSL. Affects the module ecosystem and any future hosted offering. *Decide before the first public commit.*
2. **Ghostscript: ship or omit?** Omitting loses EPS/PS and some PDF ops; shipping means owning the highest-risk engine in the stack. *Recommendation: omit from core; offer as a user-installed T3-only module.*
3. **LibreOffice dependency** — it's ~400 MB and the only realistic path to broad Office fidelity. Bundled module, or "point us at your installed copy"? *Recommendation: both, with the latter as the default on machines that already have it.*
4. **Language for modules** — Rust-only SDK first, or Rust + Go + Zig + AssemblyScript via the WASM component model? *Recommendation: Rust-first, publish the WIT interface so others can follow.*
5. **Extism vs. raw wasmtime + component model** for the plugin host. Extism is purpose-built for plugin systems with multi-language host SDKs; wasmtime + components gives type-safe composition with more setup. *Recommendation: prototype both in Phase 2; the WIT-defined interface is portable either way.*
6. **Does the intent engine ship an LLM, or use the OS one?** Apple's Foundation Models framework (WWDC 2026) now fronts on-device and third-party models via one Swift protocol ⚠️; Windows ML provides OS-managed EPs. Using OS models cuts hundreds of MB but fragments behavior across platforms.
7. **C2PA** — worth the complexity in v1, or Phase 3? *Recommendation: Phase 3, but design the receipt schema now so it maps cleanly.*
8. **Name/brand** — "OpenConvert" is precise but generic and reads as an acronym soup. The domain is in hand; the product name may want to be warmer.

---

## 12. Research gaps — what to verify next

Several findings above came from secondary/SEO-grade sources and are marked ⚠️. Before any of them drives a decision, verify against primary sources:

| Claim | Verify against |
|---|---|
| Model licenses for every checkpoint we intend to bundle | The actual `LICENSE` file in each model repo — not the README, not a blog |
| CVSS scores and exploitation status of engine CVEs | NVD, upstream advisories, CISA KEV |
| Benchmark numbers for doc parsers (olmOCR-bench), TTS, ASR | The benchmark repos and papers; re-run on our own corpus |
| Browser/format adoption percentages (JXL, AVIF) | caniuse.com directly |
| Market sizing | A real analyst report, or ignore it — it shouldn't drive product decisions anyway |
| Tauri sidecar rpath issues, WebView differences | Tauri issue tracker; build a spike |
| Windows code-signing costs and SmartScreen behavior | Microsoft Learn + Azure pricing page, current at purchase time |
| Apple Foundation Models third-party provider protocol | Apple developer documentation + the WWDC 2026 session |
| VERT's exact engine attribution | The VERT source tree (its README doesn't enumerate engines) |
| Whether HTDemucs/RIFE/Real-ESRGAN weights permit commercial redistribution | Per-checkpoint license files |

Also unresearched and worth doing before Phase 1:
- **User research.** Zero real users were consulted for this document. Ten interviews with the personas in §5.2 would likely reorder the Magic Tools catalog significantly.
- **Competitive pricing reality.** What do people actually pay for converters today?
- **Accessibility requirements** for target enterprise/government buyers (Section 508, EN 301 549) — these are procurement gates, not nice-to-haves.
- **RAW decoding** (libraw licensing and camera coverage) — the single most-requested long-tail format family in photography.

---

## Appendix A — Target format coverage

Phase 1 (core, no modules) — the 80% by volume:

| Domain | In | Out |
|---|---|---|
| Images | PNG, JPEG, WebP, GIF, BMP, TIFF, AVIF, HEIC, SVG, ICO, PSD (flat) | PNG, JPEG, WebP, AVIF, JXL, TIFF, BMP, GIF, PDF |
| Audio | MP3, AAC/M4A, FLAC, WAV, OGG/Vorbis, Opus, ALAC, AIFF, WMA | MP3, AAC, FLAC, WAV, Opus, ALAC, OGG |
| Video | MP4, MKV, MOV, AVI, WebM, WMV, FLV, TS, M4V | MP4, MKV, WebM, MOV, GIF, frame sequences |
| Documents | PDF, TXT, MD, HTML, RTF, DOCX*, ODT* | PDF, MD, HTML, TXT, DOCX*, EPUB* |
| Data | CSV, TSV, JSON, JSONL, YAML, TOML, XML, Parquet, XLSX | all of the above |
| Archives | ZIP, TAR(.gz/.bz2/.xz/.zst), 7z, RAR (extract) | ZIP, TAR.*, 7z, ZSTD |

`*` via the LibreOffice/Pandoc modules. Phase 2+ adds: RAW, ebooks, 3D, CAD, GIS, email, fonts, subtitles, notebooks, DICOM (read), and the ImageMagick long tail.

## Appendix B — CLI surface

```bash
# one-shot
openconvert convert photo.heic -t avif --quality 62
openconvert convert *.mkv -t mp4 --lossless-only        # fails loudly rather than re-encoding
openconvert convert scan.pdf -t pdf --safety paranoid --ocr en,de

# inspect before doing
openconvert plan *.mov -t mp4 --json                    # the plan object, machine-readable
openconvert inspect weird.jpg                           # detected type, polyglot warnings, metadata
openconvert verify original.mkv converted.mp4           # VMAF/SSIM/stream comparison

# recipes & automation
openconvert run contracts.recipe.toml ./inbox --out ./archive
openconvert watch add ./Downloads --recipe safe-pdf.recipe.toml
openconvert watch status

# natural language (local model, always shows the plan first)
openconvert do "turn these into 1080p mp4s under 100MB each" *.mov

# modules & models
openconvert module add dev.openconvert.mod.libreoffice
openconvert module list --capabilities
openconvert model pull whisper-large-v3-turbo
openconvert model bench

# config
openconvert config explain output.metadata_policy
openconvert doctor                                      # hardware, sandbox availability, engine versions

# agent integration
openconvert mcp serve                                   # expose tools over MCP on a local socket
```

Every command supports `--json`, `--dry-run`, and meaningful exit codes. `--json` output is schema-versioned.

## Appendix C — Glossary

| Term | Meaning |
|---|---|
| **IR hub** | Canonical intermediate representation that all conversions route through (§6.2) |
| **Tier (T0–T3)** | Isolation strength for executing a parser/encoder (§6.3) |
| **Class (A–D)** | Fidelity/determinism class of an operation (§5.9) |
| **Receipt** | Machine-readable record of what a conversion did (§5.8) |
| **Recipe** | Saved, parameterized, shareable pipeline file |
| **Profile** | Named bundle of defaults (quality, metadata, safety, naming) |
| **CDR** | Content Disarm & Reconstruction — parse, strip active content, rebuild clean |
| **Paranoid mode** | Conversion through a non-executable bottleneck (structural or pixel) |
| **Planner** | A* path search over the converter graph using a weighted cost function |
| **Capability** | A permission a module declares and the runtime enforces |

---

## Sources

**Competitive / prior art**
- [VERT — GitHub](https://github.com/VERT-sh/VERT) · [VERT README](https://github.com/VERT-sh/VERT/blob/main/README.md) · [vert.sh](https://vert.sh/)
- [ConvertX — GitHub](https://github.com/c4illin/ConvertX)
- [CloudConvert — file converter](https://cloudconvert.com/) · [CloudConvert API operations/engines](https://cloudconvert.com/docs/api-reference/operations)
- [Dangerzone — About](https://dangerzone.rocks/about/) · [Dangerzone — GitHub](https://github.com/freedomofpress/dangerzone) · [Reducing attack surface with gVisor](https://dangerzone.rocks/news/2024-09-23-gvisor/)
- [Stirling PDF — docs](https://docs.stirlingpdf.com/) · [Stirling PDF self-hosting overview](https://blog.elest.io/self-host-stirling-pdf-50-private-pdf-tools-on-your-server/)
- [Upscayl review](https://www.aiarty.com/ai-image-enhancer/upscayl-review.htm) · [Video2X overview](https://www.greptile.com/grepository/video2x)

**Security**
- [CVE-2025-55298 — ImageMagick RCE](https://www.sentinelone.com/vulnerability-database/cve-2025-55298/) · [CVE-2025-68618 — ImageMagick SVG DoS](https://www.sentinelone.com/vulnerability-database/cve-2025-68618/) · [Critical ImageMagick RCE (BMP encoder)](https://gbhackers.com/critical-imagemagick-vulnerability/) · [CVE-2025-54418 analysis](https://vicevirus.github.io/posts/cve-2025-54418/)
- [CVE-2024-29510 — Ghostscript format-string / -dSAFER bypass (Codean Labs)](https://codeanlabs.com/2024/07/cve-2024-29510-ghostscript-format-string-exploitation/) · [Ghostscript wrap-up: overflowing buffers](https://codeanlabs.com/2024/10/ghostscript-wrap-up-overflowing-buffers/) · [Exploited in the wild (BleepingComputer)](https://www.bleepingcomputer.com/news/security/rce-bug-in-widely-used-ghostscript-library-now-exploited-in-attacks/)
- [FFmpeg security page](https://www.ffmpeg.org/security.html) · [CVE-2025-1373 — FFmpeg MOV parser UAF](https://www.sentinelone.com/vulnerability-database/cve-2025-1373/) · [USN-7830-1 FFmpeg vulnerabilities](https://ubuntu.com/security/notices/USN-7830-1)
- [Wasmtime security model](https://docs.wasmtime.dev/security.html) · [Landlock](https://landlock.io/talks/2024-06-06_landlock-article.pdf) · [Locking down science gateways with Landlock and seccomp](https://arxiv.org/pdf/2509.18548)
- [Zip bomb (Wikipedia)](https://en.wikipedia.org/wiki/Zip_bomb) · [A better zip bomb](https://www.bamsoftware.com/hacks/zipbomb/) · [Pillow decompression-bomb protection](https://github.com/python-pillow/Pillow/issues/515)
- [Content Disarm and Reconstruction (Wikipedia)](https://en.wikipedia.org/wiki/Content_Disarm_and_Reconstruction) · [What is CDR (OPSWAT)](https://www.opswat.com/blog/what-is-content-disarm-and-reconstruction)
- [Safetensors security audit (Hugging Face)](https://huggingface.co/blog/safetensors-security-audit) · [Pickle scanning (Hugging Face)](https://huggingface.co/docs/hub/en/security-pickle) · [PickleScan zero-days (JFrog)](https://jfrog.com/blog/unveiling-3-zero-day-vulnerabilities-in-picklescan/)
- [Supply chain risk in VS Code extension marketplaces (Wiz)](https://www.wiz.io/blog/supply-chain-risk-in-vscode-extension-marketplaces) · [GlassWorm / Open VSX (The Hacker News)](https://thehackernews.com/2026/03/glassworm-supply-chain-attack-abuses-72.html) · [Malicious VS Code extension → OctoRAT (Hunt.io)](https://hunt.io/blog/malicious-vscode-extension-anivia-octorat-attack-chain)
- [Obsidian plugin security criticism](https://biggo.com/news/202509200713_Obsidian_Plugin_Security_Concerns)
- [FBI warning on free file-converter malware (context)](https://www.experian.com/blogs/ask-experian/risks-of-using-online-file-pdf-converters/) · [Privacy risks of online converters](https://guardiandigital.com/content/online-file-conversion-tools-privacy-risks)

**Licensing**
- [FFmpeg legal / licensing](https://www.ffmpeg.org/legal.html) · [FFmpeg commercial license guide](https://32blog.com/en/ffmpeg/ffmpeg-commercial-license-guide)
- [BRIA RMBG-2.0 (CC BY-NC 4.0)](https://huggingface.co/briaai/RMBG-2.0)

**AI / runtimes / models**
- [ONNX Runtime execution providers](https://onnxruntime.ai/docs/execution-providers/) · [DirectML EP](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html) · [CoreML EP](https://onnxruntime.ai/docs/execution-providers/CoreML-ExecutionProvider.html) · [OpenVINO EP](https://onnxruntime.ai/docs/execution-providers/OpenVINO-ExecutionProvider.html) · [Windows ML execution providers](https://learn.microsoft.com/en-us/windows/ai/new-windows-ml/supported-execution-providers)
- [MinerU](https://github.com/opendatalab/mineru) · [Marker v2 vs MinerU/Docling benchmark](https://www.marktechpost.com/2026/07/24/datalab-marker-v2-vs-mineru-docling-and-liteparse-benchmark-breakdown/) · [Open-source PDF→Markdown deep dive](https://jimmysong.io/blog/pdf-to-markdown-open-source-deep-dive/)
- [rembg](https://github.com/danielgatis/rembg) · [BiRefNet overview](https://tasarim.ai/en/models/birefnet)
- [Qwen3-VL](https://github.com/QwenLM/Qwen3-VL) · [Best local VLMs 2026](https://tinyweights.dev/posts/best-local-vision-language-models-2026/)
- [whisper.cpp vs faster-whisper (2026)](https://codersera.com/blog/faster-whisper-vs-whisper-cpp-speech-to-text-2026/)
- [Open-source TTS comparison 2026](https://texttolab.com/blog/open-source-text-to-speech) · [Kokoro vs Piper vs XTTS](https://contracollective.com/blog/kokoro-vs-piper-vs-xtts-local-text-to-speech-m5-max-2026)
- [Apple Foundation Models opened to any LLM provider (WWDC 2026)](https://dev.to/arshtechpro/wwdc-2026-apple-just-opened-the-foundation-models-framework-to-any-llm-provider-5ejn)
- [RIFE AI interpolation](https://www.svp-team.com/wiki/RIFE_AI_interpolation)

**Engineering / platform**
- [Tauri v2 — embedding external binaries (sidecar)](https://v2.tauri.app/develop/sidecar/) · [Tauri vs Electron (DoltHub)](https://www.dolthub.com/blog/2025-11-13-electron-vs-tauri/)
- [WebAssembly Component Model & WASI 0.3 in 2026](https://jsmanifest.com/wasm-component-model-wasi-javascript-developers)
- [Symphonia (pure-Rust audio)](https://github.com/pdeljanov/Symphonia) · [pdfium crate](https://crates.io/crates/pdfium)
- [Code signing options for Windows (Microsoft Learn)](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/code-signing-options) · [Azure Artifact Signing for indie devs](https://melatonin.dev/blog/code-signing-on-windows-with-azure-trusted-signing/)
- [AVIF vs JPEG XL 2026](https://uploadcare.com/blog/avif-vs-jpeg-comparison/) · [HDR gain maps vs tone mapping (Greg Benz)](https://gregbenzphotography.com/hdr-photos/hdr-gain-maps-vs-tone-mapping/)

**Market**
- [File converter software market (Verified Market Research)](https://www.verifiedmarketresearch.com/product/file-converter-software-market/) ⚠️
