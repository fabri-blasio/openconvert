# OpenConvert — Website pages

Page-by-page blueprint for `openconvert.dev`, built against [11-SITE-RESEARCH](11-SITE-RESEARCH.md). Supersedes the page specifications in [10-WEBSITE §4–5](10-WEBSITE.md) once approved.

Status: **draft v0.2** · 2026-08-25
Voice rules and gates inherit from [10-WEBSITE §2, §9](10-WEBSITE.md). Design tokens inherit from [07-DESIGN-SYSTEM](07-DESIGN-SYSTEM.md).

---

## Design direction

Minimal, modern open-source tool. Whitespace does the work; text earns its place.

| Principle | In practice |
|---|---|
| **Sparse** | Short phrases. Every section intro ≤2 lines, centre-aligned. Tables and code carry the detail |
| **Space** | Section padding 128–160px desktop. Prose column 68ch. Nothing crowds |
| **Alignment** | Hero and section intros centred; tables/code left. Asymmetric only for the demo |
| **Depth, not decoration** | Near-monochrome, hairlines, one accent (verdigris). No cards-within-cards |
| **Motion** | Micro only: hover, focus, reveal-on-scroll (once, subtle, 220ms). Nothing loops. Sequences bind to `animation-timeline: view()` so they play when reached rather than when the document loads, and only ever animate `transform`/`opacity` — an inactive timeline must cost pixels, never content |

### The ambient layer (hero)

A single WebGL canvas sits behind the hero: a slow-drifting field of small pills — engine and model names (`FFmpeg`, `libvips`, `LibreOffice`, `Whisper`, `PaddleOCR`, `Kokoro`, `Real-ESRGAN`…) orbiting the headline at low opacity.

| Rule | Value |
|---|---|
| Weight | ≤15 KB gzipped, one fragment shader, lazy-loaded after LCP |
| Fallback | Static gradient + plain pill row (no JS) |
| Reduced motion | Pills render static, scattered |
| Performance | Paused when scrolled out of view; DPR capped at 2 |

This is the only WebGL on the site. It doubles as attribution — the engines are visible before any claim is read.

---

## Sitemap — 12 routes

| Group | Route | One job |
|---|---|---|
| Product | `/` | Try it in 30 s; understand it in 60 |
| Product | `/features` | Every capability on its own anchor |
| Product | `/safety` | Bottom-up containment narrative |
| Product | `/download` | Right binary, verified |
| Product | `/changelog` | Release history through 1.0.0 |
| Company | `/about` | Who, why, licence, sustainability |
| Company | `/contact` | Email + PEC + LinkedIn |
| Legal | `/privacy` | Both audiences, ≤300 words |
| Legal | `/terms` | Italian-law website terms |
| Legal | `/accessibility` | WCAG 2.2 AA statement backed by gates |
| Legal | `/licences` | Apache-2.0 + engine/model notices (generated) |
| Legal | `/imprint` | Art. 2250 c.c. company data |

**Chrome.**

- Nav: logo · Features · Safety · Changelog · About · GitHub · Discord · **Download**
- Footer: `built from <sha>` · No analytics · No cookies · No tracking · Clura srls · PI 18314321003 · Privacy · Terms · Accessibility · Licences · Imprint
- Artefacts: `/.well-known/security.txt`, `robots.txt`

---

## `/`

Budget: **≤250 prose words**. Nine sections, most of them visual.

### 1. Hero *(centre-aligned, ambient layer behind)*

> # Convert & modify any file locally
> Every step on your machine. Every job receipted.

[ CTA: **Download** ]  [ Try in browser ↓ ]

### 2. The app, in the page

An embedded replica of the real app UI — sidebar, tools, settings included. Only image conversion is wired; everything else renders inert but honest.

Microcopy beneath, centre-aligned:

> No uploads. Just a sample of the actual app.

- Island shell ≤40 KB; engine modules (`libvips.wasm`) fetch lazily from this domain on first use
- Network-truth line: *"Requests go only to this site. Your files never do."*
- CLS-safe: fixed-height app frame

### 3. One job, four screens *(built)*

The desktop window, reproduced in markup at the app's own measurements
(`site/src/styles/app-ui.css`, `AppWindow.astro`), moved through the four
screens a conversion passes: drop → plan → confined → receipt. Scroll-driven
CSS, no JavaScript, so the same component works on pages that ship
`script-src 'none'`.

Every value on screen is read from `data/receipt.json` — the capture of one real
`openconvert convert bundle.zip -t tar --json` run — so the walkthrough and the
receipt disclosure beneath it describe the same job.

Degradation is the design, not an afterthought: a scroll timeline is inactive
until the page composites, so screen 1 carries no animation and the four step
captions sit outside the frames. A browser without scroll-driven animations, a
reader who asked for reduced motion, and a tab restored in the background all
get a window plus every word.

### 4. Refusal path

Unsupported pair gets a named reason, never silence:

```
MKV → MP4 runs in the desktop app — FFmpeg doesn't fit a browser tab.

  openconvert convert movie.mkv movie.mp4      [copy]
```

### 5. Receipt — behind a disclosure

