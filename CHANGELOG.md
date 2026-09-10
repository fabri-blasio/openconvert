# Changelog

Everything built, by phase, against
[08-EXECUTION-PLAN](docs/spec/08-EXECUTION-PLAN.md)'s 44-week schedule.

**Maintained per commit, not reconstructed at the end.** Entries that record a
defect say what it was and **how it was found**, because "found by X" is the
part that tells you whether X is worth keeping.

Cleared and restarted on 2026-09-08, at the rename from Transitus to
OpenConvert. The previous file — 2,034 lines covering phases 0 through 4 — is
in git history at `fd7fa67` and is the place to look for anything before this
date.

| | |
|---|---|
| **Tests** | **704 across 29 suites**, green on Windows · `cargo test --workspace` |
| **CI steps** | **12**, all green locally · `cargo xtask ci` |
| **Fuzz targets** | 7 |
| **Conversions** | **50 formats, 268 routes**, every one enumerated in [docs/ROUTES.md](docs/ROUTES.md) · generated from the route table, never transcribed |
| **Common conversions** | **21 of CloudConvert's 33**, measured one `openconvert routes` call each · [the comparison](docs/cloudconvert-comparison.md) |
| **Licence surfaces** | **9 native engines** declared, 4 LGPL, all dynamically linked · `node release/notice-gate.mjs` |
| **Platforms verified** | **Windows**. Linux and macOS are covered by the CI matrix in `.github/workflows/ci.yml`, not by this machine |

---

## The rename: Transitus becomes OpenConvert · 2026-09-08

Every crate, engine, binary, bundle and document. `openconvert`,
`openconvert-core`, `-run`, `-os`, `-sandbox`, `-wasm`; the five workers become
`oc-images`, `oc-pdf`, `oc-archive`, `oc-audio`, `oc-ai`; the shell becomes
`openconvert-desktop` and its installers `OpenConvert_0.1.0_x64*`.

### The gate that skipped exactly the files the rename orphaned

`xtask desktop` checks that every staged engine is newer than the crate that
builds it. A binary with **no** crate of that name it skipped outright — "not
ours to judge".

That is wrong, and the rename proved it. `bundle.resources` declares
`engines/*`, so every `.exe` in that directory goes into the installer whether
or not anything builds it. After the crates moved, five stale `tx-*.exe` files
sat there under names no crate produced any more — skipped by that branch, while
the gate reported ok. An installer built at that moment would have shipped five
engines from the previous build, under names `EngineBin::file_stem` no longer
looks for.

**Found by** running the rename to completion and scanning the tree for the old
name: the only remaining hits were those five binaries and an orphaned
`transitus_wasm.dll` beside them. An orphaned staged binary is now a failure,
verified by planting one and watching the gate refuse it.

---

## A10: conversion breadth · 2026-09-07

Phases 1–4 of the missing-routes plan,
then AVI, DXF and workbook-to-workbook. Common-conversion coverage went from
**10/33 to 21/33**. Three categories are complete; the twelve that remain are
[each blocked by something other than effort](docs/cloudconvert-comparison.md).

What the work actually produced was not mostly routes. It was **eight defects in
shipped code**, every one found by running a real file rather than by reading.

### `HTML → *` wrote an empty file and reported success

Every target. HTML arrived at `mdread::parse` alongside Markdown and plain text,
on the reasoning that CommonMark passes raw HTML through. It does — as
`Event::Html`, which `mdread` drops rather than emit markup as prose — and a
whole HTML document is one raw block. The comment in `run_text` called this "its
prose without its structure". The prose did not survive either.

`htmlread` is the fix: a text extractor, not an HTML parser. One pass, no DOM,
no tree construction, every unknown construct degrading to "the text inside it
is kept".

**Found by** `every_declared_document_route_runs`, written for the defect below
and catching this one on its first run.

### Three routes the table declared and the engine refused

`Pdf -> Html`, `Docx -> Html` and `Odt -> Html` passed routing, spawned the
worker, and were refused there by name — the engine's target lists did not carry
`html`. Worse than a missing route: it survives planning, so it appears in
`openconvert routes` and in ROUTES.md as a capability of the build and fails at
the last step. `Pdf -> Odt` was the mirror case, implemented and never declared.

**The gate:** `every_declared_document_route_runs` drives the shipped CLI and the
real sandboxed workers through every declared document pair — 49 today — and
asserts each writes a file carrying the input's own text. A canary word is why: a
valid PDF of nothing at all passes "the file is not empty", and that is exactly
what `html -> pdf` was producing.

### Every `.mov` was detected as an audio file

