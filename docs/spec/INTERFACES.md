# OpenConvert — Interface Contracts & Pre-Scaffold Specification

This document is the **single source of truth** for every type, function signature,
file path, and data format that crosses an agent boundary. If your agent creates
or consumes any of these, you MUST match them exactly.

**Orchestrator action required BEFORE any agent starts:** apply every change in
§1 (Pre-Scaffold Changes). After §1 is applied, the API surface is frozen and
agents begin.

---

## Table of contents

1. [Pre-scaffold changes (orchestrator only)](#scaffold)
2. [Shared types reference](#types)
3. [Wire protocol contract](#wire)
4. [Agent file manifests](#files)
5. [Interface: A1 ↔ host](#iface-a1)
6. [Interface: A2 ↔ host](#iface-a2)
7. [Interface: A3 ↔ host](#iface-a3)
8. [Interface: A4 ↔ host](#iface-a4)
9. [Interface: A5 ↔ workers](#iface-a5)
10. [Interface: A6 CLI commands](#iface-a6)
11. [Interface: A7 GUI ↔ backend](#iface-a7)
12. [Interface: A8 site data schema](#iface-a8)
13. [Gap analysis: uncovered items](#gaps)

---

## 1. Pre-scaffold changes (orchestrator only) {#scaffold}

These changes MUST be applied to shared files before any agent starts.
After application, the API surface is FROZEN. Agents code against these types
as-is; they never modify them.

### 1.1 Add `Probed::Document` to openconvert-worker

File: `crates/engines/openconvert-worker/src/lib.rs`

Add to `pub enum Probed`:
```rust
    /// A PDF document, from its header/trailer.
    Document {
        /// Number of pages reported by the container.
        pages: u32,
        /// Whether the document is password-protected.
        encrypted: bool,
    },
```

### 1.2 Add RAW format rows to openconvert-core format table

File: `crates/openconvert-core/src/format.rs`

Add to `FormatId` enum (after `PostScript`):
```rust
    // -- camera raw --
    /// Canon RAW v2. Shares ISO-BMFF with CR3.
    Cr3,
    /// Canon RAW v1 / CRW.
    Cr2,
    /// Nikon Electronic Format.
    Nef,
    /// Sony Alpha Raw.
    Arw,
    /// Adobe Digital Negative.
    Dng,
```

Add to `TABLE` (before the tabular section):
```rust
    // ---- camera raw ----
    fmt_row!(Cr3, Image, "cr3", "cr3", "image/x-canon-cr3", false,
        [Magic::at(4, b"ftypcrx")]),
    fmt_row!(Cr2, Image, "cr2", "cr2", "image/x-canon-cr2", false,
        [Magic::at(0, b"II\x2a\x00\x10\x00\x00\x00CR"),
         Magic::at(0, b"MM\x00\x2a\x10\x00\x00\x00CR")]),
    fmt_row!(Nef, Image, "nef", "nef", "image/x-nikon-nef", false,
        [Magic::at(0, b"II*\x00\x10\x00\x00\x00NEF")]),
    fmt_row!(Arw, Image, "arw", "arw", "image/x-sony-arw", false,
        [Magic::at(0, b"\x00\x00\x00\x08\x00\x00\x00\x00\x53\x4f\x4e\x59")]),
    fmt_row!(Dng, Image, "dng", "dng", "image/x-adobe-dng", false,
        [Magic::at(0, b"II*\x00"), Magic::at(0, b"MM\x00*")]),
```

### 1.3 Add `StepKind::Trim` and `Target::Trim` to openconvert-core

File: `crates/openconvert-core/src/plan.rs`

Add to `pub enum StepKind`:
```rust
    /// Keep only samples within [start_ms, end_ms), snapping to keyframes.
    Trim {
        start_ms: u64,
        end_ms: u64,
    },
```

File: `crates/openconvert-core/src/target.rs`

Add to `pub enum Operation` (if it exists) or create:
```rust
    /// Lossless trim to a time range.
    Trim { start_ms: u64, end_ms: u64 },
    /// Concatenate inputs (batch context only).
    Concat,
```

### 1.4 Add all new route rows to openconvert-core route table

File: `crates/openconvert-core/src/route.rs`, inside `RouteTable::v1()`:

```rust
            // ---- PDF rendering (A1 wires oc-pdf; row exists from day 1) ----
            route!(Pdf -> Png,  B, [T { from: FormatId::Pdf, to: FormatId::Png }], [Requirement::Engine("oc-pdf")]),

            // ---- Camera RAW decode (A2 fills libraw FFI; rows exist from day 1) ----
            route!(Cr3 -> Jpeg, B, [T { from: FormatId::Cr3, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Cr3 -> Png,  B, [T { from: FormatId::Cr3, to: FormatId::Png  }], [Requirement::Engine("oc-images")]),
            route!(Cr2 -> Jpeg, B, [T { from: FormatId::Cr2, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Nef -> Jpeg, B, [T { from: FormatId::Nef, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Arw -> Jpeg, B, [T { from: FormatId::Arw, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),
            route!(Dng -> Jpeg, B, [T { from: FormatId::Dng, to: FormatId::Jpeg }], [Requirement::Engine("oc-images")]),

            // ---- Audio encoding (A3 fills LAME/libopus; rows exist from day 1) ----
            route!(Wav  -> Mp3,  B, [T { from: FormatId::Wav,  to: FormatId::Mp3  }], [Requirement::Engine("oc-audio")]),
            route!(Flac -> Mp3,  B, [T { from: FormatId::Flac, to: FormatId::Mp3  }], [Requirement::Engine("oc-audio")]),

            // ---- Trim (A4 implements the executor; row exists from day 1) ----
            // NOTE: Trim is an Operation target, not a from→to pair.
            // It's handled by pick()'s Operation arm, not the table.
```

### 1.5 Add `oc-pdf` to workspace members

File: root `Cargo.toml`, add to `members`:
```toml
    "crates/engines/oc-pdf",
```

### 1.6 Update engines.rs registry test

File: `crates/openconvert-run/src/engines.rs`, update expected table:
```rust
("oc-pdf", false /* or true on Windows */, cfg!(windows), "pdfium linked on Windows"),
```

### 1.7 Wire Probed::Document through worker_client.rs

File: `crates/openconvert-run/src/worker_client.rs`, in the probe() method's match:
```rust
openconvert_worker::Probed::Document { pages, encrypted } => Properties::Document {
    pages,
    encrypted,
},
```

### Verification of scaffold

```powershell
cargo check --workspace          # exit 0
cargo test --workspace           # zero failures
cargo clippy --workspace --all-targets  # zero warnings
```

---

## 2. Shared types reference {#types}

These types live in `openconvert-core` and are READ-ONLY for all agents.
They are listed here so agents know their exact shape without reading source.

### FormatId (partial — the variants agents care about)

```rust
pub enum FormatId {
    Unknown,
    // images: Png, Jpeg, Webp, Gif, Bmp, Tiff, Heic, Avif, Jxl, Svg
    // audio: Wav, Flac, Mp3, Ogg, Mka
    // video: Mp4, Mkv, Webm
    // documents: Pdf, Docx, Odt, PostScript
    // archives: Zip, Tar, Gzip, SevenZip
    // raw: Cr3, Cr2, Nef, Arw, Dng
    // tabular: Csv, Json
}
```

### MediaKind

```rust
pub enum MediaKind { Image, Audio, Video, Document, Archive, Tabular }
```

### Properties

```rust
pub enum Properties {
    Image { width: u32, height: u32, has_alpha: bool, frames: u32 },
    Audio { duration_ms: u64, channels: u8, sample_rate: u32 },
    Video { duration_ms: u64, width: u32, height: u32,
            video: Option<VideoCodec>, audio: Option<AudioCodec> },
    Document { pages: u32, encrypted: bool },
    Archive { entries: u32, depth: u8, declared_total_bytes: Option<u64> },
    Tabular { rows: u64, columns: u32 },
    None,
}
```

### StepKind

```rust
pub enum StepKind {
    StreamCopy,
    Transcode { from: FormatId, to: FormatId },
    StripMetadata,
    Extract,
    Trim { start_ms: u64, end_ms: u64 },  // added by scaffold §1.3
}
```

### Class

```rust
pub enum Class { A, B, C, D }
// A = lossless, B = lossy deterministic, C = inferred, D = generative
```

### Limits

```rust
pub struct Limits {
    pub wall_time: Duration,
    pub cpu_time: Duration,
    pub memory_bytes: u64,       // default: 1 GiB
    pub output_bytes: u64,       // default: 4 GiB
    pub temp_bytes: u64,
    pub expansion_ratio: u32,    // tripwire only, ≥10_000
    pub decode_pixels: u64,      // default: 256 Mpx
    pub archive_depth: u8,       // default: 32
    pub archive_entries: u32,    // default: 100_000
    pub archive_total_bytes: u64,
}
```

---

## 3. Wire protocol contract {#wire}

The host↔worker protocol uses JSON frames over stdin/stdout.
Defined in `openconvert-worker`. All agents creating workers MUST use this exactly.

### Request (host → worker)

```json
{"Hello"}
{"Probe":{"input_len":12345}}
{"Run":{"input_len":12345,"to":"jpeg","limits":{"decode_pixels":268435456,"memory_bytes":1073741824,"archive_depth":32,"archive_entries":100000,"archive_total_bytes":1073741824},"params":[]}}
```

Content bytes follow the request frame as raw chunked data (not JSON).

`params` is optional (defaulted on deserialize) and carries the operation
parameters a plan step declared — today only `page`, from
`StepKind::RenderPage`. An engine that takes no parameters takes the default
`Engine::run` path; an engine handed a parameter it does not recognise must
answer `Failed`, never silently ignore it.

### Response (worker → host)

```json
{"Ready":{"engine":"oc-pdf","version":"oc-pdf 0.1.0 (pdfium)","mitigations":[["ACG",true],["CIG",false],["AppContainer",true]]}}
{"Properties":{"Image":{"width":1920,"height":1080,"has_alpha":false,"frames":1}}}
{"Properties":{"Document":{"pages":42,"encrypted":false}}}
{"Done":{"output_len":5678,"removed":["all metadata"]}}
{"Failed":{"engine":"oc-pdf","message":"could not decode page 2","retryable":false}}
```

Content bytes follow Done frames only.

### Probed enum (inside Properties response)

```json
{"Image":{"width":1920,"height":1080,"has_alpha":false,"frames":1}}
{"Archive":{"entries":42}}
{"Audio":{"sample_rate":44100,"channels":2,"duration_ms":0}}
{"Document":{"pages":42,"encrypted":false}}
```

---

## 4. Agent file manifests {#files}

Every file each agent creates or modifies. If a file is not listed here,
the agent does NOT create it.

### Agent A1 — PDF worker

| Path | Action | Description |
|---|---|---|
| `crates/engines/oc-pdf/Cargo.toml` | CREATE | Package manifest |
| `crates/engines/oc-pdf/build.rs` | CREATE | Copy pdfium.dll + link |
| `crates/engines/oc-pdf/pdfium.def` | CREATE | Export definitions for .lib generation |
| `crates/engines/oc-pdf/src/main.rs` | CREATE | Entry point + Engine impl |
| `crates/engines/oc-pdf/src/pdfium.rs` | CREATE | Raw FFI bindings |
| `crates/engines/oc-pdf/tests/end_to_end.rs` | CREATE | Protocol tests over pipes |
| `crates/engines/oc-pdf/tests/host_drives_worker.rs` | CREATE | Host-side integration tests |
| `crates/engines/oc-pdf/private implementation notes` | CREATE | Handoff |

Total: 8 files. No other files.

### Agent A2 — RAW + colour

| Path | Action | Description |
|---|---|---|
| `crates/engines/oc-images/src/raw.rs` | CREATE | libraw FFI |
| `crates/engines/oc-images/src/raw_tests.rs` | CREATE | Tests for raw module |
| `crates/engines/oc-images/src/colour.rs` | CREATE | lcms2 FFI |
| `crates/engines/oc-images/private implementation notes` | CREATE | Handoff |

Total: 4 files. Does NOT touch main.rs (orchestrator wires).

### Agent A3 — Audio encoders

| Path | Action | Description |
|---|---|---|
| `crates/engines/oc-audio/src/lame.rs` | CREATE | LAME MP3 encoder FFI |
| `crates/engines/oc-audio/src/lame_tests.rs` | CREATE | Tests |
| `crates/engines/oc-audio/src/opus.rs` | CREATE | Opus encoder + Ogg muxer |
| `crates/engines/oc-audio/src/opus_tests.rs` | CREATE | Tests |
| `crates/engines/oc-audio/private implementation notes` | CREATE | Handoff |

Total: 5 files. Does NOT touch main.rs.

### Agent A4 — Trim/concat

| Path | Action | Description |
|---|---|---|
| `crates/openconvert-run/src/matroska.rs` | MODIFY (append only) | Add trim() and concat() functions |
| `crates/openconvert-run/tests/trim_concat.rs` | CREATE | Tests |
| `private implementation notes` (in openconvert-run/) | CREATE | Handoff |

Total: 1 modified file, 2 new files.

### Agent A5 — Linux hardening

| Path | Action | Description |
|---|---|---|
| `crates/openconvert-os/src/linux.rs` | MODIFY | Fix NetNs reporting, add fallback |
| `crates/openconvert-os/src/spawn_posix.rs` | MODIFY | Add spawn_piped_namespaced() |
| `crates/openconvert-os/tests/linux_escape.rs` | CREATE | Additional escape tests |
| `private implementation notes` (in openconvert-os/) | CREATE | Handoff |

Total: 2 modified files, 2 new files.

### Agent A6 — CLI state layer

| Path | Action | Description |
|---|---|---|
| `crates/openconvert-run/src/state/mod.rs` | CREATE | Module root |
| `crates/openconvert-run/src/state/paths.rs` | CREATE | State directory resolution |
| `crates/openconvert-run/src/state/journal.rs` | CREATE | Batch journal |
| `crates/openconvert-run/src/state/config.rs` | CREATE | Config layering |
| `crates/openconvert-run/src/state/recipe.rs` | CREATE | Recipe save/load |
| `crates/openconvert-run/src/state/mod_test.rs` | CREATE | Integration tests |
| `crates/openconvert/src/cmd/batch.rs` | CREATE | batch command |
| `crates/openconvert/src/cmd/recipe.rs` | CREATE | recipe command |
| `private implementation notes` (in openconvert-run/) | CREATE | Handoff |

Total: 9 files.

### Agent A7 — GUI shell

| Path | Action | Description |
|---|---|---|
| `apps/desktop/src-tauri/Cargo.toml` | CREATE | Tauri app crate |
| `apps/desktop/src-tauri/tauri.conf.json` | CREATE | Strict CSP config |
| `apps/desktop/src-tauri/capabilities/main.json` | CREATE | Deny-by-default permissions |
| `apps/desktop/src-tauri/src/main.rs` | CREATE | Tauri commands wrapping openconvert-run |
| `apps/desktop/src-tauri/build.rs` | CREATE | Tauri build script |
| `apps/desktop/package.json` | CREATE | Svelte 5 + Vite deps |
| `apps/desktop/vite.config.ts` | CREATE | Vite config |
| `apps/desktop/tsconfig.json` | CREATE | TypeScript config |
| `apps/desktop/svelte.config.js` | CREATE | Svelte config |
| `apps/desktop/index.html` | CREATE | Entry HTML |
| `apps/desktop/src/App.svelte` | CREATE | Root component |
| `apps/desktop/src/lib/ipc.ts` | CREATE | Typed IPC client (see §11) |
| `apps/desktop/src/lib/keys.ts` | CREATE | Keyboard shortcuts |
| `apps/desktop/src/lib/theme.ts` | CREATE | Theme toggle |
| `apps/desktop/src/lib/stores/prediction.svelte.ts` | CREATE | Prediction store |
| `apps/desktop/src/lib/stores/job.svelte.ts` | CREATE | Job progress store |
| `apps/desktop/src/lib/stores/undo.svelte.ts` | CREATE | Undo stack |
| `apps/desktop/src/components/DropZone.svelte` | CREATE | Drop target |
| `apps/desktop/src/components/PredictionCard.svelte` | CREATE | Suggestion display |
| `apps/desktop/src/components/AlternateList.svelte` | CREATE | Alternate suggestions |
| `apps/desktop/src/components/PlanTable.svelte` | CREATE | Plan preview table |
| `apps/desktop/src/components/ProgressList.svelte` | CREATE | Progress indicator |
| `apps/desktop/src/components/ReceiptView.svelte` | CREATE | Receipt viewer |
| `apps/desktop/src/components/GroupedDrop.svelte` | CREATE | Mixed-drop grouping |
| `apps/desktop/src/components/ProfileBadge.svelte` | CREATE | Sandbox profile badge |

Total: 25 files.

### Agent A8 — Docs/site/packaging

| Path | Action |
|---|---|
| `site/package.json` | CREATE |
| `site/astro.config.mjs` | CREATE |
| `site/src/layouts/Base.astro` | CREATE |
| `site/src/pages/index.astro` | CREATE |
| `site/src/pages/download.astro` | CREATE |
| `site/src/pages/security.astro` | CREATE |
| `site/src/pages/privacy.astro` | CREATE |
| `site/src/pages/docs.astro` | CREATE |
| `site/src/pages/enterprise.astro` | CREATE |
| `site/src/pages/models.astro` | CREATE |
| `site/src/styles/tokens.css` | CREATE |
| `packaging/winget/openconvert.yaml` | CREATE |
| `packaging/homebrew/openconvert.rb` | CREATE |
| `packaging/flathub/dev.openconvert.openconvert.metainfo.xml` | CREATE |
| `NOTICE` | MODIFY |
| `CHANGELOG.md` | MODIFY |
| `private implementation notes` | CREATE |

Total: 17 files.

---

## 5. Interface: A1 ↔ host {#iface-a1}

### What A1 produces

A1 creates the `oc-pdf` binary. When built, it appears at `target/debug/oc-pdf.exe`.
The host (`openconvert-run`) finds it via `EngineBin::for_media(MediaKind::Document)` → `EngineBin::Pdf` → `"oc-pdf"`.

### What the host expects from oc-pdf

| Protocol message | Expected behaviour |
|---|---|
| `Hello` | Returns `Ready { engine: "oc-pdf", version: "...", mitigations: [...] }` |
| `Probe { input_len }` + content | Returns `Properties(Document { pages: u32, encrypted: bool })` |
| `Run { input_len, to, limits }` + content | Returns `Done { output_len, removed }` followed by content bytes |

Supported output formats: `"png"`, `"jpeg"` only. Anything else → `Failed`.

### What A1 expects from the host

- `spawn_piped_in_container()` or `spawn_piped_limited()` provides stdin/stdout pipes
- AppContainer ACLs granted on oc-pdf.exe AND all sibling *.dll files
- Job Object memory cap set from Limits.memory_bytes

### Shared file changes A1 needs (pre-scaffolded)

- `openconvert-worker`: `Probed::Document` variant (§1.1)
- Workspace Cargo.toml: oc-pdf member (§1.5)
- worker_client.rs: Probed::Document mapping (§1.7)
- Route table: `Pdf -> Png` row (§1.4)

---

## 6. Interface: A2 ↔ host {#iface-a2}

A2 extends oc-images with two new modules. The modules are `#[cfg(feature = "raw")]` gated until the orchestrator enables them.

### What A2 exposes (for main.rs to call)

```rust
// raw.rs
pub struct DecodedRaw {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,      // 3 channels, no alpha
}
pub fn decode_raw(bytes: &[u8]) -> Result<DecodedRaw, String>;

// colour.rs  
pub fn apply_icc_profile(rgba: &mut [u8], icc_data: &[u8]) -> Result<(), String>;
```

### How the orchestrator wires A2 into main.rs

In `run()`, after the HEIC/JXL dispatch:
```rust
// Check if content is a known RAW format by looking for magic signatures.
let raw_formats: [(FormatId, &[&[u8]]); 5] = [
    (FormatId::Cr3, &[b"ftypcrx"]),
    (FormatId::Nef, &[b"NEF"]),
    (FormatId::Arw, &[b"SONY"]),
    // ... etc
];
```
If matched → call `raw::decode_raw()` instead of image-rs.

In `probe()`, similar dispatch returns dimensions from the RAW header.

---

## 7. Interface: A3 ↔ host {#iface-a3}

A3 extends oc-audio with two new encoder modules.

### What A3 exposes (for main.rs to call)

```rust
// lame.rs
pub fn encode_mp3(samples_i16: &[i16], channels: usize, sample_rate: u32) -> Result<Vec<u8>, String>;

// opus.rs
pub fn encode_opus(samples_i16: &[i16], channels: usize, sample_rate: u32) -> Result<Vec<u8>, String>;
```

Both take full-scale i16 interleaved samples (narrowed from symphonia's i32 by the caller).

### How the orchestrator wires A3 into main.rs

In the `run()` match:
```rust
"mp3" => {
    let narrowed: Vec<i16> = interleaved.iter().map(|&v| (v >> 16) as i16).collect();
    let mp3_bytes = lame::encode_mp3(&narrowed, channels, rate)?;
    Ok(Converted { bytes: mp3_bytes, removed: vec!["...".into()] })
}
"ogg" => { /* same pattern with opus::encode_opus */ }
```

Route table already has Wav→Mp3 and Flac→Mp3 rows (removed earlier for honesty, restored when A3 delivers).

---

## 8. Interface: A4 ↔ host {#iface-a4}

A4 adds two pure functions to matroska.rs. They operate on the existing AvGraph type.

### Function signatures

```rust
pub fn trim(graph: &AvGraph, start_ms: u64, end_ms: u64) -> AvGraph;
pub fn concat(a: &AvGraph, b: &AvGraph) -> Result<AvGraph, DemuxError>;
```

### How exec.rs calls them (orchestrator wires)

For `StepKind::Trim { start_ms, end_ms }`:
```rust
let graph = matroska::demux(&current, step.limits.memory_bytes)?;
let trimmed = matroska::trim(&graph, start_ms, end_ms);
let out = matroska::ebml_to_ebml_from_graph(&trimmed)?; // serialises back
```

NOTE: A4 may need to add a `graph_to_ebml()` function that serialises an AvGraph back to EBML. This is different from `ebml_to_ebml()` which works on raw bytes.

---

## 9. Interface: A5 ↔ workers {#iface-a5}

A5 modifies confinement infrastructure. Workers are unaffected (they still self-confine). The changes are:

1. confine_worker() omits "NetNs" entry when unshare fails (instead of including it as false)
2. spawn_posix.rs gains spawn_piped_namespaced() for Full-tier policy enforcement
3. New escape tests verify namespace identity change via /proc/self/ns/net

No cross-agent impact. Other agents see the SAME confine_worker() return shape (just fewer entries on machines without userns).

---

## 10. Interface: A6 CLI commands {#iface-a6}

### Command signatures

```
openconvert batch <directory> -t <format> [--resume] [--manifest <path>] [--dry-run]
openconvert recipe save <name> <file>     # saves current conversion as recipe
openconvert recipe load <name>            # loads and shows plan preview
openconvert config explain <key>          # prints value + which layer set it
```

### Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Conversion failed (some files processed, some didn't) |
| 2 | Usage error / bad arguments |
| 3 | Disk full (resume available) |

### JournalEntry JSONL format

Each line is a self-contained JSON object:
```json
{"source_name":"photo.heic","content_id":"c4d588ae...","plan_hash":12345,"output_name":"photo.jpg","outcome":"Completed"}
```

Malformed lines are SKIPPED on read (never fatal). Truncated last line tolerated.

### Recipe TOML format

```toml
name = "heic-to-jpeg"
created = "2026-08-23"
plan_hash = 1234567890

[request]
input_format = "heic"
target_format = "jpeg"

[request.limits]
memory_bytes = 1073741824
decode_pixels = 268435456
```

On load: re-route with current machine's environment. If plan_hash differs from what route() produces now, print warning and ask for confirmation.

---

## 11. Interface: A7 GUI ↔ backend {#iface-a7}

**This section is the source of truth for the desktop IPC surface**, rewritten
2026-08-24 to match `apps/desktop/src-tauri/src/main.rs` and
`apps/desktop/src/lib/ipc.ts` field for field. The two files are edited
together or not at all: every Rust struct carries `#[serde(rename_all =
"camelCase")]`, and a field added on one side and not the other is silent
data loss.

### 11.1 Tauri commands

| Command | Rust signature | Returns |
|---|---|---|
| `probe_file` | `(path: String) -> Result<ProbeResult, String>` | What one file is |
| `get_plan` | `(paths: Vec<String>, targetFormat: String) -> Result<PlanPreview, String>` | Plan preview (`route()` without `execute()`) |
| `suggest_targets` | `(paths: Vec<String>) -> Result<Vec<Prediction>, String>` | Ranked targets per detected-format group |
| `convert_batch` | `(paths: Vec<String>, targetFormat: String) -> Result<Vec<ConversionResult>, String>` | Execute one batch |
| `cancel_all` | `() -> Result<(), String>` | Stop after the file currently running |
| `undo_last` | `() -> Result<(), String>` | Remove last batch's outputs, restore replaced originals |
| `get_config` | `() -> Result<Config, String>` | Effective configuration, defaults filled in |
| `set_config` | `(patch: ConfigPatch) -> Result<Config, String>` | Validate + merge + persist, returns effective |
| `list_history` | `(limit: Option<usize>) -> Result<Vec<HistoryEntry>, String>` | Past conversions from the journals, newest first |
| `wipe_history` | `() -> Result<usize, String>` | Delete journal files; returns count |
| `list_receipts` | `(limit: Option<usize>) -> Result<Vec<ReceiptEntry>, String>` | App-data receipt store, newest first |
| `receipts_size` | `() -> Result<u64, String>` | Bytes currently held by the store |
| `delete_receipt` | `(id: String) -> Result<(), String>` | Remove one stored receipt (immediate) |
| `delete_all_receipts` | `() -> Result<u64, String>` | Empty the store; returns bytes reclaimed |
| `list_models` | `() -> Result<Vec<ModelInfo>, String>` | Declared models with per-user state |
| `download_model` | `(id: String) -> Result<(), String>` | Fetch weights (refuses until A10's downloader lands) |
| `set_model_enabled` | `(id: String, enabled: bool) -> Result<(), String>` | Persist an enable/disable override |
| `delete_model` | `(id: String) -> Result<(), String>` | Reclaim a downloaded model ("Reclaim N MB") |
| `list_tools` | `() -> Result<Vec<ToolDescriptor>, String>` | What the Tools menu may show |
| `run_tool` | `(id: String, paths: Vec<String>, params: JsonValue) -> Result<Vec<ConversionResult>, String>` | Run a tool through the normal pipeline |
| `panic_stop` | `() -> Result<(), String>` | Cancel everything queued (see limits below) |
| `pick_files` | `() -> Result<Option<Vec<String>>, String>` | Native open dialog, host-side via `rfd`; `null` on cancel |

Naming note: Tauri v2 maps Rust snake_case arguments from camelCase keys, so
the frontend sends `{ paths, targetFormat }` to `get_plan`/`convert_batch`.
(The pre-rewrite snippet said `{ paths, target }` against a `target_format`
Rust parameter — that was the drift.)

### 11.2 Types (must match Rust serde output EXACTLY)

```typescript
// Mirrors src-tauri/src/main.rs. Rust Option<T> serialises as T | null.

interface ProbeResult {
  path: string;            // as given; addressing only, never rendered
  fileName: string;        // through DisplayName; safe to render
  detected: string;
  declared: string | null;
  mismatched: boolean;
  polyglot: boolean;
  nameAltered: boolean;    // rendering had to escape/truncate something
  inputBytes: number;
  properties: PropertiesJson | null;   // Properties::None => null, not {}
}

interface PropertiesJson {
  kind: "image" | "audio" | "video" | "document" | "archive" | "tabular";
  // Every field below is Option in Rust: number | null etc., never absent.
  width; height; hasAlpha; frames; durationMs; channels; sampleRate;
  videoCodec; audioCodec; pages; encrypted; entries; depth; rows; columns;
}

interface PlanPreview {
  executable: boolean;
  steps: PlanStep[];
  warnings: Warning[];
  estimatedDurationMs: number;   // 0 means NOT DETERMINED, never instant
  totalInputBytes: number;
}

interface PlanStep {
  fileName: string;              // per-file attribution (multi-file drops)
  kind: string;                  // "Transcode { from: heic, to: jpeg }"
  className: "A" | "B" | "C" | "D";
  isolation: string;             // "in-process" | "sandboxed (elevated)"
  limitsSummary: string;
  engineName: string;
}

interface Warning {
  fileName: string;
  blocking: boolean;
  message: string;               // rendered by describe(), display verbatim
}

interface Suggestion {
  target: string;                // pass back as targetFormat
  score: number;                 // 0.0..=1.0
  className: string;             // "B (lossy, standard)" — worst class
  armed: boolean;                // confidence >= backend threshold (predict::ARM_THRESHOLD);
                                 // Enter runs ONLY armed suggestions — below it,
                                 // open the chooser instead of converting.
}

// The thresholds themselves are owned by A10 in openconvert-core::predict and
// ship with every probe/suggest response so the shell NEVER hard-codes them:
//
//   ARM_THRESHOLD     = 0.85   // >= arms the top suggestion
//   CHOOSER_THRESHOLD = 0.60   // < shows a chooser instead of a ranked lead
//
// The backend computes `armed` (above); the chooser band is score <
// CHOOSER_THRESHOLD, which the shell reads from `suggestThresholds` on the
// response envelope rather than from its own constants:
//
// interface SuggestThresholds { arm: number; chooser: number }

interface Prediction {
  input: string;                 // detected format id of the group
  kind: PropertiesJson["kind"];  // group glyph
  paths: string[];               // members, drop order preserved
  suggestions: Suggestion[];     // best first; empty when nothing routes
  why: string;                   // one line, rendered at text-secondary
}

interface ConversionResult {
  fileName: string;
  outputPath: string;            // empty when failed
  receiptPath: string;           // empty when failed
  success: boolean;
  errorMessage: string | null;
  durationMs: number;
  inputBytes: number;
  outputBytes: number;
  classApplied: string;          // "A (lossless)" … "—" when failed
  removedMetadata: string[];     // itemised; empty list renders "Nothing"
  contentId: string;             // Blake3 hex of bytes converted; "" if failed
  receiptDetail: ReceiptDetail | null;
}

interface ReceiptDetail {         // straight from openconvert_run::receipt::Receipt
  version: number;
  tool: string;
  contentId: string;             // input identity
  detected: string;
  declaredMismatch: string | null;
  class: string | null;
  outputName: string;            // basename only — receipts never carry paths
  outputBytes: number;
  steps: Array<{
    kind: string;
    class: string;
    engine: string;              // name only; versions absent this build
    isolation: string;           // what ENGAGED (read back), not requested
    limitsSummary: string;
  }>;
}

interface ProgressEvent {        // event "conversion-progress", per file
  index: number;                 // 0-based into convert_batch's paths
  total: number;
  fileName: string;
  phase: "started" | "done" | "failed" | "cancelled";
}
```

### 11.3 Settings, history, receipts, models

```typescript
interface Config {
  defaultFormat: string | null;  quality: number | null;  maxParallel: number | null;
  theme: "system" | "light" | "dark";          // default system
  density: "regular" | "compact";              // default regular
  networkPosture: "offline" | "sealed";        // default offline; can only tighten
  metadataPolicy: "share" | "keep";            // default share
  updateChecks: boolean;                       // default true
  diagnostics: boolean;                        // default false
  confirmOverwrite: boolean;                   // default true
  writeReceipts: boolean;                      // default true — see caveat below
  outputDestination: "same_folder" | "desktop" | "replace_source";  // default same_folder
  namingTemplate: string;                      // default "{name}.{ext}"; tokens
                                               // {name} {ext} {date} {index};
                                               // {name} REQUIRED; no path separators
}
type ConfigPatch = Partial<Config>;  // absent keys unchanged; no nullable fields

interface HistoryEntry {
  sourceName: string;  outputName: string;  contentId: string;
  outcome: "completed" | "failed" | "skipped";
  reason: string | null;       // failures only
  whenSecs: number;            // journal FILE mtime — the format carries no
                               // per-line clock, so this is batch-granular
}

interface ReceiptEntry {
  id: string;                  // store id (filename stem); pass to delete_receipt
  date: string;                // ISO; from the record when it carries one,
                               // file mtime otherwise
  sourceName: string | null;   // what produced the output, when recorded
  outputName: string;
  outputBytes: number;
}

interface ModelInfo {
  id: string;  title: string;  purpose: string;
  sizeBytes: number;
  licence: string;  downloaded: boolean;  enabled: boolean;
}
```

**`write_receipts` governs the desktop receipt database and receipt UI.** The
GUI asks the runner for an in-memory receipt and never creates a temporary
sidecar. A database write failure fails the conversion and compensates its
filesystem work, including restoring a replaced original. When the switch is
off, no database row is written. The CLI is unaffected and keeps its ordinary
`<output>.receipt.json` contract.

**Receipt database** — desktop-owned SQLite at
`<state_dir>/receipts/receipts.sqlite3`, initialized with a versioned schema,
rollback journaling, full synchronous writes and a busy timeout. Rows have
unique generated ids and indexed conversion timestamps; list/count/delete/
delete-all are SQL operations. Wiping the database can never touch an output
or a CLI sidecar. No CSV migration is performed because this supported revision
assumes no CSV receipt store exists.

**Window geometry** persists to UserConfig keys `windowX/Y/W/H` on close and
restores at startup, best-effort.

### 11.4 Tools contract

The split rule: **if the only choice is the output format, it belongs to the
main screen's alternates list. If the user must supply anything else — a trim
range, a second file, a strength — it is a tool.** Model-powered conversions
that change media type (OCR, transcription) ship as ordinary routes;
model-powered same-type effects (background removal) are tools.

```typescript
interface ToolParam {
  id: string;          // stable, as the handler reads it back
  title: string;       // rendered label
  kind: "number" | "text";   // enough for v1's generic form
  required: boolean;
}

interface ToolDescriptor {
  id: string;                    // stable, passed back to run_tool ("video-trim")
  category: "image" | "pdf" | "audio" | "video";   // lowercase on the wire
  title: string;                 // "Trim clip"
  available: boolean;            // handler exists AND engines present — measured NOW
  unavailableReason: string | null;  // required when !available; tooltip text
  multiInput: boolean;           // multi-input assembly (join) vs single-input edit
  params: ToolParam[];           // rendered generically by the dialog
}
```

Rules both sides hold to:

- **The menu renders exclusively from `list_tools()`** — zero hardcoded
  entries. A category with no available tools renders nothing; an unavailable
  tool renders disabled with `unavailableReason` as its tooltip.
- **The registry is A10's** (`openconvert_run::tools`): descriptors, handlers,
  availability. The desktop command maps their structs onto this camelCase
  wire shape field-for-field; nothing else re-spells them.
- **Tools are not a second-class path**: `run_tool` emits the same
  `conversion-progress` events, writes the same receipts, and feeds the same
  undo record as `convert_batch`. Multi-input tools make one call with every
  path and produce one output + receipt; per-input tools loop with
  cancellation checked between files.
- Params cross as a JSON object (`{ startMs: 0, endMs: 4000 }`); the shell
  stringifies for the handler signature.

Ship-time contents per A10's registry: Video ▸ Trim clip… / Join clips…
(registered today), Images ▸ Remove background… and Audio ▸ Trim audio… land
with oc-ai / optional audio trim. PDF stays empty until qpdf lands.

### 11.5 Panic

`panic_stop` cancels every queued batch **at the boundary batches actually
have — between files**, exactly like `cancel_all`. It does not claim more:
workers are job-scoped (each batch owns its pool privately), so a worker
already decoding stops at its own wall/memory limits, not by handle. There is
no shared temp root to wipe — the sweep rule (`03` §13) forbids one — and
nothing outside our state directory is touched. The chord is
**Ctrl+Shift+P**, handled in the webview: the global-shortcut plugin would
need a new entry on the gated capability allowlist for no gain, since the
window is where the user already is.

### 11.6 File ingress and the capability set

Drop paths come from Tauri's `onDragDropEvent`. The Browse button and Ctrl+O
call `pick_files`, which opens the platform dialog **host-side via `rfd`** —
deliberately not `tauri-plugin-dialog`: the plugin would need a new entry on
the gated capability allowlist and a webview permission for exactly what a
host-process call already provides. The capability set remains
`core:default` and nothing else, scoped to the `main` window.

### 11.7 Event flow

```
User drops files / picks files (Ctrl+O or Browse)
  → invoke("probe_file") per file + invoke("suggest_targets") for the drop
  → cards grouped by detected type, ranked suggestions with "why" lines
  → user selects target (1–9 / click); Enter runs ARMED suggestions only,
    otherwise opens the chooser instead of guessing badly
  → invoke("get_plan", { paths, targetFormat }) per group
  → PlanPreview table: class chips + sandbox badges + limits
  → Enter / Convert → invoke("convert_batch")
  → "conversion-progress" events per file drive the top hairline bar
  → Vec<ConversionResult> renders receipts (confinement, engine, input ID)
  → undo offered 60 s; after expiry the record lives in History
```

Settings read/write through `get_config`/`set_config`; history through
`list_history`/`wipe_history`; the receipt store through
`list_receipts`/`delete_receipt`/`delete_all_receipts`.

### 11.8 Error handling

Every command returns `Result<T, String>`. The `Err(String)` is always a
human-readable message safe to display (passed through DisplayName
sanitisation). The frontend NEVER parses error strings for control flow — it
displays them verbatim in a banner, or inline under the control that caused
one (settings validation).

### 11.9 CSP configuration (tauri.conf.json)

```json
{
  "security": {
    "csp": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self' ipc: http://ipc.localhost; object-src 'none'; base-uri 'self'; form-action 'none'; frame-ancestors 'none'"
  }
}
```

No remote origins. No unsafe-inline. No unsafe-eval. Every asset local —
including InterVariable.woff2, bundled under `public/fonts/` and asserted by
an xtask desktop gate (exists AND referenced). Dynamic width on the progress
hairline uses Svelte's `style:` directive, which applies through the CSSOM
and therefore survives `style-src 'self'`.

---

## 12. Interface: A8 site data schema {#iface-a8}

The site generates pages from data the binary already carries. This section defines the JSON schemas used for generation.

### formats.json (generated from format table)

```json
[
  {
    "name": "heic",
    "kind": "image",
    "extension": "heic",
    "mediaType": "image/heic",
    "pureRustParser": false,
    "routesFrom": [
      {"to": "jpeg", "class": "B", "requires": ["oc-images"]}
    ],
    "routesTo": []
  }
]
```

Generated by a build-time script that reads the format table (via a small Rust helper or manually maintained until automation lands).

### models.json (generated from models.toml)

Same structure as models.toml but in JSON for the Astro site to consume.

### privacy.md shared source

One Markdown file referenced by README, site, and app settings screen. Build gate: if any copy diverges, CI fails.

---

## 13. Gap analysis: uncovered items {#gaps}

Items from 08-EXECUTION-PLAN that NO agent currently covers:

| Item | Phase | Why uncovered | Recommendation |
|---|---|---|---|
| Prediction engine wiring (weeks 39–40) | 8 | Requires history data that doesn't exist until users convert files | Post-1.0; five signals are implemented as a pure function but not wired to persistent history |
| Code signing / notarization | 9 | Requires certificates and Apple Developer account | Orchestrator responsibility; A8 prepares unsigned manifests |
| SBOM generation | 9 | Needs `cargo sbom` tool | A8 can generate using `cargo cyclonedx` if available |
| Reproducible builds | 9 | Needs fixed Docker image + SOURCE_DATE_EPOCH | Post-M3; documented in 09 §9 |
| OSS-Fuzz enrollment | Ongoing | Requires project to be public first | After launch |
| SOC 2 / security audit | Enterprise | $25–60K external cost | Not a coding task |

These are NOT coding tasks for agents. They are operational/orchestration tasks that happen after M3.


### Workspace review commands (September 2026)

- `preview_audio({path}) -> string`: bounded WAV data URI decoded by the confined audio worker; no file is written. The CSP permits `media-src self data:`.
- `save_text_copy({path,text,extension}) -> string`: creates a uniquely named edited copy beside the source, up to 4 MiB; extension is txt, srt, or vtt. Existing files are never overwritten.
- `save_image_corrections({path,original,strokes}) -> string`: applies normalized erase/restore brush strokes in the image worker, writes a corrected PNG and respects the receipt setting.
- `read_text_output` accepts bounded txt, md, srt, and vtt outputs.
- `audio-transcribe` accepts `format=vtt` to decode timestamp tokens; the default remains plain text. SRT conversion preserves those model timestamps.
- `image-ocr` accepts `format=pdf` to preserve the scan image with an invisible Unicode-mapped text layer.
- `pdf-compose` accepts `stamps` JSON (up to 32 placements with image, page, x, y, width). Images cross the worker boundary as extra input bytes, never worker-readable paths.
- Native tool descriptors serialize the complete registry parameter type including options/default/min/max/step. Desktop fixtures must not hide missing native fields.