After a conversion completes, the result row carries `receipt ▾`. Opened:

```json
{
  "input":   { "file": "scan.heic", "sha256": "9e4a8b…" },
  "output":  { "file": "scan.jpg",  "sha256": "c02f31…" },
  "steps": [
    { "engine": "libheif 1.19", "op": "decode", "class": "=" },
    { "engine": "libvips 8.15", "op": "encode", "class": "≈", "params": "q=82" }
  ],
  "sandbox": "per-engine, no network",
  "network_calls": 0,
  "duration_ms": 340
}
```

Caption: *"Real export. Every conversion emits one."*

### 6. What it does *(centre-aligned intro, card grid)*

> ## One tool. The whole job.

Eight quiet cards — icon, verb, four words each:

Convert `[N] formats` · OCR `scans → searchable` · Transcribe `audio → text` · Remove background `local matting` · Upscale `on-device` · Narrate `text → speech` · Read documents `→ Markdown` · Headless `CLI & CI`

### 7. Four claims, four lines

> ⛨ **Confined.** Engines get file descriptors, never filenames.
> **Verifiable.** Every job ends in a receipt.
> **Non-destructive.** Overwriting your files is impossible, not discouraged.
> **Open.** Apache-2.0 — new local models appear here as fast as the ecosystem ships them.

Each links to its proof page.

### 8. Comparison — archetypes + examples

| | Cloud (iLovePDF, Convertio) | Browser-local (VERT.sh) | Self-hosted (ConvertX, Stirling-PDF) | **OpenConvert** |
|---|---|---|---|---|
| Files leave your machine | Yes | Video does | You operate a server | Never |
| Engine isolation | Their promise | Browser jail | One shared container | Per-engine, by the OS |
| Says what it did | Retention policy | No | No | Receipt |
| Inferred vs generated | No | — | No | Enforced |

Beneath, centred:

> Meant to be the last stop for 99% of those searches — "pdf to …", "jpg to …". Locally.

### 9. Get OpenConvert

```
[ Linux ]        [ Windows ]     [ macOS ]
.AppImage .deb    .msi .exe        .dmg
```

```bash
openconvert convert scan.heic scan.jpg --json   # headless, same engine
```

*Apache-2.0 · Free forever — support and enterprise licensing fund development.* → `/download`

---

## `/features`

H1: *What OpenConvert does*. Anchor chips: `Conversions · AI · Predictor · Headless · Platforms`.

### `#conversions`

- Lead: "[N] formats across [M] categories — every route classed before it runs." *(numbers until export)*
- Legend: `=` lossless · `≈` lossy · `⌇` reads what's there · `✦` invents · ⛨ confined
- **Full route table** — generated, every path: source → target · class · engine · steps. Filterable client-side within the island budget
- Plan-before-run terminal block (batch example, struck-through refusal, `0 network calls`)
- Fidelity, four bullets: remux over re-encode · ICC carried through · HDR→SDR only named operators · metadata is policy

### `#ai`

Lead: "Eight models. All third-party. All named."

| Capability | Model | Size | Licence |
|---|---|---|---|
| OCR | PaddleOCR-mobile | 21 MB | Apache-2.0 |
| Transcription | Whisper base int8 | 74 MB | MIT |
| Speech detection | Silero VAD | 2 MB | MIT |
| Background removal | u2netp / BiRefNet-lite | 5 MB | Apache-2.0 |
| Upscaling | Real-ESRGAN compact | [size] | BSD-3 |
| Narration | Kokoro-82M | [size] | Apache-2.0 |
| Noise removal | DeepFilterNet | [size] | [verify] |
| Document structure | Granite-Docling-258M | 248 MB | MIT |
| Target ranking *(predictor)* | [model] | [size] | [licence] |

Generated from `models.toml`.

> "`⌇` reads your scan. `✦` invents pixels. Both labelled everywhere — and `✦` never arms itself."
>
> Pack is an opt-in ~500 MB download. Conversion works fully without it.

### `#predictor` *(built in the app's own row)*

> You present a file. OpenConvert proposes a destination — reason visible — and waits.

```
scan.pdf      → searchable PDF     "no text layer found"
IMG_0041.heic → JPEG               "24 sibling JPGs here"
movie.mkv     → MP4 stream copy    "codecs already compatible"
```

*A suggestion selects `⌇` and `=`. Never `✦`.*

### `#headless`

Verb rows: `convert` · `plan` · `routes` · `inspect` · `verify` — one invocation each. `--json` everywhere · exit codes · recipes · watch folders · local MCP for agents.

### `#platforms`

| OS | Confinement | Package |
|---|---|---|
| Windows | AppContainer · restricted token · Job Object | `.msi` `.exe` |
| macOS | App Sandbox | `.dmg` |
| Linux | Landlock · seccomp · namespaces | `.deb` `.AppImage` |

*If a mechanism is unavailable, the engine refuses rather than runs exposed.*

---

## `/safety`

Bottom-up narrative, Tailscale shape. Slimmed.

### 1. The problem

> A converter runs memory-unsafe parsers over files strangers wrote. Local conversion runs them on your machine. Local without confinement is a downgrade.