`ftypqt  ` was on the `M4a` signature row. So every QuickTime file — H.264 video
beside LPCM audio and a timecode track — was detected as **audio**, offered only
the audio routes, and had its video discarded by whichever one was chosen. That
row's own comment said the brand settles identity; the brand says QuickTime.

**Found by** spiking real `.mov` files before writing the `Mov` row, as the plan
required. The validation result is retained in the private project record.

### Every `mp4 -> mkv` this build wrote was unreadable

`PixelWidth` and `PixelHeight` went directly into `TrackEntry` rather than inside
a `Video` master element. ffmpeg answers "Unknown entry 0xB0" and refuses the
file. The mirror half: the reader looked for them at `TrackEntry` level too, so
the geometry of every real Matroska file read back as zero. Exactly the mistake
the `Audio` element's comment already recorded, one element over.

**Found by** feeding our own output back to ffmpeg. A round trip through our own
reader passed, because both halves were wrong in the same direction — which is
why the test now asserts the **bytes**.

### B-frame video came out in the wrong order under a Class A receipt

`ctts` — the table where MP4 records that a sample is decoded at one time and
shown at another — was read by nothing and written by nothing. On a 277-frame
export the frames came back timed `0, .066, .100, .033` against the source's
`0, .033, .067, .100`. Every frame present, byte-identical, in the wrong order.

`AvSample` now carries a composition offset; `mp4mux` writes `ctts` and the
Matroska writer emits presentation times. Reading it needed one thing the
specification does not admit: the sample file's **version 0** table carries
**negative** offsets, and read as unsigned, `-512` became 279,620 seconds.

### A real `.xlsx` was detected as a plain zip

`zip_member` answered `Docx` for anything holding `[Content_Types].xml` — which
is in every OOXML format, and is not reliably in the sniff window anyway: Excel
writes it **last**, so a real workbook has none in its first 4 KB. Detection now
reads the part-name prefixes (`word/`, `xl/`, `ppt/`), which are entry names and
therefore never compressed.

### A font conversion planned "in-process" and then ran in a worker

The font rows were marked `pure_rust_parser: true`, which is true of the parser
and wrong for what the flag decides. `openconvert plan` said in-process for a
conversion that ran in tx-archive, and a plan the receipt contradicts is the one
thing a plan must never be.

The same flag was described in `openconvert formats` as "C library (sandboxed)"
versus "pure Rust (in-process)" — but zip, tar, gzip and 7z are all pure Rust and
all sandboxed, so the column had been describing the wrong axis all along.

### An AVI's clock counts what `dwSampleSize` says it counts

Zero means chunks; non-zero means bytes. Assuming audio was byte-counted turned a
two-second file into one claiming **420 seconds**.

The same spike settled a question that looked like a defect and was not: AVI
records no display order at all, so `avi -> mp4` carries frames in decode order
with no `ctts` — and ffmpeg's own stream copy produces identical timing and
writes none either. That is now a receipt line rather than a silence.

### What shipped alongside

- **Fonts** — TTF, OTF and WOFF in; WOFF and WOFF2 out, seven Class A repacks in
  the archive worker. `OTF → TTF` is **declined**: both things that ship under
  that name (converting outlines, or renaming the container) are dishonest.
  Verified against fontTools and Chromium, not just our own round trip.
- **Decks, books and workbooks** — PPTX, ODP, EPUB, XLSX and ODS, 22 routes,
  three reader modules, no new engine. Slide order from the presentation, spine
  order from the OPF, DRM refused by name.
- **AVI and DXF** — a RIFF demuxer with Annex-B→AVCC reframing, and a DXF reader
  whose scope is the five entity types 45 real drawings actually contain. 845 of
  1,036 polylines carry a bulge; drawing those straight turns every rounded
  corner into a chamfer.
- **`ODS → XLSX`** — needed cells to keep their **types**: a number written as
  text is one Excel left-aligns and refuses to sum.
- **Archives** — `gzip → zip` and its family, once `StepKind::Extract` could name
  its own ends. It could not, so a two-step chain would have asked for a zip
  twice.

### The recurring theme, again

Three of the eight were hidden by **round-tripping through our own reader**. Two
were hidden by a **gate that passed vacuously**. One shipped because a plan and a
receipt were allowed to disagree.

And one process failure worth recording: `xtask ci` compiles the desktop shell,
the fuzz targets, and runs cargo-deny — three surfaces `cargo test --workspace`
never reaches. Running the individual gates one at a time is **not** running CI,
and doing so let the shell stay broken across three commits while every gate I
ran reported green.