### 2. Five stages, build order

| Stage | Idea |
|---|---|
| Detect | Content, never extension |
| Route | Classed plan; refusal is an outcome |
| Confine | Per-engine OS sandbox — own filesystem slice, no network, descriptors not filenames |
| Execute | What the engine tries, logged |
| Receipt | What happened, written down |

Denial log:

```
connect() → 203.0.113.7:443    EPERM  · no network namespace
open("~/.ssh/id_rsa")          EACCES · outside job directory
fork() → /bin/sh               EPERM  · seccomp denies execve
write("../../Startup/x.lnk")   EACCES · traversal refused

invoice.pdf → invoice.png · completed · nothing escaped
```

> **Recommended, kept as one collapsed block:** a short *"What we don't protect against"* `<details>` at the end of this page (sandbox-chain-to-kernel, compromised OS, side channels). It is our rarest differentiator — one paragraph, not a section.

---

## `/download`

1. **Three platform cards**, fixed order. *We don't auto-detect your OS — that takes JavaScript, and this site runs none on you.*
   - Windows `.msi` `.exe` · macOS `.dmg` · Linux `.deb` `.AppImage` — version, size each
2. **Verify** *(one line — restored: a security product without published checksums invites the wrong assumptions)*: sha256sums · minisign signature · SBOM · reproducible-build instructions. Generated by the release job.
3. **AI pack:** opt-in, ~500 MB, `openconvert models pull`. Conversion works without it.
4. **First run:** the weekly signed revocation check, disclosed again, switchable off.

---

## `/changelog`

Release entries through 1.0.0, newest first. Pre-1.0 collapses into one **Road to 1.0** entry linking GitHub milestones.

```markdown
## 1.0.0 — YYYY-MM-DD
Highlights            ← ≤5 bullets
🔒 Security fixes     ← dated, linked
Breaking changes      ← migration line
Engine bumps          ← receipts pin versions
Full diff → GitHub
```

---

## `/about`

1. **Why.** *"In March 2025 the FBI warned people away from online file converters — enough of them were malware. Every option left asked to be trusted with your files. We wanted a tool you didn't have to take on faith."*
2. **Team.** Clura srls · founder(s), roles.
3. **Licence.** Apache-2.0 in plain words. → `/licences`
4. **Attribution.** Engines and models credited high on the page, licences and link modes attached. Upstream projects read how they're described.
5. **Sustainability.** Binary free forever. Support contracts and enterprise licensing fund development. Nothing needed to convert a file is paid.

---

## `/contact`

- `[hello@openconvert.dev]` — humans. Response within [X] days (CET)
- PEC `clura@legalmail.it` — formal notices
- Security → `security.txt`
- LinkedIn `[url]` · Discord `[url]`

No form.

---

## Legal group

Short by construction: no cookies, no analytics — statements, not boilerplate. No consent banner needed; `/privacy` says so once.

### `/privacy` — ≤300 words, gated

1. **Site.** No cookies, analytics, ads, third parties. Vercel Inc. processes transient connection logs as processor; no profiles.
2. **Software.** Files never leave the machine. One weekly signed revocation fetch — no identifier, no query. Off-switch included, trade named. Local data wipeable.
3. **Contact.** Answered mail; basis: legitimate interest.
4. **Rights.** Arts. 15–21 → email or PEC. Controller: Clura srls, [address], Italy.
5. **Changes.** Announced in `/changelog` first.

### `/terms`

Site © Clura srls; software under Apache-2.0 (its warranty/liability clauses govern). Site as is; links ≠ endorsements; marks of Clura srls. Law: Italy; forum [province]; consumer rights unaffected. Formal notice: PEC.

### `/accessibility`

WCAG 2.2 AA, enforced by gates: axe + keyboard per template · contrast 4.5:1 / 3:1 · reduced-motion resolves to end states · CLS 0 · never colour-only state. Limitation: demo needs JS; static receipt renders without. Feedback: `[email]`.

### `/licences`

Generated; hand-edits fail the build. 1. Apache-2.0 text · 2. Engine table (licence, link mode, source, FFmpeg notice, LGPL source offer) · 3. Model table with full licence texts.

### `/imprint`

```
Clura srls
Sede legale: [address]
PI 18314321003 · REA [—] · Cap. soc. €[—] iv.
Reg. Impr. [—] n. [—]
PEC clura@legalmail.it · [hello@…]
Rappresentante legale: [name]
```

---

## Open items blocking v1

| # | Item | Blocks |
|---|---|---|
| 1 | Registered office, REA, share capital, registro, legal representative | imprint, privacy, terms |
| 2 | Format counts from routes export | features table, homepage |
| 3 | Packages, sizes, signing/notarisation status | download, homepage |
| 4 | Model sizes (ESRGAN/Kokoro/DeepFilterNet) + predictor model identity | model tables |
| 5 | Mailboxes + LinkedIn/Discord URLs | contact, footers, security.txt |
| 6 | Response-time numbers | three pages |
| 7 | Audit status wording | safety |
| 8 | Ambient-layer shader + embedded-app-island scope sign-off | homepage |
