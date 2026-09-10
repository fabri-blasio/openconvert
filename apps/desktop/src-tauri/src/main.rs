//! The OpenConvert desktop backend.
//!
//! **The state machine lives here, not in Svelte.** The webview is outside the
//! trust boundary (`09` §1), so it renders what it is given and sends events;
//! every decision — what a file is, what would happen, what did happen — is
//! made by the library crates behind these commands.
//!
//! Each command is a thin, typed wrapper over `openconvert-run`. There is
//! deliberately no logic here that the CLI does not also run: `probe_file`,
//! `get_plan` and `convert_batch` follow the same detect → probe → route →
//! execute path as `cmd_convert`, so the GUI and the CLI cannot disagree about
//! a file.
//!
//! # What crosses the boundary
//!
//! Only the JSON below. No paths are handed to the webview that it could feed
//! back into a filesystem API, because there is no filesystem API on the other
//! side: the capability set grants `core:default` and nothing else, and the
//! `fs`, `shell`, `process`, `http` and `opener` plugins are refused by a gate
//! in `xtask/src/desktop.rs`.
//!
//! Every string rendered by the UI that came from a filename, an archive member
//! or an engine's error text is passed through
//! [`openconvert_sandbox::display::DisplayName`] first. Svelte escapes markup on
//! its own; it does **not** neutralise a bidi override, and `09` §3 names "a
//! crafted filename rendered into the UI" as a concrete threat.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
// House style, and the same claim the library crates make. Nothing in a shell
// over safe libraries needs it.
#![forbid(unsafe_code)]
// The imperative shell: paths, files and per-user state are this crate's job,
// so the CORE's purity lints (clippy.toml) relax here, at the crate boundary.
// The bypass scan in xtask/src/lint.rs gates openconvert-core only — on purpose:
// that gate exists to keep the functional core honest, not the shells around
// it. Every use below is still inside our own state directory or a path the
// user handed us.
#![allow(clippy::disallowed_types, clippy::disallowed_methods)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};

use openconvert_core::environment::Environment;
use openconvert_core::facts::Properties;
use openconvert_core::format::{FormatId, MediaKind, TABLE};
use openconvert_core::isolation::Isolation;
use openconvert_core::plan::{Class, Plan, Step, StepKind, Warning};
use openconvert_core::policy::Policy;
use openconvert_core::predict::{self, FolderCategory, Signals};
use openconvert_core::route::{route, RouteTable};
use openconvert_core::target::Target;
use openconvert_run::state::config::UserConfig;
use openconvert_run::state::journal::{JournalEntry, Outcome as JournalOutcome};
use openconvert_run::state::paths;
use openconvert_sandbox::argv::EngineBin;
use openconvert_sandbox::display::DisplayName;

mod receipts_db;
mod store;

static REPLACEMENT_ID: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// The wire types.
//
// These are the whole contract with the frontend (INTERFACES.md §11). Every
// field is mirrored in `src/lib/ipc.ts`, and `rename_all = "camelCase"` is what
// makes the two spellings line up. A field added here without a field added
// there is silent data loss, which is why the TypeScript file says so too.
// ---------------------------------------------------------------------------

/// Format-specific facts, flattened.
///
/// One struct rather than a tagged union: `Properties` is an enum in Rust, and
/// a discriminated union in TypeScript would make every read site a narrowing
/// exercise for four fields the UI shows in one metadata line. `kind` carries
/// the discriminant and the rest are `null` where they do not apply.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PropertiesJson {
    kind: String,
    width: Option<u32>,
    height: Option<u32>,
    has_alpha: Option<bool>,
    frames: Option<u32>,
    duration_ms: Option<u64>,
    channels: Option<u8>,
    sample_rate: Option<u32>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    pages: Option<u32>,
    encrypted: Option<bool>,
    entries: Option<u32>,
    depth: Option<u8>,
    rows: Option<u64>,
    columns: Option<u32>,
}

/// What one file is.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProbeResult {
    /// The path exactly as the frontend gave it, echoed back so the UI can
    /// address this file again without holding a second copy of the list.
    ///
    /// Never rendered — `file_name` is the rendered one.
    path: String,
    file_name: String,
    detected: String,
    declared: Option<String>,
    mismatched: bool,
    polyglot: bool,
    /// Whether rendering `file_name` had to escape or truncate anything.
    ///
    /// Surfaced rather than hidden: a user shown `invoice\u{202e}fdp.exe` with
    /// no explanation concludes the tool is broken, and one told the name
    /// carries a text-direction override concludes the file is.
    name_altered: bool,
    input_bytes: u64,
    properties: Option<PropertiesJson>,
}

/// One step of a plan, rendered.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanStep {
    /// Which input this step belongs to, for a multi-file preview.
    file_name: String,
    /// `"Transcode { from: heic, to: jpeg }"` — the step's own Debug.
    kind: String,
    class_name: String,
    /// `"in-process"` or `"sandboxed (elevated)"`.
    isolation: String,
    /// `"1024 MiB memory, 256 Mpx decode, 120s wall"`.
    limits_summary: String,
    engine_name: String,
}

/// Something the user should know before running.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WarningJson {
    file_name: String,
    blocking: bool,
    message: String,
}

/// What *would* happen. `route()` without `execute()`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlanPreview {
    executable: bool,
    steps: Vec<PlanStep>,
    warnings: Vec<WarningJson>,
    /// **0 means "not determined"**, never "instant".
    ///
    /// Nothing in this build estimates conversion time, and a fabricated number
    /// on the one screen whose job is to tell the truth about what will happen
    /// is worse than an absent one. `Properties::Video::duration_ms` uses the
    /// same convention for the same reason.
    estimated_duration_ms: u64,
    total_input_bytes: u64,
}

/// What happened to one file.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversionResult {
    file_name: String,
    output_path: String,
    receipt_path: String,
    success: bool,
    error_message: Option<String>,
    duration_ms: u64,
    input_bytes: u64,
    output_bytes: u64,
    /// `"A (lossless)"`, `"B (lossy, standard)"`, …
    class_applied: String,
    removed_metadata: Vec<String>,
    /// The receipt's own account of the run, when there is one. This is the
    /// data SR-11 demands be checkable — engines, isolation per step, input
    /// identity, declared-vs-detected — carried straight from the receipt
    /// struct rather than re-derived by the UI.
    content_id: String,
    receipt_detail: Option<ReceiptDetailJson>,
}

/// One executed step as the receipt recorded it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct StepReceiptJson {
    kind: String,
    class: String,
    /// Engine name as recorded. Versions are absent from this build's
    /// receipts; when the receipt carries them the UI will render them.
    engine: String,
    isolation: String,
    limits_summary: String,
}

/// The receipt, for the done screen.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReceiptDetailJson {
    version: u32,
    tool: String,
    /// Blake3 hex of the bytes actually converted — the input's identity.
    content_id: String,
    detected: String,
    /// What the filename claimed, when it disagreed with the content.
    declared_mismatch: Option<String>,
    class: Option<String>,
    output_name: String,
    output_bytes: u64,
    steps: Vec<StepReceiptJson>,
}

/// One candidate target, scored.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SuggestionJson {
    /// The format name to pass back as `targetFormat`.
    target: String,
    /// `0.0..=1.0`.
    score: f32,
    /// The worst class the best route for this pair costs.
    class_name: String,
    /// Whether this one is confident enough to arm the Enter key.
    armed: bool,
}

/// What to offer for one group of files.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Prediction {
    /// The input format these files share.
    input: String,
    /// The media kind, for the group's glyph.
    kind: String,
    /// The paths in this group, in the order they were given.
    paths: Vec<String>,
    /// Best first. Empty when nothing is routable from this input.
    suggestions: Vec<SuggestionJson>,
    /// One line saying *why* the top suggestion is the top suggestion.
    why: String,
}

/// A rendered preview of one file.
///
/// The bytes are a PNG **this app produced in a confined worker**, never the
/// user's original file. That is what lets the webview decode it at all: the
/// CSP permits `img-src 'self' data:`, and the thing behind the data URI is our
/// output rather than attacker-chosen input.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FilePreview {
    data_uri: String,
    width: u32,
    height: u32,
    page_count: u32,
    page: u32,
    /// The page's physical size in PDF points (1/72 in), when the renderer
    /// reported one. `None` for images, and for documents until `oc-pdf`
    /// returns the MediaBox — the interface then shows the aspect it measured
    /// rather than naming a paper size it cannot know.
    page_geometry_error: Option<String>,
    page_width_pt: Option<f32>,
    page_height_pt: Option<f32>,
    page_rotation: Option<i64>,
    original_bytes: Option<u64>,
    compressed_bytes: Option<u64>,
}

/// Progress, emitted per file during `convert_batch`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    /// 0-based index into the `paths` given to `convert_batch`.
    index: usize,
    total: usize,
    file_name: String,
    /// `"started"`, `"done"`, `"failed"` or `"cancelled"`.
    phase: &'static str,
}

// ---------------------------------------------------------------------------
// Settings — the config surface (INTERFACES.md §11).
// ---------------------------------------------------------------------------

/// What the interface showed for one file, and what the user did with it.
///
/// **Sent by the shell rather than recomputed here**, because the question
/// this answers is "did the user accept what they were shown". A backend that
/// re-ranked at journal time could disagree with the screen -- history moves,
/// files appear beside other files -- and would then record its own later
/// opinion as if it were the user's choice.
///
/// Untrusted, like the journal it lands in: these values can only ever change
/// a future ranking, never what a conversion is permitted to do.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChoiceJson {
    /// The file this describes, matched against the batch's paths.
    path: String,
    /// The target the app ranked first, as a format name.
    suggested: String,
    /// That suggestion's score.
    suggested_score: f32,
    /// Where the chosen target sat in the ranking; -1 if it was not in it.
    chosen_rank: i32,
}

/// The effective configuration, with defaults filled in.
///
/// The frontend never applies a default of its own: what ships here is what
/// the app means, so a setting that reads as ON is ON everywhere.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigJson {
    /// `"balanced" | "isolated"`.
    worker_reuse: String,
    /// Confine every step that has a worker, including the pure-Rust ones.
    force_sandbox: bool,
    /// Whether the first-run AI chooser has been answered.
    ai_setup_done: bool,
    /// True when a policy layer pinned `isolated`, so the control is shown
    /// disabled rather than accepting a change the ratchet would discard.
    worker_reuse_locked: bool,
    default_format: Option<String>,
    theme: String,
    /// Whether conversions may use a GPU where one measurably helps.
    use_gpu: bool,
    /// Memory one conversion may use, in MB. The effective value, after the
    /// policy's bounds -- so a hand-edited file out of range shows what will
    /// actually apply rather than what it asked for.
    worker_memory_mb: u64,
    /// The largest this build accepts, so the screen can say so.
    worker_memory_max_mb: u64,
    /// Re-fetch superseded model artifacts without asking. Default on.
    model_auto_update: bool,
    write_receipts: bool,
    output_destination: String,
    naming_template: String,
}

impl From<&UserConfig> for ConfigJson {
    fn from(c: &UserConfig) -> Self {
        // The effective value, not the stored one: the ratchet may have raised
        // it above what this file asks for, and the screen must show what will
        // actually happen.
        let effective = policy_from(c).worker_reuse();
        Self {
            force_sandbox: c.force_sandbox.unwrap_or(false),
            ai_setup_done: c.ai_setup_done.unwrap_or(false),
            worker_reuse: match effective {
                openconvert_core::policy::WorkerReuse::Isolated => "isolated".to_string(),
                openconvert_core::policy::WorkerReuse::Balanced => "balanced".to_string(),
            },
            worker_reuse_locked: effective == openconvert_core::policy::WorkerReuse::Isolated
                && c.worker_reuse.as_deref() != Some("isolated"),
            default_format: c.default_format.clone(),
            theme: enum_or(&c.theme, &["system", "light", "dark"], "system"),
            // The effective value, not the stored one: a policy layer may have
            // withdrawn the GPU, and the screen must show what will happen.
            use_gpu: policy_from(c).base_limits().use_gpu,
            worker_memory_mb: policy_from(c).base_limits().memory_bytes >> 20,
            worker_memory_max_mb: openconvert_core::policy::MAX_WORKER_MEMORY >> 20,
            model_auto_update: c.model_auto_update.unwrap_or(true),
            write_receipts: c.write_receipts.unwrap_or(true),
            output_destination: enum_or(
                &c.output_destination,
                &["same_folder", "downloads", "desktop", "replace_source"],
                "same_folder",
            ),
            naming_template: valid_template_or(&c.naming_template, DEFAULT_NAMING_TEMPLATE),
        }
    }
}

/// What the user may change in one call. Absent keys mean unchanged; there
/// are deliberately no nullable fields — a setting is either left alone or
/// set to a value.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigPatch {
    worker_reuse: Option<String>,
    force_sandbox: Option<bool>,
    ai_setup_done: Option<bool>,
    theme: Option<String>,
    use_gpu: Option<bool>,
    worker_memory_mb: Option<u64>,
    model_auto_update: Option<bool>,
    write_receipts: Option<bool>,
    output_destination: Option<String>,
    naming_template: Option<String>,
}

/// The default output-name template.
const DEFAULT_NAMING_TEMPLATE: &str = "{name}.{ext}";

fn enum_or(value: &Option<String>, allowed: &[&str], default: &str) -> String {
    match value {
        Some(v) if allowed.contains(&v.as_str()) => v.clone(),
        // A corrupt or hand-edited key degrades to the default, like every
        // other untrusted read under the state directory.
        _ => default.to_string(),
    }
}

fn valid_template_or(value: &Option<String>, default: &str) -> String {
    match value {
        Some(t) if validate_naming_template(t).is_ok() => t.clone(),
        _ => default.to_string(),
    }
}

/// The naming template: tokens validated by the canonical name module
/// (`openconvert_run::naming`), plus this shell's own policy — `{name}` is
/// required, because an output that loses its input's stem loses the thing
/// the user recognises it by.
fn validate_naming_template(template: &str) -> Result<(), String> {
    openconvert_run::naming::validate_template(template).map_err(|e| e.to_string())?;
    if template.trim().is_empty() {
        return Err("the naming template cannot be empty".to_string());
    }
    if template.contains(['/', '\\', ':', '*']) || template.contains("..") {
        return Err(
            "the naming template must be a plain file name — no path separators".to_string(),
        );
    }
    if !template.contains("{name}") {
        return Err(
            "the template needs {name} — without it the output loses the \
                    original's name"
                .to_string(),
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

/// A file this process created and may therefore remove again.
#[derive(Debug, Clone)]
struct UndoItem {
    output: PathBuf,
    /// A disposable sidecar created by a CLI-style caller. Desktop conversions
    /// persist directly to SQLite, which is history and must survive Undo.
    receipt: PathBuf,
    /// Set when a replace-originals run moved the input aside:
    /// `(quarantine path, original path)`, so Undo can put it back. Without
    /// this a destructive setting would be a trap.
    replaced_from: Option<(PathBuf, PathBuf)>,
}

/// Everything the commands share.
struct AppState {
    /// Set by `cancel_all`, cleared at the start of each batch.
    ///
    /// Cancellation is checked **between files**, which is the honest
    /// granularity: a worker mid-decode is killed by its own limits, not by a
    /// flag the host polls, and claiming finer resolution than that in the UI
    /// would be a promise this cannot keep.
    cancel: Arc<AtomicBool>,
    /// The outputs of the most recent batch, and only those.
    ///
    /// `undo_last` deletes exactly these paths. It never takes a path from the
    /// webview, which is the whole point: the one deletion path in this process
    /// can only ever name a file this process created moments earlier.
    undo: Mutex<Vec<UndoItem>>,
}

impl AppState {
    fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            undo: Mutex::new(Vec::new()),
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// What one file actually is.
///
/// Runs phase 1 (`detect`) **and phase 2** (`probe`). The phase-2 call is not
/// optional decoration: `route::limits_for` narrows `decode_pixels` from the
/// properties, so a plan built on `Properties::None` runs every conversion at
/// the full 256 Mpx ceiling — the same defect the CLI carried until it started
/// probing here too.
#[tauri::command]
async fn probe_file(path: String) -> Result<ProbeResult, String> {
    blocking(move || probe_one(&PathBuf::from(path))).await
}

/// What *would* happen, for a whole drop, without doing any of it.
///
/// The plan preview is not a feature layered on top of routing; it is
/// `route()` with `execute()` never called.
#[tauri::command]
async fn get_plan(paths: Vec<String>, target_format: String) -> Result<PlanPreview, String> {
    blocking(move || Ok(plan_for(&paths, &target_format))).await
}

/// Rank the targets worth offering for a drop, grouped by input format.
///
/// The ranking is `openconvert_core::predict::rank` over the *routable* targets
/// for each input — so a suggestion the UI shows is always one `route()` would
/// accept. The signals come from the batch journals on disk, which is where the
/// "you have done this 14 times" line gets its 14.
#[tauri::command]
async fn suggest_targets(paths: Vec<String>) -> Result<Vec<Prediction>, String> {
    blocking(move || Ok(predictions_for(&paths))).await
}

/// What a drop actually contains.
///
/// A dropped **folder** arrives as one path, and everything downstream expects
/// files: without this the whole drop failed with "could not detect", which is
/// a poor answer to a reasonable gesture.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ExpandedDrop {
    /// Files to convert, in a stable order.
    files: Vec<String>,
    /// How many dropped entries were folders.
    folders: usize,
    /// Files inside those folders that no format row claims.
    skipped: usize,
    /// True when [`MAX_DROP_FILES`] stopped the walk.
    truncated: bool,
}

/// The most files one drop may queue.
///
/// A limit rather than a promise of completeness: dropping a home directory
/// should say "10000 files, that is more than this was meant for" rather than
/// walk a million entries while the window sits frozen.
const MAX_DROP_FILES: usize = 10_000;

/// How deep a dropped folder is walked.
///
/// Bounded because the walk follows whatever the filesystem presents, and a
/// directory tree is not something this process created.
const MAX_DROP_DEPTH: usize = 16;

/// Expand a drop into the files it means.
///
/// Rules, and the reason for each:
///
/// - a dropped **file** passes through untouched, whatever its extension —
///   dropping it is the explicit request, and *detection* decides what it is
///   (extensions lie, which is the whole of SR-4);
/// - a dropped **folder** contributes only files whose extension a format row
///   claims, because dropping a project folder should not queue `.gitignore`
///   and `node_modules`;
/// - our own receipt sidecars are never inputs;
/// - symlinks are not followed, so a cycle cannot hang the window.
#[tauri::command]
async fn expand_paths(paths: Vec<String>) -> Result<ExpandedDrop, String> {
    blocking(move || {
        let mut out = ExpandedDrop {
            files: Vec::new(),
            folders: 0,
            skipped: 0,
            truncated: false,
        };
        for p in &paths {
            let path = PathBuf::from(p);
            if path.is_dir() {
                out.folders += 1;
                walk_drop(&path, 0, &mut out);
            } else {
                out.files.push(p.clone());
            }
        }
        // Stable order: it decides `{index}` in a naming template, and an
        // output called `holiday_3.jpg` should mean the same file every time.
        out.files.sort();
        out.files.dedup();
        Ok(out)
    })
    .await
}

/// One level of [`expand_paths`]'s folder walk.
fn walk_drop(dir: &Path, depth: usize, out: &mut ExpandedDrop) {
    if depth >= MAX_DROP_DEPTH || out.files.len() >= MAX_DROP_FILES {
        out.truncated = out.files.len() >= MAX_DROP_FILES;
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // an unreadable folder contributes nothing, and is not fatal
    };
    for entry in entries.flatten() {
        if out.files.len() >= MAX_DROP_FILES {
            out.truncated = true;
            return;
        }
        // `file_type` does not follow the link, which is what stops a cycle.
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            walk_drop(&path, depth + 1, out);
        } else if is_droppable(&path) {
            out.files.push(path.to_string_lossy().into_owned());
        } else {
            out.skipped += 1;
        }
    }
}

/// True for a file inside a dropped folder that is worth queueing.
fn is_droppable(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Our own sidecars from an earlier run are not inputs.
    if name.ends_with(".receipt.json") {
        return false;
    }
    let Some(ext) = path.extension().map(|e| e.to_string_lossy().to_lowercase()) else {
        return false;
    };
    TABLE.iter().any(|row| row.extension == ext)
}

/// Render one file for the tool workspace.
///
/// `ops` are the tool ids applied so far, so the stage shows the **pending
/// result** rather than the source. Nothing is written: the worker returns
/// bytes, they are scaled here, and they go out as a data URI.
#[tauri::command]
async fn preview_file(
    path: String,
    page: u32,
    max_px: u32,
    ops: Vec<String>,
) -> Result<FilePreview, String> {
    blocking(move || render_preview(&PathBuf::from(path), page, max_px, &ops)).await
}

/// Convert every file in the drop, and record what happened to each.
///
/// One `Err` per file rather than one for the batch: a drop of forty photos
/// where the third is a polyglot should convert the other thirty-nine and say
/// so, not refuse the lot.
#[tauri::command]
async fn convert_batch(
    app: tauri::AppHandle,
    paths: Vec<String>,
    target_format: String,
    // Absent for callers that have no prediction behind them.
    choices: Option<Vec<ChoiceJson>>,
) -> Result<Vec<ConversionResult>, String> {
    let state = app.state::<AppState>();
    let cancel = Arc::clone(&state.cancel);
    cancel.store(false, Ordering::SeqCst);

    let history_request = serde_json::json!({"paths":paths,"target":target_format});
    let handle = app.clone();
    let (results, created) =
        blocking(move || run_batch(&handle, &paths, &target_format, &cancel, choices.as_deref()))
            .await?;

    if let Ok(db) = receipts_db::ReceiptDatabase::open() {
        let _ = db.record_run(history_request, &results);
    }
    // Replace rather than extend: undo is "the last thing that happened", and a
    // stack that accumulates across batches would let a second Undo remove
    // files the user never saw a toast for.
    let state = app.state::<AppState>();
    if let Ok(mut undo) = state.undo.lock() {
        *undo = created;
    }
    Ok(results)
}

/// Stop the batch after the file that is currently running.
///
/// `Result<(), String>` is INTERFACES.md §11's signature, not a shape clippy
/// would pick: every command returns one so the frontend has exactly one error
/// path to render, and setting a flag cannot fail today.
#[tauri::command]
#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
fn cancel_all(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// Remove the outputs of the most recent batch.
///
/// Deletes only the paths recorded by `convert_batch`, and clears the record
/// whether or not every removal succeeded — a second Undo must not retry
/// against a directory the user has since changed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri injects `State` by value.
fn undo_last(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let items = {
        let mut undo = state
            .undo
            .lock()
            .map_err(|_| "the undo record is unreadable; nothing was removed".to_string())?;
        std::mem::take(&mut *undo)
    };

    let mut failed = Vec::new();
    for item in &items {
        // The receipt goes first. If removal is interrupted between the two,
        // the surviving state is an output whose receipt is missing, which the
        // product already treats as invalid -- rather than a receipt claiming
        // an output that is no longer there.
        for path in [&item.receipt, &item.output] {
            if path.as_os_str().is_empty() {
                continue;
            }
            if let Err(e) = std::fs::remove_file(path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    failed.push(format!("{}: {e}", show(path)));
                }
            }
        }
        // A replaced original is restored before anything else can claim the
        // vacated name. Failure here is reported with the quarantine path, so
        // the file is findable even when the restore could not finish.
        if let Some((quarantined, original)) = &item.replaced_from {
            if let Err(e) = restore_original(quarantined, original) {
                failed.push(format!(
                    "{} could not be restored ({}): {e}",
                    show(original),
                    show(&quarantine_dir())
                ));
            }
        }
    }

    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "some outputs could not be removed — {}",
            failed.join("; ")
        ))
    }
}

// ---------------------------------------------------------------------------
// Settings commands.
// ---------------------------------------------------------------------------

/// The effective configuration, defaults filled in.
#[tauri::command]
async fn get_config() -> Result<ConfigJson, String> {
    blocking(move || Ok(ConfigJson::from(&UserConfig::load()))).await
}

/// Merge a patch into the stored configuration and return it, effective.
///
/// Validation happens here, not in the UI: the config file is also
/// hand-editable, so every read site re-validates anyway (`ConfigJson`'s
/// `enum_or`/`valid_template_or`). A bad value is refused with an inline,
/// field-specific message rather than silently corrected — the user typed it
/// on purpose.
#[tauri::command]
async fn set_config(patch: ConfigPatch) -> Result<ConfigJson, String> {
    blocking(move || {
        let mut config = UserConfig::load();
        merge_patch(&mut config, patch)?;
        config
            .save()
            .map_err(|e| format!("could not save the configuration: {e}"))?;
        Ok(ConfigJson::from(&config))
    })
    .await
}

/// Validate and apply one patch. Pure with respect to disk, so tests can run
/// it without touching the real state directory.
/// The policy this machine runs under, given the user's configuration.
///
/// **`raise_worker_reuse` is a ratchet.** It takes the maximum of what the
/// policy already holds and what is asked for, so a managed deployment that
/// pinned `Isolated` cannot be lowered by this file — which is why the config
/// is read through it rather than assigned over it. `balanced` therefore means
/// "do not raise", not "lower to balanced".
fn policy_from(config: &UserConfig) -> Policy {
    let requested = match config.worker_reuse.as_deref() {
        Some("isolated") => openconvert_core::policy::WorkerReuse::Isolated,
        _ => openconvert_core::policy::WorkerReuse::Balanced,
    };
    let policy = Policy::default().raise_worker_reuse(requested);

    // Not a ratchet. `with_worker_memory` bounds the value at both ends, so
    // an absent key and an out-of-range one both land on something sane.
    let policy = match config.worker_memory_mb {
        Some(mb) => policy.with_worker_memory(mb.saturating_mul(1 << 20)),
        None => policy,
    };

    // A ratchet, like `force_sandbox` below and unlike the memory limit above:
    // present-and-false takes the GPU away, and nothing turns it back on.
    let policy = if config.use_gpu == Some(false) {
        policy.without_gpu()
    } else {
        policy
    };
    // A ratchet: present-and-true turns it on, and nothing turns it off.
    if config.force_sandbox == Some(true) {
        policy.force_sandbox_everywhere()
    } else {
        policy
    }
}

fn merge_patch(config: &mut UserConfig, patch: ConfigPatch) -> Result<(), String> {
    if let Some(t) = &patch.theme {
        if !matches!(t.as_str(), "system" | "light" | "dark") {
            return Err("theme must be system, light or dark".to_string());
        }
    }
    if let Some(d) = &patch.output_destination {
        if !matches!(
            d.as_str(),
            "same_folder" | "downloads" | "desktop" | "replace_source"
        ) {
            return Err(
                "output destination must be same_folder, downloads, desktop or replace_source"
                    .to_string(),
            );
        }
    }
    if let Some(w) = &patch.worker_reuse {
        if !matches!(w.as_str(), "balanced" | "isolated") {
            return Err("worker reuse must be balanced or isolated".to_string());
        }
    }
    if let Some(t) = &patch.naming_template {
        validate_naming_template(t)?;
    }

    // **Absent means unchanged, and that has to be written as a move-if-present
    // rather than an assignment.** Plain `config.theme = patch.theme` sets the
    // stored value to `None` whenever the key is absent — and every patch this
    // UI sends carries exactly one key, so changing the theme was erasing the
    // output destination, the naming template and the receipt preference with
    // it. The doc comment on `ConfigPatch` always said absent meant unchanged;
    // the code said otherwise.
    macro_rules! set_if_given {
        ($($field:ident),+ $(,)?) => {
            $(if patch.$field.is_some() {
                config.$field = patch.$field;
            })+
        };
    }
    set_if_given!(
        worker_reuse,
        force_sandbox,
        use_gpu,
        worker_memory_mb,
        ai_setup_done,
        theme,
        model_auto_update,
        write_receipts,
        output_destination,
        naming_template,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// History + receipt-database commands. The database is desktop-owned; the CLI
// deliberately retains its beside-output JSON receipt contract.
// ---------------------------------------------------------------------------

/// Past conversions, newest first, from the batch journals.
#[tauri::command]
async fn list_history(limit: Option<usize>) -> Result<Vec<serde_json::Value>, String> {
    blocking(move || {
        let limit = limit.unwrap_or(200).min(1000);
        let mut recent = receipts_db::ReceiptDatabase::open()?.runs(limit)?;
        let older = store::read_history(&paths::journal_dir(), limit);
        for entry in older {
            if !recent.iter().any(|r| {
                r["sourceName"] == entry.source_name
                    && r["outputName"] == entry.output_name
                    && r["whenSecs"]
                        .as_u64()
                        .is_some_and(|t| t.abs_diff(entry.when_secs) < 5)
            }) {
                recent.push(serde_json::to_value(entry).map_err(|e| e.to_string())?);
            }
        }
        recent.sort_by_key(|r| std::cmp::Reverse(r["whenSecs"].as_u64().unwrap_or(0)));
        recent.truncate(limit);
        Ok(recent)
    })
    .await
}

// ---------------------------------------------------------------------------
// The signature library
// ---------------------------------------------------------------------------
//
// A named, reusable transparent PNG. It is deliberately NOT called a signature
// anywhere the user might read it as a legal one -- the tool is "Place image on
// page" for exactly that reason -- but the thing a person keeps and reaches for
// is their signature, and pretending otherwise would make the feature harder to
// find than it is to understand.
//
// Files live in `<state>/signatures/<name>.png`. Flat, one directory, no index:
// the filesystem already knows what is in a directory, and a manifest beside
// the files is one more thing that can disagree with them.

/// One stored image, with the bytes the webview needs to draw it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SignatureJson {
    /// What the user called it.
    name: String,
    /// Full path, which is what `run_tool` is given.
    path: String,
    /// `data:image/png;base64,...`, for the picker and the placement ghost.
    data_uri: String,
}

fn signature_dir() -> PathBuf {
    paths::state_dir().join("signatures")
}

/// A filename that is one path component and stays inside the directory.
///
/// The name comes from a text field, so it is the one piece of this that is
/// genuinely user input. Anything outside the allow-list becomes `-`, which
/// makes traversal, device names and shell metacharacters all impossible by
/// construction rather than by a list of things to reject.
fn signature_stem(name: &str) -> Result<String, String> {
    let stem: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let stem = stem.trim().to_string();
    if stem.is_empty() {
        return Err("that name has no usable characters in it".to_string());
    }
    if stem.len() > 64 {
        return Err("that name is too long".to_string());
    }
    Ok(stem)
}

/// Every stored image, newest names last, sorted so the list does not reshuffle.
#[tauri::command]
async fn list_signatures() -> Result<Vec<SignatureJson>, String> {
    blocking(move || {
        let dir = signature_dir();
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            // No directory yet is an empty library, not a failure.
            return Ok(out);
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("png") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("untitled")
                .to_string();
            out.push(SignatureJson {
                name,
                path: path.to_string_lossy().into_owned(),
                data_uri: format!("data:image/png;base64,{}", base64(&bytes)),
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    })
    .await
}

/// Copy a PNG into the library under `name`.
///
/// **The file is decoded before it is stored**, and stored re-encoded. Two
/// reasons: it proves the thing is actually a PNG rather than trusting the
/// extension (SR-4 again), and it means everything in the library is one
/// format, so the stamp path never meets a surprise. The alpha channel is
/// preserved — it is the entire point of the feature.
#[tauri::command]
async fn add_signature(name: String, source: String) -> Result<SignatureJson, String> {
    blocking(move || {
        let stem = signature_stem(&name)?;
        let bytes = if let Some(data) = source.strip_prefix("data:image/png;base64,") {
            use base64::Engine as _;
            if data.len() > 16 << 20 {
                return Err("signature is too large".into());
            }
            base64::engine::general_purpose::STANDARD
                .decode(data)
                .map_err(|_| "invalid signature data")?
        } else {
            std::fs::read(&source)
                .map_err(|e| format!("could not read {}: {e}", show(&PathBuf::from(&source))))?
        };

        // A 64 MB ceiling on something a person drew: generous by two orders of
        // magnitude, and it means a hostile file cannot make the decoder chew
        // through a gigabyte before anyone notices.
        if bytes.len() > 64 << 20 {
            return Err("that image is far too large for a signature".to_string());
        }
        let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(128 << 20);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|e| format!("that signature could not be decoded: {e}"))?
            .to_rgba8();
        if image.width() == 0 || image.height() == 0 {
            return Err("that image has no pixels".to_string());
        }

        let image = trim_signature(image)?;
        std::fs::create_dir_all(signature_dir()).map_err(|e| e.to_string())?;
        let path = signature_dir().join(format!("{stem}.png"));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut out, image::ImageFormat::Png)
            .map_err(|e| format!("that image could not be stored ({e})"))?;
        let encoded = out.into_inner();
        // Overwrites deliberately: adding a signature under a name you already
        // used is a replacement, and the alternative is a silent `-2` suffix
        // the user never asked for.
        std::fs::write(&path, &encoded).map_err(|e| e.to_string())?; // openconvert-lint: allow -- the user named this file in our own store

        Ok(SignatureJson {
            name: stem,
            path: path.to_string_lossy().into_owned(),
            data_uri: format!("data:image/png;base64,{}", base64(&encoded)),
        })
    })
    .await
}

fn trim_signature(mut image: image::RgbaImage) -> Result<image::RgbaImage, String> {
    let (w, h) = image.dimensions();
    let (mut left, mut top, mut right, mut bottom) = (w, h, 0, 0);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        if pixel[0] > 245 && pixel[1] > 245 && pixel[2] > 245 {
            pixel[3] = 0;
        }
        if pixel[3] > 8 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
    }
    if left >= w || top >= h {
        return Err("the signature is blank".into());
    }
    let left = left.saturating_sub(4);
    let top = top.saturating_sub(4);
    let right = (right + 5).min(w);
    let bottom = (bottom + 5).min(h);
    Ok(image::imageops::crop_imm(&image, left, top, right - left, bottom - top).to_image())
}

/// Remove one stored image.
#[tauri::command]
async fn delete_signature(name: String) -> Result<bool, String> {
    blocking(move || {
        let stem = signature_stem(&name)?;
        let path = signature_dir().join(format!("{stem}.png"));
        match std::fs::remove_file(&path) {
            // openconvert-lint: allow -- scoped to our own signature store
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

/// Read back a text output a run just produced.
///
/// **Narrow on purpose, like `reveal_receipt`.** The webview asking the host
/// to read an arbitrary path would be a general file-read capability arriving
/// through the back door. Three checks hold it to shape: it must exist as a
/// file, it must be named `.txt`, and it must be small enough to be a
/// transcript rather than a corpus.
///
/// The path comes from a `ConversionResult` this process produced moments
/// earlier. That is not a reason to skip the checks — the check is what makes
/// it true.
#[tauri::command]
async fn read_text_output(path: String) -> Result<String, String> {
    /// A transcript of a long recording is tens of kilobytes. Four megabytes
    /// is generous by two orders of magnitude and still bounds the read.
    const MAX_BYTES: u64 = 4 << 20;

    blocking(move || {
        let p = PathBuf::from(&path);
        if !p.is_file() {
            return Err("that output is no longer on disk".to_string());
        }
        if !matches!(
            p.extension()
                .and_then(|x| x.to_str())
                .map(|x| x.to_ascii_lowercase())
                .as_deref(),
            Some("txt" | "md" | "srt" | "vtt")
        ) {
            return Err("only a text output can be read back here".to_string());
        }
        match std::fs::metadata(&p) {
            Ok(m) if m.len() > MAX_BYTES => {
                return Err("that file is too large to show here".to_string());
            }
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
        // Lossy rather than a hard failure: a transcript with one unmappable
        // byte in it is still the transcript, and refusing to show it would be
        // the worse answer.
        std::fs::read(&p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .map_err(|e| e.to_string())
    })
    .await
}

/// Peak amplitude per bucket for a recording, for drawing its waveform.
///
/// Replaces a waveform the interface used to invent from a seeded random
/// number generator and label with the user's filename. The values come from
/// the audio itself, decoded in the confined worker like everything else.
#[tauri::command]
async fn audio_peaks(path: String, buckets: Option<usize>) -> Result<Vec<u8>, String> {
    blocking(move || {
        let policy = policy_from(&UserConfig::load());
        openconvert_run::tools::audio_peaks(&PathBuf::from(path), buckets.unwrap_or(96), &policy)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
async fn preview_audio(path: String) -> Result<String, String> {
    blocking(move || {
        use base64::Engine as _;
        let data = openconvert_run::tools::audio_preview(
            &PathBuf::from(path),
            &policy_from(&UserConfig::load()),
        )
        .map_err(|e| e.to_string())?;
        Ok(format!(
            "data:audio/wav;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(data)
        ))
    })
    .await
}

#[tauri::command]
async fn save_image_corrections(
    path: String,
    original: String,
    strokes: String,
) -> Result<String, String> {
    blocking(move || {
        if strokes.len() > 2 << 20 {
            return Err("Too many brush strokes".into());
        }
        let config = UserConfig::load();
        let run = openconvert_run::tools::correct_image(
            &PathBuf::from(path),
            &PathBuf::from(original),
            &strokes,
            &policy_from(&config),
            openconvert_run::tools::ToolRunOptions {
                write_sidecar: config.write_receipts.unwrap_or(true),
            },
        )
        .map_err(|e| e.to_string())?;
        Ok(run.output.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
async fn save_text_copy(path: String, text: String, extension: String) -> Result<String, String> {
    blocking(move || {
        if !matches!(extension.as_str(), "txt" | "srt" | "vtt") || text.len() > 4 << 20 {
            return Err("Invalid text export".into());
        }
        let source = PathBuf::from(&path);
        if !source.is_file() {
            return Err("The source file is no longer on disk".into());
        }
        let stem = source
            .file_stem()
            .ok_or("Source has no filename")?
            .to_string_lossy();
        let name = format!("{stem}-edited.{extension}");
        let dir = source.parent().ok_or("Source has no folder")?;
        let (placed, file) = openconvert_run::write::create_output(
            dir,
            &name,
            openconvert_core::policy::OnConflict::Suffix,
        )
        .map_err(|e| e.to_string())?;
        let mut file = file.ok_or("Could not create an edited copy")?;
        let output = match placed {
            openconvert_run::write::Placed::Created(p)
            | openconvert_run::write::Placed::Skipped(p) => p,
        };
        use std::io::Write as _;
        let written = file
            .write_all(text.as_bytes())
            .and_then(|_| file.sync_all());
        drop(file);
        if let Err(e) = written {
            let _ = remove_created_output(&output);
            return Err(e.to_string());
        }
        Ok(output.to_string_lossy().into_owned())
    })
    .await
}

/// Show a receipt in the operating system's file manager.
///
/// **Not the `opener` plugin.** The capability gate refuses every plugin
/// namespace, and rightly: `opener` would let the webview hand any string to
/// the shell. This is one verb, over one kind of path, on this side of the
/// boundary where it can be checked first.
///
/// The checks are the whole of it: the path must exist, be a file, and resolve
/// to the desktop-owned SQLite database. The database lives in the user's own
/// app-data directory, but it is still hand-editable; "reveal whatever this
/// row says" is not a shell capability the webview receives.
///
/// Revealing does not open. `explorer /select`, `open -R` and the Linux
/// fallback all put a folder on screen with the file highlighted; none of them
/// executes it.
#[tauri::command]
async fn reveal_receipt(path: String) -> Result<(), String> {
    blocking(move || {
        let p = PathBuf::from(&path);
        if !p.is_file() {
            return Err("that receipt is no longer on disk".to_string());
        }
        let expected = receipts_db::database_path();
        let requested = p.canonicalize().map_err(|e| e.to_string())?;
        let owned = expected.canonicalize().map_err(|e| e.to_string())?;
        if requested != owned {
            return Err("only a receipt can be revealed from here".to_string());
        }
        reveal_in_file_manager(&p)
    })
    .await
}

/// Show an output from the most recent batch in its containing folder.
#[tauri::command]
async fn reveal_output(path: String, state: tauri::State<'_, AppState>) -> Result<(), String> {
    let requested = PathBuf::from(path);
    let allowed = state
        .undo
        .lock()
        .map_err(|_| "the recent-output record is unavailable".to_string())?
        .iter()
        .any(|item| item.output == requested);
    let allowed =
        allowed || receipts_db::ReceiptDatabase::open().is_ok_and(|db| db.has_output(&requested));
    if !allowed || !requested.is_file() {
        return Err("only an output from the most recent batch can be revealed".to_string());
    }
    blocking(move || reveal_in_file_manager(&requested)).await
}

/// Open the file manager with `path` selected.
///
/// # Explorer does not parse arguments the way anything else does
///
/// Two separate things made this open **Documents** every time instead of the
/// output's folder, and both are Windows-specific:
///
/// **Separators.** This tree hands paths around with forward slashes -- every
/// path the CLI prints looks like `C:/Users/...` -- and `/select,C:/Users/...`
/// is not a path Explorer resolves. It fails, and a failed `/select` does not
/// error: Explorer opens its default folder, which is Documents.
///
/// **Quoting.** `Command::arg` quotes any argument containing a space, so a
/// path with one arrives as `"/select,C:\Program Files\x.png"` -- the switch
/// inside the quotes. Explorer reads that as a single path, does not find it,
/// and opens Documents again. `raw_arg` is the escape from Rust's quoting, and
/// the quotes then go where Explorer wants them: around the path alone.
///
/// Both failures look identical to a user and neither reports anything, which
/// is why "show in folder does nothing useful" went unexplained for so long.
fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt as _;
        // Absolute, and with the separators Explorer understands. A relative
        // path would resolve against the app's working directory, which is not
        // where the output is.
        let full = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .replace('/', "\\");
        // `canonicalize` yields a `\\?\` extended-length prefix, and Explorer
        // does not accept one.
        let full = full.strip_prefix("\\\\?\\").unwrap_or(&full).to_string();
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{full}\""))
            .spawn()
    };
    #[cfg(target_os = "macos")]
    let spawned = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let spawned = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    spawned
        .map(|_| ())
        .map_err(|e| format!("the file manager could not be opened ({e})"))
}

/// Delete every journal file. History only — nothing outside the journal
/// directory, ever.
#[tauri::command]
async fn wipe_history() -> Result<usize, String> {
    blocking(move || {
        let count = store::wipe_history(&paths::journal_dir()).map_err(|e| e.to_string())?;
        Ok(count + receipts_db::ReceiptDatabase::open()?.clear_runs()?)
    })
    .await
}

/// The app-data receipt database, newest first.
#[tauri::command]
async fn list_receipts(limit: Option<usize>) -> Result<Vec<receipts_db::ReceiptEntry>, String> {
    blocking(move || receipts_db::ReceiptDatabase::open()?.list(limit.unwrap_or(200).min(1000), 0))
        .await
}

/// What the graphics card is called, for the line beside the GPU switch.
///
/// `None` when there is no card, when the platform cannot say, or when the
/// registry could not be read -- all three render the same way, as the switch
/// with nothing beside it.
///
/// **The name is not a claim that the card will be used.** Whether DirectML
/// loads, and whether a model fits, are decided per run and much later. The
/// interface pairs this with the switch's own state and says no more than
/// "this is the card, and you have asked for it to be used".
#[tauri::command]
async fn gpu_device() -> Result<Option<String>, String> {
    blocking(move || Ok(openconvert_os::gpu::primary().map(|a| a.name))).await
}

/// Bytes currently held by the receipt database, for the reclaim label.
#[tauri::command]
async fn receipts_size() -> Result<u64, String> {
    blocking(move || Ok(receipts_db::ReceiptDatabase::open()?.storage_bytes())).await
}

/// Exact number of stored conversions; unlike the listing, this is not capped.
#[tauri::command]
async fn receipts_count() -> Result<usize, String> {
    blocking(move || {
        usize::try_from(receipts_db::ReceiptDatabase::open()?.count()?)
            .map_err(|_| "the receipt count is too large to display".to_string())
    })
    .await
}

/// Remove one stored receipt. Immediate and unrecoverable, which the button
/// copy says too.
#[tauri::command]
async fn delete_receipt(id: String) -> Result<(), String> {
    blocking(move || {
        receipts_db::ReceiptDatabase::open()?
            .delete(&id)
            .map(|_| ())
            .map_err(|e| format!("{} could not be removed: {e}", DisplayName::new(&id)))
    })
    .await
}

/// Remove every stored receipt. Returns the bytes reclaimed.
#[tauri::command]
async fn delete_all_receipts() -> Result<u64, String> {
    blocking(move || receipts_db::ReceiptDatabase::open()?.delete_all()).await
}

// ---------------------------------------------------------------------------
// Model management commands, over A10's registry (openconvert_run::models).
// ---------------------------------------------------------------------------

/// One declared model with its per-user state, on the wire.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfoJson {
    id: String,
    title: String,
    purpose: String,
    size_bytes: u64,
    licence: String,
    downloaded: bool,
    enabled: bool,
    /// Why it cannot be downloaded, or absent when it can.
    blocked: Option<String>,
}

/// One AI capability, as a chooser shows it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AiFeatureJson {
    id: String,
    title: String,
    does: String,
    size_bytes: u64,
    downloaded: bool,
    /// Whether anything in the app can actually reach it.
    ///
    /// A capability with no interface is still listed -- hiding it would make
    /// the registry and the chooser disagree -- but it is never offered for
    /// download, because a user who accepts a download and then finds no button
    /// has been told something untrue.
    usable: bool,
    licences: Vec<String>,
    models: Vec<String>,
    /// The tool these artifacts serve, when one exists.
    ///
    /// Several features may name the SAME tool and differ only in tier -- that
    /// is what a tier is. Settings groups on this, so the user chooses between
    /// options for a job rather than between files.
    tool: Option<String>,
    /// `small` | `better` | `best`.
    tier: String,
    /// Whether every artifact of this tier is present AND switched on.
    ///
    /// Not the same as `downloaded`: a tier is all-or-nothing, and a model the
    /// user switched off is a model they said no to.
    ready: bool,
    /// True when this is the tier a run would actually pick right now.
    ///
    /// The chooser has to be able to say WHICH of two installed tiers is in use,
    /// or installing the better one and seeing no difference is
    /// indistinguishable from it not working.
    active: bool,
    /// Whether the FIRST-RUN screen offers this one.
    ///
    /// Settings lists every tier, because Settings is where you manage them.
    /// The first-run screen is a different question -- "what should I fetch
    /// before I have used the program at all" -- and a 224 MB tier is not an
    /// answer to it. The registry decides, so this screen and the installer's
    /// own page cannot drift apart.
    offered_at_install: bool,
}

/// Every AI capability, with the size it costs and what it does.
///
/// Sizes are summed from the registry, so the number on the chooser is the
/// number that gets fetched.
#[tauri::command]
async fn list_ai_features() -> Result<Vec<AiFeatureJson>, String> {
    blocking(move || {
        Ok(openconvert_run::models::features()
            .into_iter()
            .map(|f| AiFeatureJson {
                id: f.feature.id.clone(),
                title: f.feature.title.clone(),
                does: f.feature.does.clone(),
                size_bytes: f.size_bytes,
                downloaded: f.downloaded,
                // NAMING A TOOL IS NOT THE SAME AS HAVING ONE. This asked
                // only whether `models.toml` filled in a `tool` field, so a
                // capability whose descriptor had been withdrawn still
                // advertised itself as usable -- which is exactly what
                // happened to OCR. Ask the registry that owns the answer.
                usable: f.feature.tool.as_deref().is_some_and(|id| {
                    openconvert_run::tools::list_tools()
                        .iter()
                        .any(|t| t.id == id)
                }),
                licences: f.licences,
                models: f.feature.models.clone(),
                tool: f.feature.tool.clone(),
                tier: f.feature.tier.name().to_string(),
                ready: f
                    .feature
                    .tool
                    .as_deref()
                    .is_some_and(|tool| openconvert_run::models::tier_ready(tool, f.feature.tier)),
                active: f.feature.tool.as_deref().is_some_and(|tool| {
                    openconvert_run::tools::tier_for_tool(tool) == Some(f.feature.tier)
                }),
                offered_at_install: f.feature.offered_at_install,
            })
            .collect())
    })
    .await
}

/// Fetch every artifact one capability needs, and switch it on.
///
/// Through the same registry path a single model takes: sha256 pinned, the
/// sentinel row refused, nothing written outside the content-addressed store.
/// A capability with no interface is refused here as well as hidden in the
/// chooser -- two places, because a command is reachable from more than the UI
/// that normally calls it.
#[tauri::command]
async fn download_ai_feature(app: tauri::AppHandle, id: String) -> Result<(), String> {
    blocking(move || {
        let feature = openconvert_run::models::feature_rows()
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| format!("no AI capability called {id:?}"))?;
        // Same question, asked the same way, in the second place a caller can
        // reach: the command is not only called by the chooser.
        let has_tool = feature.tool.as_deref().is_some_and(|id| {
            openconvert_run::tools::list_tools().iter().any(|t| t.id == id)
        });
        if !has_tool {
            return Err(format!(
                "{} has no interface in this build. Downloading it would cost you the disk space and do nothing.",
                feature.title
            ));
        }
        // Progress is reported per ARTIFACT, under the feature's id: a
        // capability is one row in the chooser, so four whisper files must not
        // reset the same bar four times without explanation. The frontend shows
        // the feature moving; the id it receives is the one it is drawing.
        for model in &feature.models {
            let emit_id = feature.id.clone();
            let handle = app.clone();
            openconvert_run::models::download_with_progress(model, &mut |received, total| {
                emit_progress(&handle, &emit_id, received, total, "downloading");
            })
            .map_err(|e| e.to_string())?;
            openconvert_run::models::set_enabled(model, true).map_err(|e| e.to_string())?;
        }
        emit_progress(&app, &feature.id, 0, 0, "done");
        Ok(())
    })
    .await
}

/// Every declared model with its per-user state.
#[tauri::command]
async fn list_models() -> Result<Vec<ModelInfoJson>, String> {
    blocking(move || {
        Ok(openconvert_run::models::list()
            .into_iter()
            .map(|m| ModelInfoJson {
                id: m.id,
                title: m.title,
                purpose: m.purpose,
                size_bytes: m.size_bytes,
                licence: m.licence,
                downloaded: m.downloaded,
                enabled: m.enabled,
                blocked: m.blocked,
            })
            .collect())
    })
    .await
}

/// Emit one `model-download-progress` event.
///
/// The whole reason this helper exists: the frontend has listened for this
/// event since the settings screen was written, the preview harness emitted it,
/// and the product never did -- so the bar sat at zero for the length of a
/// 78 MB download and then disappeared. One producer, used by both commands
/// that download.
fn emit_progress(app: &tauri::AppHandle, id: &str, received: u64, total: u64, phase: &str) {
    // A dropped event must not fail a download. The bar is a courtesy; the
    // bytes are the job.
    let _ = app.emit(
        "model-download-progress",
        serde_json::json!({
            "id": id,
            "receivedBytes": received,
            "totalBytes": total,
            "phase": phase,
            "message": serde_json::Value::Null,
        }),
    );
}

/// Download one model, reporting progress as it goes.
///
/// The registry enforces sha256 pinning and the sentinel rule — a row whose
/// hash was never verified refuses here, with that reason.
///
/// **`verifying` is a real phase, not decoration.** The full-file hash runs
/// after the last byte arrives, and on a 78 MB artifact it is long enough to
/// look like a hang if the bar simply sits at 100%.
#[tauri::command]
async fn download_model(app: tauri::AppHandle, id: String) -> Result<(), String> {
    blocking(move || {
        let emit_id = id.clone();
        let handle = app.clone();
        let mut seen_total = 0_u64;
        let result =
            openconvert_run::models::download_with_progress(&id, &mut |received, total| {
                seen_total = total;
                emit_progress(&handle, &emit_id, received, total, "downloading");
            });
        match result {
            Ok(_) => {
                emit_progress(&app, &id, seen_total, seen_total, "done");
                Ok(())
            }
            Err(e) => {
                let message = e.to_string();
                let _ = app.emit(
                    "model-download-progress",
                    serde_json::json!({
                        "id": id,
                        "receivedBytes": 0,
                        "totalBytes": seen_total,
                        "phase": "failed",
                        "message": message,
                    }),
                );
                Err(message)
            }
        }
    })
    .await
}

/// Enable or disable a downloaded model. Disabled is not deleted: routing
/// simply refuses, and the worker's message says where to switch it back on.
#[tauri::command]
async fn set_model_enabled(id: String, enabled: bool) -> Result<(), String> {
    blocking(move || openconvert_run::models::set_enabled(&id, enabled).map_err(|e| e.to_string()))
        .await
}

/// Choose which tier of a tool's models to run.
///
/// `auto` -- the default, and what an absent entry means -- resolves to the
/// best tier that is ready. An explicit tier is honoured or refused, never
/// silently downgraded.
#[tauri::command]
async fn set_model_tier(tool: String, tier: String) -> Result<(), String> {
    blocking(move || {
        // Validated here, at the boundary, rather than trusted from the
        // interface: this writes to a config file the CLI reads too.
        if tier != "auto" && openconvert_run::models::Tier::parse(&tier).is_none() {
            return Err(format!(
                "{} is not a tier; expected auto, small, better or best",
                DisplayName::new(&tier)
            ));
        }
        let mut config = UserConfig::load();
        if tier == "auto" {
            // `auto` is the ABSENCE of a preference, so it is stored as one.
            // Writing the word would make a file that grows an entry per tool
            // the user ever looked at.
            config.model_tier.remove(&tool);
        } else {
            config.model_tier.insert(tool, tier);
        }
        config
            .save()
            .map_err(|e| format!("could not save the configuration: {e}"))
    })
    .await
}

/// Reclaim a downloaded model's disk space.
#[tauri::command]
async fn delete_model(id: String) -> Result<(), String> {
    blocking(move || {
        openconvert_run::models::delete(&id)
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .await
}

// ---------------------------------------------------------------------------
// Tools commands, over A10's registry (openconvert_run::tools).
// ---------------------------------------------------------------------------

// Keep the complete registry schema on the wire, including choices and defaults.
type ToolParamJson = openconvert_run::tools::ToolParam;

/// What one tool looks like on the wire (INTERFACES.md §11.4).
///
/// Re-mapped from `openconvert_run::tools::ToolDescriptor` so the IPC surface
/// keeps its single camelCase convention regardless of how the registry
/// spells its own structs; the mapping is field-for-field and tested below.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolDescriptorJson {
    id: String,
    category: String,
    title: String,
    available: bool,
    unavailable_reason: Option<String>,
    multi_input: bool,
    params: Vec<ToolParamJson>,
}

fn tool_json(d: &openconvert_run::tools::ToolDescriptor) -> ToolDescriptorJson {
    ToolDescriptorJson {
        category: format!("{:?}", d.category).to_lowercase(),
        id: d.id.clone(),
        title: d.title.clone(),
        available: d.available,
        unavailable_reason: d.unavailable_reason.clone(),
        multi_input: d.multi_input,
        params: d.params.clone(),
    }
}

/// What the Tools menu may show. No frontend-side menu entries exist.
#[tauri::command]
async fn list_tools() -> Result<Vec<ToolDescriptorJson>, String> {
    blocking(move || {
        Ok(openconvert_run::tools::list_tools()
            .iter()
            .map(tool_json)
            .collect())
    })
    .await
}

/// Run a tool through the same pipeline as any conversion: progress events,
/// receipts, undo.
///
/// Multi-input tools take one call with every path; per-input tools loop with
/// cancellation checked between files, exactly like `convert_batch`.
#[tauri::command]
async fn run_tool(
    app: tauri::AppHandle,
    id: String,
    paths: Vec<String>,
    params: serde_json::Value,
) -> Result<Vec<ConversionResult>, String> {
    let state = app.state::<AppState>();
    let cancel = Arc::clone(&state.cancel);
    cancel.store(false, Ordering::SeqCst);

    let history_request = serde_json::json!({"paths":paths,"target":id,"params":params});
    let handle = app.clone();
    let (results, created) =
        blocking(move || run_tool_batch(&handle, &id, &paths, params, &cancel)).await?;

    if let Ok(db) = receipts_db::ReceiptDatabase::open() {
        let _ = db.record_run(history_request, &results);
    }
    if let Ok(mut undo_record) = app.state::<AppState>().undo.lock() {
        *undo_record = created;
    }
    Ok(results)
}

/// The bridge between A10's handler signature and this shell's pipeline.
fn run_tool_batch(
    app: &tauri::AppHandle,
    id: &str,
    paths: &[String],
    params: serde_json::Value,
    cancel: &AtomicBool,
) -> Result<(Vec<ConversionResult>, Vec<UndoItem>), String> {
    use openconvert_run::tools as registry;

    // The descriptor decides availability BEFORE anything runs — a refused
    // tool names its reason, which is the message the UI shows.
    //
    // `all_tools`, NOT `list_tools`. The second is the MENU, and the workspace
    // saves through `pdf-compose`, which is deliberately not on it — so this
    // lookup refused every merge, reorder and signature with "no tool named
    // pdf-compose is registered", and the combined PDF flow could never save.
    let descriptor = registry::all_tools()
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| format!("no tool named {} is registered", DisplayName::new(id)))?;
    if !descriptor.available {
        return Err(descriptor
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| "this tool is not available on this machine".to_string()));
    }

    // Params arrive as a JSON object; the handlers read string pairs.
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(map) = params.as_object() {
        for (k, v) in map {
            let value = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            pairs.push((k.clone(), value));
        }
    }

    let config = UserConfig::load();
    let write_receipts = config.write_receipts.unwrap_or(true);
    let policy = policy_from(&config);
    let total = paths.len();
    let mut results = Vec::new();
    let mut created = Vec::new();

    if descriptor.multi_input {
        // One run over everything: join/merge is one output, one receipt.
        for (index, raw) in paths.iter().enumerate() {
            emit(
                app,
                &ProgressEvent {
                    index,
                    total,
                    file_name: DisplayName::new(&file_name_of(Path::new(raw)))
                        .as_str()
                        .to_string(),
                    phase: "started",
                },
            );
        }
        let start = std::time::Instant::now();
        let inputs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
        return match registry::run_tool_with(
            id,
            &inputs,
            &pairs,
            &policy,
            registry::ToolRunOptions {
                write_sidecar: false,
            },
        )
        .and_then(|run| {
            place_tool_output(run, &pairs, &config, 0).map_err(registry::ToolError::BadInput)
        }) {
            Ok(run) => {
                for (index, raw) in paths.iter().enumerate() {
                    emit(
                        app,
                        &ProgressEvent {
                            index,
                            total,
                            file_name: DisplayName::new(&file_name_of(Path::new(raw)))
                                .as_str()
                                .to_string(),
                            phase: "done",
                        },
                    );
                }
                let mut result = tool_result(
                    &first_name(paths),
                    &inputs,
                    &run.output,
                    &run.receipt_body,
                    millis(start),
                );
                let receipt = commit_desktop_receipt(
                    &run.receipt_body,
                    &first_name(paths),
                    write_receipts,
                    &run.output,
                    None,
                );
                let receipt = match receipt {
                    Ok(path) => path,
                    Err(error) => {
                        results.push(failure(&first_name(paths), &error, millis(start), 0));
                        return Ok((results, created));
                    }
                };
                result.receipt_path = show(&receipt);
                created.push(UndoItem {
                    output: run.output,
                    receipt: PathBuf::new(),
                    replaced_from: None,
                });
                results.push(result);
                Ok((results, created))
            }
            Err(e) => {
                for (index, raw) in paths.iter().enumerate() {
                    emit(
                        app,
                        &ProgressEvent {
                            index,
                            total,
                            file_name: DisplayName::new(&file_name_of(Path::new(raw)))
                                .as_str()
                                .to_string(),
                            phase: "failed",
                        },
                    );
                }
                results.push(failure(
                    &first_name(paths),
                    &e.to_string(),
                    millis(start),
                    0,
                ));
                Ok((results, created))
            }
        };
    }

    for (index, raw) in paths.iter().enumerate() {
        let path = PathBuf::from(raw);
        let name = DisplayName::new(&file_name_of(&path)).as_str().to_string();

        if cancel.load(Ordering::SeqCst) {
            emit(
                app,
                &ProgressEvent {
                    index,
                    total,
                    file_name: name.clone(),
                    phase: "cancelled",
                },
            );
            results.push(failure(&name, "cancelled before this file started", 0, 0));
            continue;
        }
        emit(
            app,
            &ProgressEvent {
                index,
                total,
                file_name: name.clone(),
                phase: "started",
            },
        );

        let start = std::time::Instant::now();

        // A MULTI-OUTPUT TOOL GETS THE OTHER ENTRY POINT, and its parts each
        // become a result row.
        //
        // Split is the only one today. Calling `run_tool_with` for it returns
        // an error rather than one part of six -- deliberately, so this branch
        // cannot be forgotten and leave the user looking at a single file
        // believing that is all the tool produced.
        if registry::is_multi_output(id) {
            match registry::run_multi_tool(
                id,
                std::slice::from_ref(&path),
                &pairs,
                &policy,
                registry::ToolRunOptions {
                    write_sidecar: false,
                },
            )
            .and_then(|parts| {
                place_tool_outputs(parts, &pairs, &config).map_err(registry::ToolError::BadInput)
            }) {
                Ok(parts) => {
                    for part in &parts {
                        created.push(UndoItem {
                            output: part.output.clone(),
                            receipt: PathBuf::new(),
                            replaced_from: None,
                        });
                        // One receipt per file, because each describes only
                        // its own part.
                        let mut result = tool_result(
                            &name,
                            std::slice::from_ref(&path),
                            &part.output,
                            &part.receipt_body,
                            millis(start),
                        );
                        // A receipt that could not be written does not fail the
                        // part: the file exists either way, and the row saying
                        // so is more useful than a refusal.
                        if let Ok(receipt) = commit_desktop_receipt(
                            &part.receipt_body,
                            &name,
                            write_receipts,
                            &part.output,
                            None,
                        ) {
                            result.receipt_path = show(&receipt);
                        }
                        results.push(result);
                    }
                    emit(
                        app,
                        &ProgressEvent {
                            index,
                            total,
                            file_name: name.clone(),
                            phase: "done",
                        },
                    );
                }
                Err(error) => {
                    emit(
                        app,
                        &ProgressEvent {
                            index,
                            total,
                            file_name: name.clone(),
                            phase: "failed",
                        },
                    );
                    results.push(failure(&name, &error.to_string(), millis(start), 0));
                }
            }
            continue;
        }

        let progress_app = app.clone();
        match openconvert_run::worker_client::with_progress(
            move |completed, units| {
                let _ = progress_app.emit(
                    "tool-progress",
                    serde_json::json!({
                        "index": index, "total": total, "fraction": completed as f64 / units as f64
                    }),
                );
            },
            || {
                registry::run_tool_with(
                    id,
                    std::slice::from_ref(&path),
                    &pairs,
                    &policy,
                    registry::ToolRunOptions {
                        write_sidecar: false,
                    },
                )
                .and_then(|run| {
                    place_tool_output(run, &pairs, &config, 0)
                        .map_err(registry::ToolError::BadInput)
                })
            },
        ) {
            Ok(run) => {
                let mut result = tool_result(
                    &name,
                    std::slice::from_ref(&path),
                    &run.output,
                    &run.receipt_body,
                    millis(start),
                );
                let receipt = match commit_desktop_receipt(
                    &run.receipt_body,
                    &name,
                    write_receipts,
                    &run.output,
                    None,
                ) {
                    Ok(path) => path,
                    Err(error) => {
                        emit(
                            app,
                            &ProgressEvent {
                                index,
                                total,
                                file_name: name.clone(),
                                phase: "failed",
                            },
                        );
                        results.push(failure(&name, &error, millis(start), 0));
                        continue;
                    }
                };
                result.receipt_path = show(&receipt);
                created.push(UndoItem {
                    output: run.output,
                    receipt: PathBuf::new(),
                    replaced_from: None,
                });
                emit(
                    app,
                    &ProgressEvent {
                        index,
                        total,
                        file_name: name.clone(),
                        phase: "done",
                    },
                );
                results.push(result);
            }
            Err(e) => {
                emit(
                    app,
                    &ProgressEvent {
                        index,
                        total,
                        file_name: name.clone(),
                        phase: "failed",
                    },
                );
                results.push(failure(&name, &e.to_string(), millis(start), 0));
            }
        }
    }
    Ok((results, created))
}

fn place_tool_outputs(
    parts: Vec<openconvert_run::tools::ToolRun>,
    params: &[(String, String)],
    config: &UserConfig,
) -> Result<Vec<openconvert_run::tools::ToolRun>, String> {
    let originals: Vec<PathBuf> = parts.iter().map(|r| r.output.clone()).collect();
    let mut out: Vec<openconvert_run::tools::ToolRun> = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        match place_tool_output(part, params, config, i) {
            Ok(run) => out.push(run),
            Err(e) => {
                for path in originals.iter().chain(out.iter().map(|r| &r.output)) {
                    let _ = remove_created_output(path);
                }
                return Err(e);
            }
        }
    }
    Ok(out)
}

fn place_tool_output(
    run: openconvert_run::tools::ToolRun,
    params: &[(String, String)],
    config: &UserConfig,
    index: usize,
) -> Result<openconvert_run::tools::ToolRun, String> {
    let original = run.output.clone();
    let result = place_tool_output_inner(run, params, config, index);
    if result.is_err() {
        let _ = remove_created_output(&original);
    }
    result
}

fn place_tool_output_inner(
    mut run: openconvert_run::tools::ToolRun,
    params: &[(String, String)],
    config: &UserConfig,
    index: usize,
) -> Result<openconvert_run::tools::ToolRun, String> {
    let mut body: serde_json::Value =
        serde_json::from_str(&run.receipt_body).map_err(|e| e.to_string())?;
    let requested = params
        .iter()
        .find(|(k, _)| k == "destination")
        .map(|(_, v)| v.as_str())
        .unwrap_or("same_folder");
    let dir = match requested {
        "downloads" => downloads_dir().ok_or("Downloads folder not found")?,
        "desktop" => desktop_dir().ok_or("Desktop folder not found")?,
        "same_folder" => run
            .output
            .parent()
            .ok_or("output has no folder")?
            .to_path_buf(),
        _ => return Err("unsupported tool destination".into()),
    };
    let names: Vec<String> = params
        .iter()
        .find(|(k, _)| k == "output_names")
        .map(|(_, v)| serde_json::from_str(v))
        .transpose()
        .map_err(|_| "invalid output names")?
        .unwrap_or_default();
    let name = if let Some(custom) = names.get(index).filter(|v| !v.trim().is_empty()) {
        if custom
            .chars()
            .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))
            || custom.trim().starts_with('.')
            || custom.ends_with(' ')
            || custom.ends_with('.')
        {
            return Err("output names must be plain file names".into());
        }
        let ext = run
            .output
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("pdf");
        if custom.to_lowercase().ends_with(&format!(".{ext}")) {
            custom.clone()
        } else {
            format!("{custom}.{ext}")
        }
    } else {
        file_name_of(&run.output)
    };
    if dir.join(&name) == run.output {
        return Ok(run);
    }
    let policy = policy_from(config);
    let (placed, file) = openconvert_run::write::create_output(&dir, &name, policy.on_conflict())
        .map_err(|e| e.to_string())?;
    let mut file = file.ok_or("an output with this name already exists")?;
    let output = match placed {
        openconvert_run::write::Placed::Created(p) | openconvert_run::write::Placed::Skipped(p) => {
            p
        }
    };
    let copied = (|| -> Result<(), String> {
        let mut source = std::fs::File::open(&run.output).map_err(|e| e.to_string())?;
        std::io::copy(&mut source, &mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();
    drop(file);
    if let Err(e) = copied {
        let _ = remove_created_output(&output);
        return Err(e);
    }
    if let Err(e) = remove_created_output(&run.output) {
        let _ = remove_created_output(&output);
        return Err(e);
    }
    run.output = output;

    body["output"] = serde_json::json!(run.output.to_string_lossy());
    run.receipt_body = body.to_string();
    Ok(run)
}

fn first_name(paths: &[String]) -> String {
    match paths {
        [only] => DisplayName::new(&file_name_of(Path::new(only)))
            .as_str()
            .to_string(),
        [first, rest @ ..] => format!(
            "{} (+{} more)",
            DisplayName::new(&file_name_of(Path::new(first))).as_str(),
            rest.len()
        ),
        [] => String::new(),
    }
}

/// Commit one desktop receipt, compensating the filesystem if SQLite fails.
fn commit_desktop_receipt(
    body: &str,
    input_name: &str,
    keep: bool,
    output: &Path,
    replaced: Option<&(PathBuf, PathBuf)>,
) -> Result<PathBuf, String> {
    commit_receipt_with(body, input_name, keep, output, replaced, |body, input| {
        let database = receipts_db::ReceiptDatabase::open()?;
        database.insert_json(body, input)?;
        Ok(database.path().to_path_buf())
    })
}

/// Injectable core of the commit protocol. Keeping the writer behind a small
/// closure makes database-failure rollback testable without touching the real
/// application-data directory.
fn commit_receipt_with<F>(
    body: &str,
    input_name: &str,
    keep: bool,
    output: &Path,
    replaced: Option<&(PathBuf, PathBuf)>,
    write: F,
) -> Result<PathBuf, String>
where
    F: FnOnce(&str, &str) -> Result<PathBuf, String>,
{
    if !keep {
        return Ok(PathBuf::new());
    }
    match write(body, input_name) {
        Ok(path) => Ok(path),
        Err(error) => match rollback_conversion_files(output, replaced) {
            Ok(()) => Err(format!(
                "the conversion record could not be saved ({error}); the new output was rolled back and the original was restored"
            )),
            Err(rollback_error) => Err(format!(
                "the conversion record could not be saved ({error}); rollback also failed ({rollback_error})"
            )),
        },
    }
}

/// Remove only the output created by the current operation during rollback.
fn remove_created_output(output: &Path) -> Result<(), String> {
    match std::fs::remove_file(output) {
        // openconvert-lint: allow -- rolls back only this operation's newly-created output
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn rollback_conversion_files(
    output: &Path,
    replaced: Option<&(PathBuf, PathBuf)>,
) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(error) = remove_created_output(output) {
        errors.push(format!("new output: {error}"));
    }
    if let Some((quarantined, original)) = replaced {
        if let Err(error) = restore_original(quarantined, original) {
            errors.push(format!("original: {error}"));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(", "))
    }
}

/// Build one result row from what a tool produced, parsing its receipt body
/// leniently: container surgery writes its own receipt shape, so any field
/// that is absent stays empty rather than inventing data.
fn tool_result(
    name: &str,
    inputs: &[PathBuf],
    output: &Path,
    receipt_body: &str,
    duration_ms: u64,
) -> ConversionResult {
    let input_bytes = inputs
        .iter()
        .filter_map(|p| p.metadata().ok())
        .map(|m| m.len())
        .sum();
    let parsed: serde_json::Value =
        serde_json::from_str(receipt_body).unwrap_or(serde_json::Value::Null);

    let steps = parsed
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .map(|st| StepReceiptJson {
                    kind: st
                        .get("kind")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    class: st
                        .get("class")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    engine: st
                        .get("engine")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    isolation: st
                        .get("isolation")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    limits_summary: String::new(),
                })
                .collect()
        })
        .unwrap_or_default();

    let removed_metadata = parsed
        .get("steps")
        .and_then(serde_json::Value::as_array)
        .map(|list| {
            list.iter()
                .flat_map(|st| {
                    st.get("removed")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default()
                })
                .filter_map(|r| r.as_str().map(|s| DisplayName::new(s).as_str().to_string()))
                .collect()
        })
        .unwrap_or_default();

    let detail = ReceiptDetailJson {
        version: parsed.get("version").and_then(Value::as_u64).unwrap_or(1) as u32,
        tool: parsed
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        content_id: parsed
            .get("content_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        detected: parsed
            .get("detected")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        declared_mismatch: None,
        class: parsed
            .get("class")
            .and_then(Value::as_str)
            .map(str::to_string),
        output_name: parsed
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        output_bytes: output.metadata().map(|m| m.len()).unwrap_or(0),
        steps,
    };

    ConversionResult {
        file_name: name.to_string(),
        output_path: show(output),
        receipt_path: String::new(),
        success: true,
        error_message: None,
        duration_ms,
        input_bytes,
        output_bytes: detail.output_bytes,
        class_applied: detail.class.clone().unwrap_or_else(|| "—".to_string()),
        removed_metadata,
        content_id: detail.content_id.clone(),
        receipt_detail: Some(detail),
    }
}

// ---------------------------------------------------------------------------
// Panic, browse.
// ---------------------------------------------------------------------------

/// Stop everything, as far as this build honestly can.
///
/// Sets the cancellation flag, which stops every batch at the boundary it
/// actually has — between files. Workers are job-scoped: each batch owns its
/// pool privately, so a panic cannot reach a mid-decode worker by handle; the
/// worker's own wall-clock and memory limits are what bound it. There is no
/// shared temp root to wipe — the sweep rule forbids one — so nothing outside
/// our state directory is touched. The limits are recorded here rather than
/// papered over: a panic button that promises more than it does is worse than
/// one that says exactly what it stops.
#[tauri::command]
#[allow(clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
fn panic_stop(state: tauri::State<'_, AppState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

/// Open the platform file dialog and return absolute paths.
///
/// Runs host-side via `rfd`, so no webview capability or plugin permission is
/// involved: the paths reach the UI the same way drop paths already do. The
/// webview cannot open a dialog itself, and that is the point of the gate.
#[tauri::command]
async fn pick_files(kind: Option<String>) -> Result<Option<Vec<String>>, String> {
    blocking(move || {
        // A tool that only works on PDFs should offer PDFs. The filter is a
        // convenience, never a check: whatever comes back is still detected by
        // content, because an extension is a claim and not evidence (SR-4).
        let mut dialog = rfd::FileDialog::new().set_title("Choose files");
        match kind.as_deref() {
            Some("image") => {
                dialog = dialog.add_filter(
                    "Images",
                    &[
                        "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "heic", "avif",
                        "jxl",
                    ],
                );
            }
            Some("audio") => {
                // EVERY RECORDING THE AUDIO TOOLS CAN ACTUALLY READ.
                //
                // This listed six extensions while the routes accepted nine.
                // `mkv`, `webm` and `mp4` transcribe and denoise perfectly —
                // the worker demuxes them and takes the audio track — but the
                // dialog hid them, so someone with a screen recording or a
                // phone video was told, by an empty file list, that the file
                // was not supported. It was; the filter had a shorter opinion
                // than the program.
                //
                // Still only a convenience: whatever comes back is detected by
                // content, and a file whose extension lies is refused on its
                // bytes further in (SR-4).
                dialog = dialog.add_filter(
                    "Audio and recordings",
                    &[
                        "mp3", "wav", "flac", "ogg", "oga", "m4a", "mka", "mkv", "webm", "mp4",
                        "m4b",
                    ],
                );
            }
            Some("pdf") => {
                dialog = dialog.add_filter("PDF", &["pdf"]);
            }
            _ => {}
        }
        let picked = dialog.add_filter("All files", &["*"]).pick_files();
        Ok(picked.map(|list| {
            list.iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect()
        }))
    })
    .await
}

/// One row of an exported batch log.
///
/// A **typed** row rather than the frontend's own result object: everything
/// written to the user's disk is re-serialised from these fields here, so the
/// webview chooses the values but never the shape of the file.
#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LogRow {
    file_name: String,
    ok: bool,
    output: String,
    error: String,
    duration_ms: u64,
    input_bytes: u64,
    output_bytes: u64,
    class_applied: String,
}

/// Write the last batch's outcome to a file the user chooses.
///
/// The path comes from the host-side save dialog — the same `rfd` route
/// `pick_files` uses — so no filesystem capability is granted to the webview
/// and the plugin gate in `xtask/src/desktop.rs` stays intact. Returns the
/// path written, or `None` when the dialog was dismissed.
#[tauri::command]
async fn export_batch_log(rows: Vec<LogRow>) -> Result<Option<String>, String> {
    blocking(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save the conversion log")
            .set_file_name("openconvert-log.json")
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return Ok(None);
        };
        let doc = serde_json::json!({
            "tool": concat!("openconvert-desktop ", env!("CARGO_PKG_VERSION")),
            "converted": rows.iter().filter(|r| r.ok).count(),
            "failed": rows.iter().filter(|r| !r.ok).count(),
            "files": rows,
        });
        let text = serde_json::to_string_pretty(&doc)
            .map_err(|e| format!("the log could not be written: {e}"))?;
        std::fs::write(&path, text)
            .map_err(|e| format!("could not write {}: {e}", path.display()))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

/// Save plain text through the host's save dialog.
///
/// `None` when the dialog is dismissed — a cancelled save is a normal outcome,
/// not an error, and the caller shows nothing rather than an alarm.
///
/// **The path comes from the dialog, never from the webview.** The frontend
/// supplies the text and a suggested name; where it lands is the user's answer
/// to an OS prompt, which is what keeps an arbitrary path from crossing the
/// boundary (I5).
#[tauri::command]
async fn save_text_file(suggested_name: String, text: String) -> Result<Option<String>, String> {
    blocking(move || {
        // The suggestion is untrusted: it is derived from a filename. Strip it
        // to a leaf so a crafted name cannot steer the dialog out of the folder
        // the user is looking at.
        let leaf = Path::new(&suggested_name).file_name().map_or_else(
            || "transcript.txt".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        let Some(path) = rfd::FileDialog::new()
            .set_title("Save text")
            .set_file_name(&leaf)
            .add_filter("Text", &["txt"])
            .add_filter("Markdown", &["md"])
            .save_file()
        else {
            return Ok(None);
        };
        std::fs::write(&path, text).map_err(|e| format!("could not write {}: {e}", show(&path)))?;
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
}

// ---------------------------------------------------------------------------
// The work behind the commands
// ---------------------------------------------------------------------------

/// Run a blocking job off the IPC thread.
///
/// Every command here touches the filesystem and may start a confined worker.
/// On the main thread that freezes the window — including the Cancel button,
/// which is the one control that has to keep working while a batch runs.
async fn blocking<T, F>(job: F) -> Result<T, String>
where
    F: FnOnce() -> Result<T, String> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|_| "the background task did not finish; nothing was written".to_string())?
}

fn probe_one(path: &Path) -> Result<ProbeResult, String> {
    let policy = policy_from(&UserConfig::load());
    let mut table = openconvert_run::handles::HandleTable::new();
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());

    let facts =
        openconvert_run::detect::detect(path, &mut table).map_err(|e| readable(path, &e))?;
    let bytes =
        openconvert_run::detect::bytes_of(&facts, &mut table).map_err(|e| readable(path, &e))?;
    let input_bytes = bytes.len() as u64;
    let props = openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool);

    let s = facts.sniff();
    let name = DisplayName::new(&file_name_of(path));

    Ok(ProbeResult {
        path: path.to_string_lossy().into_owned(),
        file_name: name.as_str().to_string(),
        detected: s.detected.to_string(),
        declared: s.declared.map(|d| d.to_string()),
        mismatched: s.mismatched(),
        polyglot: s.polyglot,
        name_altered: name.altered(),
        input_bytes,
        properties: properties_json(props),
    })
}

/// Infallible by construction: a file that cannot be read, or a target that
/// cannot be parsed, becomes a **blocking warning on that file** rather than an
/// error for the drop. One bad file in forty must not cost the other
/// thirty-nine their preview.
fn plan_for(paths: &[String], target_format: &str) -> PlanPreview {
    let policy = policy_from(&UserConfig::load());
    let env = build_environment();

    let mut steps = Vec::new();
    let mut warnings = Vec::new();
    let mut total_input_bytes = 0_u64;
    let mut any_executable = false;
    let mut any_file = false;

    for raw in paths {
        let path = PathBuf::from(raw);
        let name = DisplayName::new(&file_name_of(&path)).as_str().to_string();
        any_file = true;

        let mut table = openconvert_run::handles::HandleTable::new();
        let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());

        let facts = match openconvert_run::detect::detect(&path, &mut table) {
            Ok(f) => f,
            Err(e) => {
                warnings.push(WarningJson {
                    file_name: name,
                    blocking: true,
                    message: readable(&path, &e),
                });
                continue;
            }
        };
        let bytes = match openconvert_run::detect::bytes_of(&facts, &mut table) {
            Ok(b) => b,
            Err(e) => {
                warnings.push(WarningJson {
                    file_name: name,
                    blocking: true,
                    message: readable(&path, &e),
                });
                continue;
            }
        };
        total_input_bytes += bytes.len() as u64;
        let props = openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool);

        let s = facts.sniff();
        let target = match parse_target(target_format) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(WarningJson {
                    file_name: name,
                    blocking: true,
                    message: e,
                });
                continue;
            }
        };

        // Armed by what the target names -- the same `Target::arms` the
        // dropdown, the tool path and the CLI use. The PREVIEW must arm
        // exactly as the run will, or it previews a refusal for a conversion
        // that then succeeds, or the reverse.
        let armed = policy.armed_for(target);
        let plan = route(
            openconvert_core::plan::PlanRequest {
                input: s.detected,
                target,
                polyglot: s.polyglot,
            },
            props,
            &armed,
            &env,
        );

        if plan.is_executable() {
            any_executable = true;
        }
        for step in plan.steps() {
            steps.push(step_json(&name, step, target.output_format(s.detected)));
        }
        for w in plan.warnings() {
            warnings.push(WarningJson {
                file_name: name.clone(),
                blocking: w.is_blocking(),
                message: describe(w),
            });
        }
    }

    PlanPreview {
        // Executable means "at least one file would run and nothing blocks".
        // A drop where one file is a polyglot and thirty-nine are photos is
        // still a drop the user can press Enter on; the blocked one keeps its
        // own warning row.
        executable: any_file && any_executable,
        steps,
        warnings,
        estimated_duration_ms: 0,
        total_input_bytes,
    }
}

#[allow(clippy::too_many_lines)]
fn run_batch(
    app: &tauri::AppHandle,
    paths: &[String],
    target_format: &str,
    cancel: &AtomicBool,
    choices: Option<&[ChoiceJson]>,
) -> Result<(Vec<ConversionResult>, Vec<UndoItem>), String> {
    let policy = policy_from(&UserConfig::load());
    let env = build_environment();
    let target = parse_target(target_format)?;
    // Settings are read once per batch, not per file: a mid-batch settings
    // change applying to half the drop would make one batch disagree with
    // itself about where its outputs went.
    let settings = OutputSettings::load();

    // One pool for the batch: reuse only means anything across files, and
    // `WorkerReuse` decides whether two files may share a worker at all.
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());
    let mut results = Vec::with_capacity(paths.len());
    let mut created = Vec::new();
    let total = paths.len();
    let date = today();

    // THE APP NEVER WROTE ONE OF THESE.
    //
    // `openconvert batch` journalled every file it touched; the desktop shell,
    // which is how almost everyone actually uses this program, journalled
    // nothing. The History panel read the journal directory faithfully and
    // found it empty, for every conversion anyone had ever run in the app.
    //
    // A failure to open the journal does not fail the batch: recording a
    // conversion is worth less than performing it. It is reported once, to
    // stderr, rather than per file.
    let mut journal = match openconvert_run::state::journal::Journal::open(&batch_id()) {
        Ok(j) => Some(j),
        Err(e) => {
            eprintln!("openconvert: history will not record this batch ({e})");
            None
        }
    };

    for (index, raw) in paths.iter().enumerate() {
        let path = PathBuf::from(raw);
        let name = DisplayName::new(&file_name_of(&path)).as_str().to_string();

        // MATCHED BY PATH, NOT BY POSITION. The shell groups a drop by chosen
        // target before calling this, so the batch's paths are a subset of the
        // drop in an order this function does not control; indexing would
        // attribute one file's choice to another and quietly poison the very
        // measurement these fields exist to provide.
        let choice = choices.and_then(|cs| cs.iter().find(|c| c.path == *raw));

        if cancel.load(Ordering::SeqCst) {
            emit(
                app,
                &ProgressEvent {
                    index,
                    total,
                    file_name: name.clone(),
                    phase: "cancelled",
                },
            );
            results.push(failure(&name, "cancelled before this file started", 0, 0));
            continue;
        }

        emit(
            app,
            &ProgressEvent {
                index,
                total,
                file_name: name.clone(),
                phase: "started",
            },
        );
        let start = std::time::Instant::now();

        match convert_one(
            &path, target, &policy, &env, &mut pool, &settings, index, &date,
        ) {
            Ok(done) => {
                created.push(UndoItem {
                    output: done.output.clone(),
                    receipt: PathBuf::new(),
                    replaced_from: done.replaced,
                });
                emit(
                    app,
                    &ProgressEvent {
                        index,
                        total,
                        file_name: name.clone(),
                        phase: "done",
                    },
                );
                journal_append(
                    journal.as_mut(),
                    JournalEntry {
                        source_name: name.clone(),
                        content_id: done.detail.content_id.clone(),
                        plan_hash: 0,
                        output_name: file_name_of(&done.output),
                        outcome: JournalOutcome::Completed,
                        output_bytes: done.output_bytes,
                        receipt_path: (!done.receipt.as_os_str().is_empty())
                            .then(|| done.receipt.to_string_lossy().into_owned()),
                        source_folder: folder_name_of(&path),
                        suggested: choice.map(|c| c.suggested.clone()),
                        suggested_score: choice.map(|c| c.suggested_score),
                        chosen_rank: choice.map(|c| c.chosen_rank),
                    },
                );
                results.push(ConversionResult {
                    file_name: name,
                    output_path: show(&done.output),
                    receipt_path: show(&done.receipt),
                    success: true,
                    error_message: None,
                    duration_ms: millis(start),
                    input_bytes: done.input_bytes,
                    output_bytes: done.output_bytes,
                    class_applied: done.class_applied,
                    removed_metadata: done.removed_metadata,
                    content_id: done.detail.content_id.clone(),
                    receipt_detail: Some(done.detail),
                });
            }
            Err(message) => {
                emit(
                    app,
                    &ProgressEvent {
                        index,
                        total,
                        file_name: name.clone(),
                        phase: "failed",
                    },
                );
                journal_append(
                    journal.as_mut(),
                    JournalEntry {
                        source_name: name.clone(),
                        content_id: String::new(),
                        plan_hash: 0,
                        output_name: String::new(),
                        outcome: JournalOutcome::Failed {
                            reason: message.clone(),
                        },
                        output_bytes: 0,
                        receipt_path: None,
                        // Recorded on failures too. A suggestion the user
                        // accepted and which then could not run is not a
                        // correct suggestion, and a measurement that counted
                        // only successes would never see it.
                        source_folder: folder_name_of(&path),
                        suggested: choice.map(|c| c.suggested.clone()),
                        suggested_score: choice.map(|c| c.suggested_score),
                        chosen_rank: choice.map(|c| c.chosen_rank),
                    },
                );
                results.push(failure(&name, &message, millis(start), 0));
            }
        }
    }

    Ok((results, created))
}

/// The NAME of the folder a file sits in, for the prediction history.
///
/// Never the path -- see `JournalEntry::source_folder`.
fn folder_name_of(path: &Path) -> Option<String> {
    path.parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
}

/// A batch id: the local date and time, which sorts and reads.
///
/// The journal file is named after it, and `read_history` orders batches by
/// the file's mtime, so this only has to be unique enough that two batches a
/// second apart do not share a file.
fn batch_id() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("{secs}-{}", std::process::id())
}

/// Append to the journal if there is one, and never fail the batch over it.
fn journal_append(
    journal: Option<&mut openconvert_run::state::journal::Journal>,
    entry: JournalEntry,
) {
    if let Some(j) = journal {
        if let Err(e) = j.append(&entry) {
            eprintln!("openconvert: history entry not recorded ({e})");
        }
    }
}

/// The per-batch slice of the user configuration that changes where outputs
/// land and what the app records about them.
struct OutputSettings {
    destination: String,
    naming_template: String,
    write_receipts: bool,
}

impl OutputSettings {
    fn load() -> Self {
        let effective = ConfigJson::from(&UserConfig::load());
        Self {
            destination: effective.output_destination,
            naming_template: effective.naming_template,
            write_receipts: effective.write_receipts,
        }
    }
}

/// Where this run's outputs go.
enum Destination {
    /// Next to each input (default).
    BesideInput,
    /// The user's Desktop folder.
    Desktop(PathBuf),
    /// The user's Downloads folder.
    Downloads(PathBuf),
    /// Next to each input, and the input moves to the quarantine afterwards.
    ReplaceSource,
}

/// Resolve the configured destination, or a per-file error message when it
/// cannot be honoured ("Desktop" is best-effort: the folder can be missing
/// or redirected).
fn resolve_destination(settings: &OutputSettings) -> Result<Destination, String> {
    match settings.destination.as_str() {
        "desktop" => desktop_dir().map(Destination::Desktop).ok_or_else(|| {
            "the Desktop folder could not be found for this file; \
                 outputs stayed unwritten"
                .to_string()
        }),
        "downloads" => downloads_dir().map(Destination::Downloads).ok_or_else(|| {
            "the Downloads folder could not be found for this file; outputs stayed unwritten"
                .to_string()
        }),
        "replace_source" => Ok(Destination::ReplaceSource),
        _ => Ok(Destination::BesideInput),
    }
}

/// The user's Desktop folder, or nothing — never a guessed fallback, because
/// writing somewhere the user did not ask for is worse than saying no.
fn desktop_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    let dir = PathBuf::from(home).join("Desktop");
    dir.is_dir().then_some(dir)
}

/// The user's Downloads folder, or nothing.
///
/// Same rule as [`desktop_dir`]: the folder is located or the write is refused.
/// Windows lets Downloads be relocated, and a guessed path would write
/// somewhere the user cannot find.
fn downloads_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")?;
    let dir = PathBuf::from(home).join("Downloads");
    dir.is_dir().then_some(dir)
}

/// Where replaced originals wait out their undo window.
///
/// Under our own state directory: the sweep rule (`03` §13) forbids cleanup
/// over any shared location, and a quarantine outside it would be exactly
/// that.
fn quarantine_dir() -> PathBuf {
    paths::state_dir().join("replaced")
}

/// What one successful conversion produced.
struct Converted {
    output: PathBuf,
    receipt: PathBuf,
    input_bytes: u64,
    output_bytes: u64,
    class_applied: String,
    removed_metadata: Vec<String>,
    /// The receipt as recorded, for the done screen.
    detail: ReceiptDetailJson,
    /// `(quarantine path, original path)` when this run replaced an original.
    replaced: Option<(PathBuf, PathBuf)>,
}

#[allow(clippy::too_many_arguments)]
fn convert_one(
    path: &Path,
    target: Target,
    policy: &Policy,
    env: &Environment,
    pool: &mut openconvert_run::pool::WorkerPool,
    settings: &OutputSettings,
    index: usize,
    date: &str,
) -> Result<Converted, String> {
    let mut table = openconvert_run::handles::HandleTable::new();

    let facts =
        openconvert_run::detect::detect(path, &mut table).map_err(|e| readable(path, &e))?;
    let bytes =
        openconvert_run::detect::bytes_of(&facts, &mut table).map_err(|e| readable(path, &e))?;
    let input_bytes = bytes.len() as u64;
    let props = openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), pool);

    let s = facts.sniff();
    // Same arming as the preview and the CLI: see `Target::arms`.
    let armed = policy.armed_for(target);
    let plan = route(
        openconvert_core::plan::PlanRequest {
            input: s.detected,
            target,
            polyglot: s.polyglot,
        },
        props,
        &armed,
        env,
    );

    // Refusal is the absence of a plan. Say why, and write nothing.
    if !plan.is_executable() {
        return Err(refusal_message(&plan));
    }

    let dest = match resolve_destination(settings)? {
        Destination::Desktop(dir) | Destination::Downloads(dir) => dir,
        Destination::BesideInput | Destination::ReplaceSource => path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };
    let replacing = matches!(resolve_destination(settings)?, Destination::ReplaceSource);

    // THE NAME COMES FROM THE PLAN, NOT THE TARGET.
    //
    // `Target::output_format` answers "what did the user ask for"; for an
    // operation that is "the format it came in as". What goes in the FILE is
    // whatever the last step produces, and for noise removal those two differ
    // whenever the plan cannot reach the original encoder. Naming by the
    // target produced a WAV called `.mp3`.
    //
    // The fallback is the target, for a step that names no format of its own —
    // a stream copy, a metadata strip — which is correct, because those do not
    // change the format either.
    let out_format = plan
        .output_format()
        .unwrap_or_else(|| target.output_format(s.detected));
    let stem = path.file_stem().map_or_else(
        || "output".to_string(),
        |x| x.to_string_lossy().into_owned(),
    );
    let ext = out_format
        .row()
        .map_or_else(|| "bin".to_string(), |r| r.extension.to_string());
    // The canonical renderer: total over any input filename, traversal-proof,
    // and the same module the CLI renders through.
    let out_name = openconvert_run::naming::render(
        &settings.naming_template,
        &openconvert_run::naming::RenderContext {
            name: &stem,
            ext: &ext,
            date,
            index: index + 1,
        },
    )
    .map_err(|e| e.to_string())?;

    let outcome = openconvert_run::exec::execute_with(
        &plan,
        &facts,
        &mut table,
        &openconvert_run::exec::ExecOptions {
            dest_dir: dest.clone(),
            output_name: out_name.clone(),
            write_receipt: false,
            replace_source: false,
            source_path: None,
        },
        policy,
        pool,
    )
    .map_err(|e| e.to_string())?;

    // Finish every fallible receipt-serialization step before replacing an
    // original. After replacement, only the database commit remains, and its
    // failure path knows how to compensate the filesystem.
    let receipt_body = if settings.write_receipts {
        let body = serde_json::to_string_pretty(&outcome.record).map_err(|e| e.to_string())?;
        Some(enrich_for_store(&body, &file_name_of(path)).map_err(|e| e.to_string())?)
    } else {
        None
    };

    // Replace-originals: the original moves aside only after the conversion
    // succeeded, and it moves into our own quarantine rather than being
    // deleted, so Undo can put it back. A same-format operation (strip
    // metadata on a JPEG) wrote its output under a suffixed name because the
    // original was still there; that output takes over the vacated name.
    let mut replaced = None;
    let mut final_output = outcome.output.clone();
    if replacing {
        let quarantined = replace_original(&outcome.output, path, &out_name)?;
        if out_name.eq_ignore_ascii_case(&file_name_of(path))
            && outcome.output != path
            && outcome.output.parent() == path.parent()
        {
            final_output = path.to_path_buf();
        }
        replaced = Some((quarantined, path.to_path_buf()));
    }

    // A desktop conversion commits only when both the filesystem result and
    // its history row exist. Compensate every completed filesystem step if
    // SQLite refuses the record.
    let receipt = if let Some(body) = receipt_body.as_deref() {
        commit_desktop_receipt(
            body,
            &file_name_of(path),
            true,
            &final_output,
            replaced.as_ref(),
        )?
    } else {
        PathBuf::new()
    };

    // The stored row carries `input_name` so Settings can say what produced
    // the output without ever storing an absolute source path.
    let removed_metadata = outcome
        .record
        .steps
        .iter()
        .flat_map(|st| st.removed.iter())
        .map(|r| DisplayName::new(r).as_str().to_string())
        .collect();

    let detail = receipt_detail(&outcome.record);

    Ok(Converted {
        input_bytes,
        output_bytes: outcome.record.output_bytes,
        class_applied: class_display(plan.class()),
        removed_metadata,
        detail,
        replaced,
        output: final_output,
        receipt,
    })
}

/// Move a successfully-replaced original into the quarantine.
///
/// `desired_name` is what the conversion intended to write. When it is the
/// original's own name (a same-format operation), the output takes over the
/// vacated name; otherwise it keeps its own.
///
/// Returns the quarantine path. Order matters and is crash-safe without ever
/// losing bytes: rename the original to a temp name beside itself (same
/// volume), move the output onto the original's name if that is where it
/// belonged, copy the original into the quarantine under our state
/// directory, then delete the temp name. Every step leaves either the
/// original, the output, or both intact.
fn replace_original(output: &Path, original: &Path, desired_name: &str) -> Result<PathBuf, String> {
    replace_original_in(output, original, desired_name, &quarantine_dir())
}

fn replace_original_in(
    output: &Path,
    original: &Path,
    desired_name: &str,
    quarantine: &Path,
) -> Result<PathBuf, String> {
    use std::io::Write;

    let parent = original.parent().unwrap_or_else(|| Path::new("."));
    let holding = parent.join(format!(
        "{}.replacing-{}",
        file_name_of(original),
        std::process::id()
    ));

    std::fs::rename(original, &holding).map_err(|e| {
        format!(
            "{} could not be set aside for replacement: {e}",
            show(original)
        )
    })?;

    // Same-format operation: the pipeline wrote beside the still-existing
    // original, so its O_EXCL output carries a suffix. On the vacated name,
    // the output takes the name the user asked for. Windows names
    // case-insensitively, so the comparison is too.
    let wants_vacated_name = desired_name.eq_ignore_ascii_case(&file_name_of(original));
    if wants_vacated_name && output != original && output.parent() == Some(parent) {
        if let Err(rename_err) = std::fs::rename(output, original) {
            // Put the original back before reporting; nothing was lost.
            let _ = std::fs::rename(&holding, original);
            return Err(format!(
                "{} could not take the original's place: {rename_err}",
                show(output)
            ));
        }
    }

    if let Err(error) = std::fs::create_dir_all(quarantine) {
        // Windows can report ERROR_ALREADY_EXISTS when two conversions race
        // to create the same directory. The postcondition is what matters.
        if !quarantine.is_dir() {
            return Err(format!("the quarantine could not be created: {error}"));
        }
    }
    let stamp = now_secs();
    let unique = REPLACEMENT_ID.fetch_add(1, Ordering::Relaxed);
    let quarantined = quarantine.join(format!(
        "{stamp}-{}-{unique}-{}",
        std::process::id(),
        file_name_of(&holding)
    ));
    let copy_err = (|| -> std::io::Result<()> {
        let mut from = std::fs::File::open(&holding)?;
        let mut to = std::fs::File::create_new(&quarantined)?;
        std::io::copy(&mut from, &mut to)?;
        to.flush()?;
        Ok(())
    })()
    .err();

    // The holding name goes away once the copy has landed; if the copy
    // failed, the holding name survives so the bytes survive.
    if let Some(e) = copy_err {
        return Err(format!(
            "{} was replaced but could not be archived: {e}. It is still at {}",
            show(original),
            show(&holding)
        ));
    }
    let _ = std::fs::remove_file(&holding);
    Ok(quarantined)
}

/// Put a replaced original back after Undo.
///
/// Rename first (free when the quarantine and the file share a volume),
/// falling back to copy-then-remove across volumes. The destination must be
/// free — Undo deletes the output that occupied it before restoring.
fn restore_original(quarantined: &Path, original: &Path) -> Result<(), String> {
    match std::fs::rename(quarantined, original) {
        Ok(()) => Ok(()),
        Err(_) => {
            let copy_err = (|| -> std::io::Result<()> {
                std::fs::copy(quarantined, original)?;
                std::fs::remove_file(quarantined)
            })();
            copy_err.map_err(|e| e.to_string())
        }
    }
}

/// Seconds since the epoch, saturating.
fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The receipt body as the store receives it: the original JSON untouched,
/// plus the input's name so a listing can say what produced the output. If
/// the body is not valid JSON (a foreign writer), it is stored as-is — the
/// store skips unparseable records on read anyway.
fn enrich_for_store(body: &str, source_name: &str) -> Result<String, serde_json::Error> {
    let mut value: serde_json::Value = serde_json::from_str(body)?;
    if let Some(map) = value.as_object_mut() {
        map.entry("input_name".to_string())
            .or_insert_with(|| Value::String(source_name.to_string()));
    }
    serde_json::to_string(&value)
}

/// Today as `YYYY-MM-DD`, for `{date}` in naming templates.
fn today() -> String {
    // No chrono dependency: days since the epoch to civil date, by Howard
    // Hinnant's algorithm. Runs once per batch.
    let secs = now_secs();
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// Infallible for the same reason as [`plan_for`]: a file that cannot be read
/// groups under `Unknown` and is offered nothing, rather than taking the rest
/// of the drop's suggestions down with it.
/// Rasterise one file (or one page of it) through the confined worker.
fn render_preview(
    path: &Path,
    page: u32,
    max_px: u32,
    ops: &[String],
) -> Result<FilePreview, String> {
    let policy = policy_from(&UserConfig::load());
    let mut table = openconvert_run::handles::HandleTable::new();
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());

    let facts =
        openconvert_run::detect::detect(path, &mut table).map_err(|e| readable(path, &e))?;
    let mut bytes =
        openconvert_run::detect::bytes_of(&facts, &mut table).map_err(|e| readable(path, &e))?;
    let limits = policy.base_limits();
    let props = openconvert_run::probe::probe(&facts, &bytes, &limits, &mut pool);

    let detected = facts.sniff().detected;
    let kind = detected
        .kind()
        .ok_or_else(|| "this file is not a kind we can draw".to_string())?;
    let bin = EngineBin::for_media(kind)
        .ok_or_else(|| format!("no engine renders {}", detected.name()))?;

    // ASK THE WORKER HOW MANY PAGES THERE ARE.
    //
    // This read `props`, and `probe()` has no document branch -- it returns
    // `Properties::None` for every PDF and says so in a comment. So the match
    // fell to `_ => 1` for EVERY document, whatever its length, and one number
    // being wrong produced three separate bug reports: the reorder board drew a
    // single card, the eye preview showed a single page, and the signing column
    // offered a single sheet.
    //
    // `oc-pdf` walks the page tree it has already parsed and answers with an
    // integer. A document that will not answer keeps the old assumption of one
    // page, which is the honest floor: the file is open, so it has at least one.
    let page_count = match props {
        Properties::Document { pages, .. } => pages.max(1),
        _ if bin == EngineBin::Pdf => pool
            .run(
                bin,
                facts.provenance(),
                &limits,
                bytes.clone(),
                "txt",
                &[("op".to_string(), "pages".to_string())],
            )
            .ok()
            .and_then(|(out, _, _)| String::from_utf8(out).ok())
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(1)
            .max(1),
        _ => 1,
    };
    let page = page.clamp(1, page_count);
    let mut geometry_error = None;
    let geometry = if bin == EngineBin::Pdf {
        pool.run(
            bin,
            facts.provenance(),
            &limits,
            bytes.clone(),
            "txt",
            &[
                ("op".into(), "geometry".into()),
                ("page".into(), page.to_string()),
            ],
        )
        .map_err(|e| format!("Could not read PDF page dimensions: {e}"))
        .and_then(|(out, _, _)| {
            serde_json::from_slice::<(f32, f32, i64)>(&out)
                .map_err(|e| format!("Invalid PDF page dimensions: {e}"))
        })
        .map_err(|e| {
            geometry_error = Some(e);
        })
        .ok()
    } else {
        None
    };

    // Only parameters the worker declares. `oc-pdf` takes `page`; `oc-images`
    // takes none, and an unrecognised parameter is an ERROR there rather than a
    // silent ignore -- so an empty slice is the correct call, not a lazy one.
    //
    // ONE-BASED HERE, ZERO-BASED ON THE WIRE, AND THE CONVERSION WAS MISSING.
    //
    // `page` is one-based everywhere this function can see: the argument comes
    // from a page number a person is looking at, and it is clamped against
    // `page_count` on that basis. `oc-pdf` counts from zero -- it defaults to 0
    // for the first page and refuses `page >= pages`. The number was forwarded
    // without the subtraction, and all three consequences were live:
    //
    //   * a ONE-PAGE document asked for page 1 of 1 and was refused with
    //     "this document has 1 page(s); asked for page 1 (zero-based)" -- the
    //     reported error, verbatim, and the reason single-page PDFs never
    //     previewed;
    //   * every "page 1" preview of a longer document rendered page TWO, which
    //     is silent because the wrong page is still a page;
    //   * the LAST page of any document always failed.
    //
    // It survived because the preview harness mocks `preview_file` and answers
    // with a picture for any page number at all, so the one path that could
    // have shown this was the one path that could not.
    let mut compression = None;
    if let Some(quality) = ops.iter().find_map(|op| op.strip_prefix("compress:")) {
        let original = bytes.len();
        let to = if bin == EngineBin::Pdf {
            "pdf"
        } else {
            detected.name()
        };
        let (smaller, _, _) = pool
            .run(
                bin,
                facts.provenance(),
                &limits,
                bytes.clone(),
                to,
                &[
                    ("op".into(), "compress".into()),
                    ("quality".into(), quality.into()),
                ],
            )
            .map_err(|e| e.to_string())?;
        if smaller.len() < original {
            bytes = smaller;
        }
        compression = Some((original as u64, bytes.len() as u64));
    }

    let params: Vec<(String, String)> = if bin == EngineBin::Pdf {
        vec![
            ("page".to_string(), (page - 1).to_string()),
            ("max_px".to_string(), max_px.to_string()),
        ]
    } else {
        Vec::new()
    };

    let (png, _removed, _confinement) = pool
        .run(bin, facts.provenance(), &limits, bytes, "png", &params)
        .map_err(|e| e.to_string())?;

    // Apply previews in the same order as the editor and saved workflow.
    let mut scaled = downscale_png(&png, max_px, &[])?;
    for op in ops {
        if op == "image-remove-background" {
            scaled = preview_background_removal(&scaled.0, &mut pool, facts.provenance(), &limits)
                .unwrap_or(scaled);
        } else if op == "image-invert" || op == "image-greyscale" || op.starts_with("pdf-") {
            scaled = downscale_png(&scaled.0, max_px, std::slice::from_ref(op))?;
        }
    }

    Ok(FilePreview {
        data_uri: format!("data:image/png;base64,{}", base64(&scaled.0)),
        width: scaled.1,
        height: scaled.2,
        page_count,
        page,
        // Not yet reported: `oc-pdf` returns a raster, not the MediaBox. Stated
        // as absent rather than guessed — A4 and A3 share a ratio, so a paper
        // size cannot be recovered from pixels alone.
        page_geometry_error: geometry_error,
        page_width_pt: geometry.map(|g| g.0),
        page_height_pt: geometry.map(|g| g.1),
        page_rotation: geometry.map(|g| g.2),
        original_bytes: compression.map(|c| c.0),
        compressed_bytes: compression.map(|c| c.1),
    })
}

/// Cut the background out of an already-downscaled preview PNG.
///
/// Returns `None` on any failure, and the caller keeps the un-matted preview:
/// a stage that goes blank because a model was switched off between the click
/// and the render is worse than one showing the picture it started from. The
/// Save path reports model problems properly, with the reason.
fn preview_background_removal(
    png: &[u8],
    pool: &mut openconvert_run::pool::WorkerPool,
    provenance: openconvert_core::facts::Provenance,
    limits: &openconvert_core::limits::Limits,
) -> Option<(Vec<u8>, u32, u32)> {
    // The same choice `tools.rs` makes, for the same reason, and the quality
    // tier carries its fallback model with it.
    let quality = openconvert_run::tools::modnet_ready();
    let names: &[&str] = if quality {
        &["modnet", "u2netp"]
    } else {
        &["u2netp"]
    };
    let weights: Vec<Vec<u8>> = names
        .iter()
        .map(|n| openconvert_run::models::verified_bytes(n).ok())
        .collect::<Option<Vec<_>>>()?;

    let task = if quality {
        "remove-background-quality"
    } else {
        "remove-background"
    };
    let (out, _, _) = pool
        .run_with_models(
            EngineBin::Ai,
            provenance,
            limits,
            png.to_vec(),
            &weights,
            "png",
            &[("task".to_string(), task.to_string())],
        )
        .ok()?;

    use image::GenericImageView as _;
    let img = image::load_from_memory_with_format(&out, image::ImageFormat::Png).ok()?;
    let (w, h) = img.dimensions();
    Some((out, w, h))
}

/// Decode our worker's PNG, scale the long edge to `max_px`, re-encode.
///
/// Decoding in-process is safe here in the way the project already means it:
/// `image` is memory-safe Rust, and the input is a PNG this app produced, not
/// the file the user supplied.
fn downscale_png(png: &[u8], max_px: u32, ops: &[String]) -> Result<(Vec<u8>, u32, u32), String> {
    use image::GenericImageView;

    // The PDF worker already renders at the requested size. Re-encoding that
    // PNG in the host dominated high-resolution zoom latency, with no pixel change.
    if ops.is_empty() {
        let (width, height) =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png)
                .into_dimensions()
                .map_err(|e| format!("the rendered preview could not be read: {e}"))?;
        if max_px == 0 || width.max(height) <= max_px {
            return Ok((png.to_vec(), width, height));
        }
    }

    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|e| format!("the rendered preview could not be read: {e}"))?;
    let (w, h) = img.dimensions();
    let longest = w.max(h);

    let mut out = if longest > max_px && max_px > 0 {
        img.thumbnail((w * max_px / longest).max(1), (h * max_px / longest).max(1))
    } else {
        img
    };

    // Pixel operations run AFTER scaling, on our own decoded buffer — the
    // untrusted decode already happened behind the sandbox, and these touch
    // nothing but colour channels. Order matters only for cost: inverting a
    // thumbnail is cheaper than inverting a 48 Mpx original and throwing most
    // of it away.
    for op in ops {
        match op.as_str() {
            s if s.starts_with("pdf-crop:") => {
                let v = s[9..]
                    .split(',')
                    .map(str::parse::<f64>)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| "invalid preview crop")?;
                if v.len() != 4
                    || v.iter()
                        .any(|x| !x.is_finite() || !(0.0..=49.0).contains(x))
                {
                    return Err("invalid preview crop".into());
                }
                let (w, h) = out.dimensions();
                let x = (f64::from(w) * v[0] / 100.0) as u32;
                let y = (f64::from(h) * v[3] / 100.0) as u32;
                let cw = (f64::from(w) * (100.0 - v[0] - v[2]) / 100.0) as u32;
                let ch = (f64::from(h) * (100.0 - v[1] - v[3]) / 100.0) as u32;
                out = out.crop_imm(x, y, cw.max(1).min(w - x), ch.max(1).min(h - y));
            }
            "pdf-turn:90" => out = out.rotate90(),
            "pdf-turn:180" => out = out.rotate180(),
            "pdf-turn:270" => out = out.rotate270(),
            "image-invert" => out.invert(),
            "image-greyscale" => out = out.grayscale(),
            // Background removal and everything else is the engine's, not ours.
            _ => {}
        }
    }
    let (ow, oh) = out.dimensions();

    let mut buf = std::io::Cursor::new(Vec::new());
    out.write_to(&mut buf, image::ImageFormat::Png)
        .map_err(|e| format!("the preview could not be encoded: {e}"))?;
    Ok((buf.into_inner(), ow, oh))
}

/// Base64, standard alphabet with padding.
///
/// Hand-rolled rather than pulled in: it is twenty lines with no configuration,
/// and a dependency added for a data URI is a dependency to review forever.
fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn predictions_for(paths: &[String]) -> Vec<Prediction> {
    let policy = policy_from(&UserConfig::load());
    let env = build_environment();
    let history = History::load();

    // Group by detected format, preserving first-seen order. A mixed drop is
    // several decisions, not one: PNGs and MP3s do not share a target.
    let mut groups: Vec<(FormatId, Vec<String>)> = Vec::new();
    for raw in paths {
        let path = PathBuf::from(raw);
        let mut table = openconvert_run::handles::HandleTable::new();
        let detected = openconvert_run::detect::detect(&path, &mut table)
            .map_or(FormatId::Unknown, |f| f.sniff().detected);
        match groups.iter_mut().find(|(id, _)| *id == detected) {
            Some((_, list)) => list.push(raw.clone()),
            None => groups.push((detected, vec![raw.clone()])),
        }
    }

    let batch_size = u32::try_from(paths.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(groups.len());

    for (input, group_paths) in groups {
        let first = group_paths.first().map(PathBuf::from);

        // Rank against the group's first file, not against `Properties::None`.
        //
        // Some requirements are properties of the FILE — `StreamsCompatible`
        // asks whether *these* codecs can be copied into that container — so
        // routing with no properties answers a different question from the one
        // the plan preview will answer a moment later. Probing one file per
        // group is the cheap half of being right; `get_plan` probes every file.
        let props = first.as_deref().map_or(Properties::None, probe_properties);
        let candidates = routable_targets(input, props, &policy, &env);

        // Once per group, not once per candidate: every suggestion in this
        // group came from the same folder, and this is a string allocation.
        let folder_key = first
            .as_deref()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();

        let suggestions: Vec<SuggestionJson> = candidates
            .iter()
            .map(|&(target, ref class_name)| {
                let to = target.output_format(input);
                let signals = Signals {
                    input,
                    history_count: history.from(input),
                    history_to_target: history.pair(input, to),
                    folder_history_count: history.folder_from(&folder_key, input),
                    folder_history_to_target: history.folder_pair(&folder_key, input, to),
                    folder_pattern: first.as_deref().map_or(0, |p| folder_pattern(p, input, to)),
                    sibling_same_stem: first.as_deref().is_some_and(|p| sibling_exists(p, to)),
                    batch_size,
                    folder: folder_category(first.as_deref()),
                };
                let s = predict::Suggestion {
                    target,
                    score: predict::score(&signals, target),
                };
                SuggestionJson {
                    target: format_name(to),
                    score: s.score,
                    class_name: class_name.clone(),
                    armed: predict::should_arm(&s),
                }
            })
            .collect();

        let mut suggestions = suggestions;
        // Stable sort, best first: equal scores keep table order so the list
        // does not flicker between two identical drops.
        suggestions.sort_by(|a, b| b.score.total_cmp(&a.score));

        let why = explain(
            input,
            suggestions.first(),
            &history,
            first.as_deref(),
            batch_size,
        );

        out.push(Prediction {
            input: input.to_string(),
            kind: kind_name(input),
            paths: group_paths,
            suggestions,
            why,
        });
    }

    out
}

// ---------------------------------------------------------------------------
// Rendering — every one of these turns a library value into a string the UI
// shows verbatim. The UI does no formatting of its own and parses none of it.
// ---------------------------------------------------------------------------

fn properties_json(props: Properties) -> Option<PropertiesJson> {
    Some(match props {
        Properties::Image {
            width,
            height,
            has_alpha,
            frames,
        } => PropertiesJson {
            kind: "image".into(),
            width: Some(width),
            height: Some(height),
            has_alpha: Some(has_alpha),
            frames: Some(frames),
            ..PropertiesJson::default()
        },
        Properties::Audio {
            duration_ms,
            channels,
            sample_rate,
        } => PropertiesJson {
            kind: "audio".into(),
            duration_ms: Some(duration_ms),
            channels: Some(channels),
            sample_rate: Some(sample_rate),
            ..PropertiesJson::default()
        },
        Properties::Video {
            duration_ms,
            width,
            height,
            video,
            audio,
        } => PropertiesJson {
            kind: "video".into(),
            duration_ms: Some(duration_ms),
            width: Some(width),
            height: Some(height),
            video_codec: video.map(|v| format!("{v:?}")),
            audio_codec: audio.map(|a| format!("{a:?}")),
            ..PropertiesJson::default()
        },
        Properties::Document { pages, encrypted } => PropertiesJson {
            kind: "document".into(),
            pages: Some(pages),
            encrypted: Some(encrypted),
            ..PropertiesJson::default()
        },
        Properties::Archive {
            entries,
            depth,
            declared_total_bytes: _,
        } => PropertiesJson {
            kind: "archive".into(),
            entries: Some(entries),
            depth: Some(depth),
            ..PropertiesJson::default()
        },
        Properties::Tabular { rows, columns } => PropertiesJson {
            kind: "tabular".into(),
            rows: Some(rows),
            columns: Some(columns),
            ..PropertiesJson::default()
        },
        // Not an error: a format with no phase-2 parser lands here and must
        // still route. `null` on the wire, so the UI shows the name and nothing
        // it would have to invent.
        Properties::None => return None,
    })
}

fn step_json(file_name: &str, step: &Step, plan_target: FormatId) -> PlanStep {
    PlanStep {
        file_name: file_name.to_string(),
        kind: format!("{:?}", step.kind),
        class_name: format!("{:?}", step.class),
        isolation: isolation_display(step.isolation),
        limits_summary: format!(
            "{} MiB memory, {} decode, {}s wall",
            step.limits.memory_bytes / (1 << 20),
            pixels(step.limits.decode_pixels),
            step.limits.wall_time.as_secs()
        ),
        engine_name: engine_name(step, plan_target),
    }
}

fn isolation_display(isolation: Isolation) -> String {
    match isolation {
        Isolation::InProcess => "in-process".to_string(),
        Isolation::Sandboxed(p) => format!("sandboxed ({})", p.strength()),
    }
}

/// Which engine this step will reach.
///
/// The same question `exec::run_sandboxed` asks, asked before anything runs:
/// the destination format's media kind names the worker. An in-process step
/// names the pure-Rust module that handles it, which is the honest answer —
/// "in-process" is where it runs, not what runs.
fn engine_name(step: &Step, plan_target: FormatId) -> String {
    let to = match step.kind {
        StepKind::Transcode { to, .. } => to,
        StepKind::Extract { to, .. } => to,
        // A page render's destination is its own parameter.
        StepKind::RenderPage { .. } => plan_target,
        // A pixel operation keeps the format; the engine follows from it, the
        // same way an ordinary transcode's does.
        StepKind::Pixel { .. } => plan_target,
        StepKind::StripMetadata => return "openconvert-strip".to_string(),
        // Inference names its own worker, exactly as `exec` does: what decides
        // here is what the STEP does, not what the file is.
        StepKind::Infer { .. } => return "oc-ai".to_string(),
        // Trim and StreamCopy both run on the pure-Rust container crates, in
        // process.
        StepKind::Trim { .. } | StepKind::StreamCopy => return "openconvert-container".to_string(),
    };

    if matches!(step.isolation, Isolation::InProcess) {
        return match to {
            FormatId::Pdf => "openconvert-pdf".to_string(),
            FormatId::Csv | FormatId::Json => "openconvert-tabular".to_string(),
            _ => "image-rs".to_string(),
        };
    }

    to.kind()
        .and_then(EngineBin::for_media)
        .map_or_else(|| "(none)".to_string(), |b| b.file_stem().to_string())
}

fn pixels(n: u64) -> String {
    const M: u64 = 1 << 20;
    const K: u64 = 1 << 10;
    if n >= M {
        format!("{} Mpx", n / M)
    } else if n >= K {
        format!("{} Kpx", n / K)
    } else {
        format!("{n} px")
    }
}

fn class_display(class: Option<Class>) -> String {
    match class {
        Some(Class::A) => "A (lossless)".to_string(),
        Some(Class::B) => "B (lossy, standard)".to_string(),
        Some(Class::C) => "C (rebuilt)".to_string(),
        Some(Class::D) => "D (generative)".to_string(),
        None => "—".to_string(),
    }
}

/// Every message names what is wrong **and what to do about it** (`03` §7.5).
///
/// Deliberately the same wording as the CLI's `describe`: two shells telling a
/// user two different things about one refusal is how they learn to trust
/// neither.
fn describe(w: &Warning) -> String {
    use openconvert_core::isolation::Refusal;
    use openconvert_core::plan::NoRoute;
    use openconvert_core::route::Requirement;

    match w {
        Warning::TypeMismatch { declared, detected } => {
            format!("the name says {declared} and the content says {detected}; routing by content")
        }
        Warning::Polyglot => {
            "more than one format signature matched; this file is being quarantined".into()
        }
        Warning::Lossy { class } => {
            format!("this conversion is class {class:?} — some fidelity is lost")
        }
        Warning::AboveAutoClass { needed, allowed } => format!(
            "this route is class {needed:?} and the policy arms up to {allowed:?}. \
             Ask for the operation by name to run it."
        ),
        Warning::NoRoute(NoRoute::CrossKind) => {
            "those two formats are different kinds of thing; there is no route between them".into()
        }
        Warning::NoRoute(NoRoute::NeedsMoreMemory { needed, allowed }) => format!(
            "this model needs a memory limit of at least {} GB per conversion, and it is \
             set to {} GB. Raise “Memory limit per conversion” in Settings, or \
             choose a smaller model for this tool.",
            needed >> 30,
            allowed >> 30
        ),
        Warning::NoRoute(NoRoute::UnsupportedPair) => "no route exists for that pair yet".into(),
        Warning::NoRoute(NoRoute::OperationInputMismatch { operation }) => format!(
            "{operation} is an operation on a different kind of file than this one, so \
             there is nothing here for it to do"
        ),
        Warning::NoRoute(NoRoute::UnknownInput) => {
            "the content did not match any format we know, so there is nothing to route".into()
        }
        Warning::NoRoute(NoRoute::RequirementUnmet { requirement }) => match requirement {
            Requirement::Engine(name) => format!(
                "the best route needs the {name} engine, which is not installed. \
                 Reinstall to repair the engine set."
            ),
            Requirement::StreamsCompatible => {
                "this file's streams cannot be copied into that container unchanged, so a \
                 lossless remux is not possible. Converting it instead would re-encode."
                    .into()
            }
            Requirement::AudioCompatible => {
                "the audio codec in this file cannot move unchanged into that container, so \
                 lossless extraction is not possible."
                    .into()
            }
            Requirement::Always => "the best route was refused".into(),
        },
        Warning::BelowFloor(Refusal::NoNetworkDenial) => {
            "this machine offers no way to deny an engine network access, and that is not \
             something policy can waive. No conversion will run."
                .into()
        }
        Warning::BelowFloor(Refusal::NoFilesystemConfinement) => {
            "this machine offers no filesystem confinement, and policy requires it".into()
        }
        Warning::BelowFloor(Refusal::BelowStrength {
            required,
            available,
        }) => format!("policy requires {required} isolation and this machine offers {available}"),
    }
}

fn refusal_message(plan: &Plan) -> String {
    let blocking: Vec<String> = plan
        .warnings()
        .iter()
        .filter(|w| w.is_blocking())
        .map(describe)
        .collect();
    if blocking.is_empty() {
        "this plan is not executable; nothing was written".to_string()
    } else {
        blocking.join(" ")
    }
}

/// Why the top suggestion is the top suggestion.
///
/// **In the order the score is actually built**, so the sentence names the
/// signal that decided it rather than the first one that happens to be true.
/// This used to lead with a global count and mention a sibling second, which
/// meant a folder full of matching pairs was explained as "this is what jpeg
/// usually becomes".
///
/// No number is quoted any more. History is a decayed weight now, not a tally
/// -- a conversion done last year counts for less than one done yesterday --
/// and "you have converted this 6.4 times" is not a sentence. The weight is a
/// good ranking signal and a bad thing to read out, so the line says what kind
/// of evidence exists and leaves the arithmetic where it belongs.
fn explain(
    input: FormatId,
    top: Option<&SuggestionJson>,
    history: &History,
    first: Option<&Path>,
    batch_size: u32,
) -> String {
    let Some(top) = top else {
        return "nothing routes from this format on this machine yet".to_string();
    };
    let to = TABLE
        .iter()
        .find(|f| f.name == top.target)
        .map_or(FormatId::Unknown, |f| f.id);

    // 1. This folder's own habit, which is the innermost blend and therefore
    //    the thing most likely to have decided the score.
    if let Some(folder) = first.and_then(Path::parent).and_then(Path::file_name) {
        let key = folder.to_string_lossy().to_ascii_lowercase();
        if history.folder_pair(&key, input, to) > 0.0 {
            return format!(
                "you usually convert {input} to {} in {}",
                top.target,
                folder.to_string_lossy()
            );
        }
    }

    // 2. The folder already contains this exact conversion, done to other
    //    files -- evidence that needs no history at all.
    let pattern = first.map_or(0, |p| folder_pattern(p, input, to));
    if pattern > 0 {
        return if pattern == 1 {
            format!("another file here has a {} beside it", top.target)
        } else {
            format!(
                "{pattern} other files here have a {} beside them",
                top.target
            )
        };
    }

    // 3. The habit anywhere.
    if history.pair(input, to) > 0.0 {
        return format!("you have converted {input} to {} before", top.target);
    }

    // 4. This exact file, already converted once. Named for what it is, since
    //    it no longer arms on its own.
    if first.is_some_and(|p| sibling_exists(p, to)) {
        return format!("a {} already sits beside this file", top.target);
    }

    match folder_category(first) {
        FolderCategory::Screenshots => "screenshots usually want compressing".to_string(),
        FolderCategory::Scans => "scans usually become documents".to_string(),
        FolderCategory::Downloads | FolderCategory::Unknown => {
            if batch_size > 1 {
                format!("{batch_size} files, and this is what {input} usually becomes")
            } else {
                format!("this is what {input} usually becomes")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

/// Conversion history, read from the batch journals.
///
/// **Untrusted on read**, like everything else under the state directory: a
/// malformed line is skipped and a hand-edited one can only change a ranking,
/// never what a conversion is permitted to do.
///
/// # Two keys, and why the second one exists
///
/// This held one map from `(input ext, output ext)` to a count. That map
/// cannot represent a person who sends `~/Scans` to PDF and `~/Web` to WebP:
/// both habits land in the same bucket and average into a third that is
/// neither of them. Format choice is workflow-local, so history is now keyed
/// by folder as well, and `predict` blends the folder's evidence over the
/// global evidence rather than replacing it.
///
/// # Weights, not counts
///
/// A tally makes an old habit as strong as a current one, so it takes as many
/// conversions to leave a format as it took to adopt it. Each entry is decayed
/// by the age of the journal file it came from, with a half-life of
/// [`HALF_LIFE_DAYS`].
///
/// The age is the journal FILE's mtime, which is batch-level rather than
/// per-entry. That is the same approximation `store::read_history` already
/// makes, and for the same stated reason: the journal format carries no
/// per-line timestamp, and inventing one would be worse than an honest
/// batch-level number.
#[derive(Default)]
struct History {
    /// `(input extension, output extension)` → decayed weight, both lowercase.
    pairs: std::collections::HashMap<(String, String), f32>,
    /// `(folder, input extension, output extension)` → decayed weight.
    ///
    /// The folder is the lowercased final component of the parent directory,
    /// not a full path: it is a habit key, and it should survive a folder being
    /// moved or a drive letter changing.
    by_folder: std::collections::HashMap<(String, String, String), f32>,
}

/// How long it takes for one past conversion to count for half as much.
///
/// Sixty days is roughly "last two months matter most, last year still counts
/// for something". It is a guess, like every other number in the predictor, and
/// it is the one the `chosen_rank` journal field exists to replace with a
/// measurement.
const HALF_LIFE_DAYS: f32 = 60.0;

/// The weight one conversion carries, given how long ago its batch ran.
///
/// Clamped at 1.0 so a journal with a future mtime -- a clock change, a copied
/// file, a restored backup -- cannot manufacture evidence stronger than
/// something that happened today.
fn decay(age_secs: f32) -> f32 {
    if !age_secs.is_finite() || age_secs <= 0.0 {
        return 1.0;
    }
    let days = age_secs / 86_400.0;
    (0.5_f32).powf(days / HALF_LIFE_DAYS).clamp(0.0, 1.0)
}

impl History {
    fn load() -> Self {
        let mut me = Self::default();
        let dir = openconvert_run::state::paths::journal_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return me;
        };
        let now = std::time::SystemTime::now();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            // One mtime per file, read once: the weight is the same for every
            // line in a batch, and stat-ing per line would be a syscall per
            // conversion ever made.
            let age = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .map_or(0.0, |d| d.as_secs_f32());
            let weight = decay(age);

            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in contents.lines() {
                let Ok(e) =
                    serde_json::from_str::<openconvert_run::state::journal::JournalEntry>(line)
                else {
                    continue; // a torn or hand-edited line is skipped, never fatal
                };
                if e.outcome != JournalOutcome::Completed {
                    continue;
                }
                let from = ext_of(&e.source_name);
                let to = ext_of(&e.output_name);
                if from.is_empty() || to.is_empty() {
                    continue;
                }
                *me.pairs.entry((from.clone(), to.clone())).or_default() += weight;
                if let Some(folder) = e.source_folder.as_deref().map(folder_key) {
                    if !folder.is_empty() {
                        *me.by_folder.entry((folder, from, to)).or_default() += weight;
                    }
                }
            }
        }
        me
    }

    /// Decayed weight of everything this user converted `input` to.
    fn from(&self, input: FormatId) -> f32 {
        let from = extension_of(input);
        self.pairs
            .iter()
            .filter(|((f, _), _)| *f == from)
            .map(|(_, n)| *n)
            .sum()
    }

    /// How much of that went to `to`.
    fn pair(&self, input: FormatId, to: FormatId) -> f32 {
        self.pairs
            .get(&(extension_of(input), extension_of(to)))
            .copied()
            .unwrap_or(0.0)
    }

    /// The same two numbers, restricted to one folder.
    fn folder_from(&self, folder: &str, input: FormatId) -> f32 {
        let from = extension_of(input);
        self.by_folder
            .iter()
            .filter(|((d, f, _), _)| d == folder && *f == from)
            .map(|(_, n)| *n)
            .sum()
    }

    fn folder_pair(&self, folder: &str, input: FormatId, to: FormatId) -> f32 {
        self.by_folder
            .get(&(folder.to_string(), extension_of(input), extension_of(to)))
            .copied()
            .unwrap_or(0.0)
    }
}

/// The habit key for a directory: its own name, lowercased.
fn folder_key(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// How many OTHER files in this folder already have an output of `to` beside
/// them.
///
/// The generalising half of what `sibling_exists` was reaching for. A folder
/// holding `a.heic a.jpg b.heic b.jpg` says what to do with `c.heic`; the
/// same-stem check only ever said that `c.heic` had already been done.
///
/// Bounded work: the directory listing stops at [`PATTERN_SCAN_LIMIT`] entries.
/// A drop from a folder of forty thousand files must not turn a suggestion
/// into a directory walk, and three examples is already the top of the scale
/// `predict::pattern_floor` uses.
fn folder_pattern(input: &Path, from: FormatId, to: FormatId) -> u32 {
    const PATTERN_SCAN_LIMIT: usize = 2_000;

    let (Some(from_row), Some(to_row)) = (from.row(), to.row()) else {
        return 0;
    };
    let Some(dir) = input.parent() else {
        return 0;
    };
    let this_stem = input.file_stem().map(|s| s.to_ascii_lowercase());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    // One pass, collecting the stems that have each extension, so the pairing
    // is a set intersection rather than a second listing per candidate.
    let mut sources: std::collections::HashSet<std::ffi::OsString> =
        std::collections::HashSet::new();
    let mut outputs: std::collections::HashSet<std::ffi::OsString> =
        std::collections::HashSet::new();
    for entry in entries.flatten().take(PATTERN_SCAN_LIMIT) {
        let path = entry.path();
        let Some(ext) = path.extension().map(|e| e.to_ascii_lowercase()) else {
            continue;
        };
        let Some(stem) = path.file_stem().map(|s| s.to_ascii_lowercase()) else {
            continue;
        };
        // The file being predicted for is not evidence about itself.
        if this_stem.as_deref() == Some(stem.as_os_str()) {
            continue;
        }
        if ext == std::ffi::OsStr::new(from_row.extension) {
            sources.insert(stem);
        } else if ext == std::ffi::OsStr::new(to_row.extension) {
            outputs.insert(stem);
        }
    }
    u32::try_from(sources.intersection(&outputs).count()).unwrap_or(u32::MAX)
}

fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

fn extension_of(id: FormatId) -> String {
    id.row()
        .map_or_else(String::new, |r| r.extension.to_string())
}

fn format_name(id: FormatId) -> String {
    id.row()
        .map_or_else(|| id.to_string(), |r| r.name.to_string())
}

fn kind_name(id: FormatId) -> String {
    match id.kind() {
        Some(MediaKind::Image) => "image",
        Some(MediaKind::Audio) => "audio",
        Some(MediaKind::Video) => "video",
        Some(MediaKind::Document) => "document",
        Some(MediaKind::Archive) => "archive",
        Some(MediaKind::Tabular) => "tabular",
        Some(MediaKind::Spreadsheet) => "spreadsheet",
        Some(MediaKind::Font) => "font",
        None => "unknown",
    }
    .to_string()
}

/// `photo.jpg` sitting next to `photo.heic` — the strongest single signal.
fn sibling_exists(input: &Path, to: FormatId) -> bool {
    let Some(row) = to.row() else {
        return false;
    };
    let Some(stem) = input.file_stem() else {
        return false;
    };
    let dir = input.parent().unwrap_or_else(|| Path::new("."));
    dir.join(format!("{}.{}", stem.to_string_lossy(), row.extension))
        .exists()
}

fn folder_category(path: Option<&Path>) -> FolderCategory {
    let Some(name) = path
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
    else {
        return FolderCategory::Unknown;
    };
    if name.contains("screenshot") {
        FolderCategory::Screenshots
    } else if name.contains("scan") {
        FolderCategory::Scans
    } else if name.contains("download") {
        FolderCategory::Downloads
    } else {
        FolderCategory::Unknown
    }
}

/// The targets that would actually run on this machine, each with its class.
///
/// **Every candidate is routed, not just looked up.** The route table says a
/// pair exists; `route()` says whether it survives this machine's engines, this
/// file's properties and the isolation floor. Offering a target the plan
/// preview then refuses is the one failure this function exists to prevent, so
/// a candidate whose plan is not executable is dropped rather than shown greyed
/// out — the preview is where a refusal gets explained.
///
/// Same-format outputs are deliberately absent. An operation such as metadata
/// stripping is not a format conversion and cannot be serialized as `jpeg`
/// without losing its identity; doing that made the first JPEG upload offer an
/// impossible JPEG-to-JPEG conversion.
fn routable_targets(
    input: FormatId,
    props: Properties,
    policy: &Policy,
    env: &Environment,
) -> Vec<(Target, String)> {
    let mut candidates: Vec<Target> = Vec::new();
    for r in env.routes().all() {
        if r.from == input && !candidates.contains(&Target::Format(r.to)) {
            candidates.push(Target::Format(r.to));
        }
    }
    candidates
        .into_iter()
        .filter(|target| target.output_format(input) != input)
        .filter_map(|target| {
            // ROUTED WITH THE TARGET'S OWN ARMING, exactly as the CLI and the
            // tool path do (`Target::arms`).
            //
            // Without this the dropdown could only ever offer Class A and B.
            // That was invisible while every route in the table was A or B;
            // `pdf -> docx` is the first Class C row, and it would have been
            // reachable from the command line and absent from the app --
            // with no error, because a target that does not route is a target
            // that is simply not listed.
            //
            // Nothing lossier appears silently as a result. A target is listed
            // only when its plan is EXECUTABLE, so a Class D row still needs
            // its models actually installed before it can be offered, and the
            // class travels with the entry to the plan table and the receipt.
            // Choosing an item from a list of destinations is the naming
            // gesture the ceiling asks for; it is not an automatic decision.
            let armed = policy.armed_for(target);
            let plan = route(
                openconvert_core::plan::PlanRequest {
                    input,
                    target,
                    polyglot: false,
                },
                props,
                &armed,
                env,
            );
            plan.is_executable()
                .then(|| (target, class_display(plan.class())))
        })
        .collect()
}

/// Phase 2 for one file, or `Properties::None` if it cannot be read.
///
/// Ranking must not fail because a file went away between the drop and the
/// probe; the plan preview will say so properly a moment later.
fn probe_properties(path: &Path) -> Properties {
    let policy = policy_from(&UserConfig::load());
    let mut table = openconvert_run::handles::HandleTable::new();
    let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());

    let Ok(facts) = openconvert_run::detect::detect(path, &mut table) else {
        return Properties::None;
    };
    let Ok(bytes) = openconvert_run::detect::bytes_of(&facts, &mut table) else {
        return Properties::None;
    };
    openconvert_run::probe::probe(&facts, &bytes, &policy.base_limits(), &mut pool)
}

// ---------------------------------------------------------------------------
// Plumbing
// ---------------------------------------------------------------------------

fn build_environment() -> Environment {
    let caps = openconvert_os::available::probe();
    Environment::new(
        // A PLANNING input, and it can only understate: `probe()` creates an
        // AppContainer and a Job Object and closes them again rather than
        // reading a version number. The receipt does not come from here —
        // `exec` overwrites each sandboxed step's isolation with what the
        // worker read back about itself (I11, `03` §9.5).
        openconvert_os::available::planning_profile(caps),
        openconvert_run::engines::registry(),
        RouteTable::v1(),
    )
}

fn parse_target(format: &str) -> Result<Target, String> {
    let want = format.trim_start_matches('.').to_ascii_lowercase();
    if want == "strip-metadata" {
        return Ok(Target::Operation(
            openconvert_core::target::Operation::StripMetadata,
        ));
    }
    TABLE
        .iter()
        .find(|f| f.name == want || f.extension == want)
        .map(|f| Target::Format(f.id))
        .ok_or_else(|| {
            format!(
                "{} is not a format this build knows",
                DisplayName::new(format)
            )
        })
}

fn file_name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The one safe rendering of a path.
fn show(path: &Path) -> String {
    DisplayName::new(&path.to_string_lossy())
        .as_str()
        .to_string()
}

/// An I/O failure, named against a file the user can identify.
fn readable(path: &Path, e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => format!("{} is not there any more", show(path)),
        std::io::ErrorKind::PermissionDenied => {
            format!("{} cannot be read: permission denied", show(path))
        }
        _ => format!("{} could not be read: {e}", show(path)),
    }
}

fn failure(file_name: &str, message: &str, duration_ms: u64, input_bytes: u64) -> ConversionResult {
    ConversionResult {
        file_name: file_name.to_string(),
        output_path: String::new(),
        receipt_path: String::new(),
        success: false,
        error_message: Some(message.to_string()),
        duration_ms,
        input_bytes,
        output_bytes: 0,
        class_applied: "—".to_string(),
        removed_metadata: Vec::new(),
        content_id: String::new(),
        receipt_detail: None,
    }
}

/// The receipt struct, as the wire sees it. Field-for-field from
/// `openconvert_run::receipt::Receipt` — nothing re-derived, nothing dropped.
fn receipt_detail(record: &openconvert_run::receipt::Receipt) -> ReceiptDetailJson {
    ReceiptDetailJson {
        version: record.version,
        tool: record.tool.clone(),
        content_id: record.content_id.clone(),
        detected: record.detected.clone(),
        declared_mismatch: record.declared_mismatch.clone(),
        class: record.class.clone(),
        output_name: record.output.clone(),
        output_bytes: record.output_bytes,
        steps: record
            .steps
            .iter()
            .map(|st| StepReceiptJson {
                kind: st.kind.clone(),
                class: st.class.clone(),
                engine: st.engine.clone(),
                isolation: st.isolation.clone(),
                limits_summary: format!(
                    "{} MiB memory, {} decode, {}s wall",
                    st.limits.memory_bytes / (1 << 20),
                    pixels(st.limits.decode_pixels),
                    st.limits.wall_time_secs
                ),
            })
            .collect(),
    }
}

/// Elapsed milliseconds, saturating.
///
/// A conversion that ran for 585 million years is not a case worth a wrapping
/// cast, but it is worth not lying about.
fn millis(start: std::time::Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Progress is advisory. A window that has gone away is not a batch failure.
fn emit(app: &tauri::AppHandle, event: &ProgressEvent) {
    let _ = app.emit("conversion-progress", event);
}

/// A screen rectangle in physical pixels. Monitors and windows are both this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

impl Rect {
    /// Area of the overlap between two rectangles, in physical pixels.
    fn overlap(self, other: Self) -> u64 {
        // Saturating throughout: a saved position can be any i32 the config
        // file happens to contain, and a coordinate arithmetic overflow must
        // not be the thing that decides where a window opens.
        let right = |r: Self| i64::from(r.x) + i64::from(r.w);
        let bottom = |r: Self| i64::from(r.y) + i64::from(r.h);
        let w = right(self).min(right(other)) - i64::from(self.x.max(other.x));
        let h = bottom(self).min(bottom(other)) - i64::from(self.y.max(other.y));
        if w <= 0 || h <= 0 {
            return 0;
        }
        (w as u64) * (h as u64)
    }
}

/// The smallest visible fraction of a restored window that counts as "usable".
///
/// A window a third of which is on a real screen can be grabbed and dragged;
/// one with a corner poking onto the desktop cannot. The fraction is of the
/// WINDOW, not of the monitor, so a small window on a large display is not
/// judged by how little of the screen it covers.
const MIN_VISIBLE_FRACTION: f64 = 0.30;

/// Whether a saved window rectangle is still reachable on the displays now
/// attached.
///
/// **This is the check that was missing, and its absence is the bug.** The
/// comment that used to stand here said an off-screen position "is simply
/// refused by the OS call" — it is not. Windows places a window wherever it is
/// told, including at the coordinates of a monitor that was unplugged three
/// weeks ago, and the app then opens onto a desktop nobody can see: no window,
/// no error, and the only recovery is deleting the config by hand.
///
/// Overlap is measured against every attached monitor and summed, because a
/// window straddling two screens is visible on both and belongs to neither.
fn is_visible_on(window: Rect, monitors: &[Rect]) -> bool {
    if window.w == 0 || window.h == 0 {
        return false;
    }
    let area = u64::from(window.w) * u64::from(window.h);
    let visible: u64 = monitors.iter().map(|m| window.overlap(*m)).sum();
    // Integer comparison rather than a float division: `area` is never zero
    // here, but a ratio is one refactor away from a divide-by-zero and this
    // form has no such edge.
    #[allow(clippy::cast_precision_loss)]
    let needed = (area as f64 * MIN_VISIBLE_FRACTION) as u64;
    visible >= needed.max(1)
}

/// Where to put a window whose saved position is no longer on any screen:
/// centred on `primary`.
fn centred_on(primary: Rect, w: u32, h: u32) -> (i32, i32) {
    let dx = (i64::from(primary.w) - i64::from(w)) / 2;
    let dy = (i64::from(primary.h) - i64::from(h)) / 2;
    // A window wider than the display centres to a negative offset, which
    // would put its left edge off-screen; the clamp keeps the origin on the
    // monitor and lets the overflow fall off the right instead, where the
    // controls this app puts top-left stay reachable.
    let x = i64::from(primary.x) + dx.max(0);
    let y = i64::from(primary.y) + dy.max(0);
    (
        i32::try_from(x).unwrap_or(primary.x),
        i32::try_from(y).unwrap_or(primary.y),
    )
}

/// Apply the saved window geometry, validated against the displays attached
/// **now** rather than the ones attached when it was saved.
fn apply_saved_geometry(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let config = UserConfig::load();

    // Every monitor currently attached, as physical rectangles. An empty list
    // (a headless session, or a backend that will not answer) means there is
    // nothing to validate against, and the saved values are then left alone —
    // refusing to restore because we cannot see the screens would be its own
    // bug on a machine where the position was fine.
    let monitors: Vec<Rect> = window
        .available_monitors()
        .map(|list| {
            list.iter()
                .map(|m| Rect {
                    x: m.position().x,
                    y: m.position().y,
                    w: m.size().width,
                    h: m.size().height,
                })
                .collect()
        })
        .unwrap_or_default();

    // Maximized is a STATE, restored as one. See `persist_geometry`: replaying
    // a maximized window's rectangle produces a window that fills the screen
    // and cannot be resized or restored down.
    if config.window_maximized == Some(true) {
        let _ = window.maximize();
        return;
    }

    let (mut w, mut h) = match (config.window_w, config.window_h) {
        (Some(w), Some(h)) => (w, h),
        _ => (0, 0),
    };
    // A RECTANGLE SAVED BY THE OLD CODE, RECOGNISED AND CORRECTED.
    //
    // `window_maximized` did not exist until this was fixed, so a config
    // written by an earlier build carries a maximized window's rectangle with
    // no state beside it -- and restoring that is the bug. It is recognisable:
    // a window whose frame is as large as the monitor and whose origin is at or
    // outside the monitor's own origin is not a window anyone sized by hand,
    // it is a maximized one written down wrong.
    //
    // Treated as maximized rather than clamped, because clamping it would give
    // a full-screen window that merely has grabbable edges -- technically
    // resizable, still not what the user left.
    if config.window_maximized.is_none() && w > 0 && h > 0 {
        if let (Some(x), Some(y)) = (config.window_x, config.window_y) {
            let looks_maximized = monitors
                .iter()
                .any(|m| x <= m.x && y <= m.y && w >= m.w && h >= m.h);
            if looks_maximized {
                let _ = window.maximize();
                return;
            }
        }
    }

    if w > 0 && h > 0 {
        // **Physical in, physical out.**
        //
        // `persist_geometry` records `inner_size()`, which is PHYSICAL pixels.
        // Restoring the same numbers as a `LogicalSize` multiplied the window by
        // the scale factor on every launch: on a 200 % display 900x680 became
        // 1800x1360, then 3600x2720, until it was larger than the screen. The
        // units have to match the ones that were written.
        let clamped = clamp_to_monitor(&window, w, h);
        w = clamped.0;
        h = clamped.1;
        let _ = window.set_size(tauri::PhysicalSize::new(w, h));
    }

    let (Some(x), Some(y)) = (config.window_x, config.window_y) else {
        return;
    };
    // Size may not have been saved; ask the window what it is rather than
    // assuming, so the rectangle tested is the one that will actually appear.
    if w == 0 || h == 0 {
        let Ok(size) = window.inner_size() else {
            return;
        };
        w = size.width;
        h = size.height;
    }

    let saved = Rect { x, y, w, h };
    if monitors.is_empty() || is_visible_on(saved, &monitors) {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        return;
    }

    // The saved position points at a screen that is not there. Open on the
    // primary display instead — and do NOT write the corrected position back:
    // plugging the monitor in again should restore the window to where the
    // user left it, which is only possible if the coordinates survive.
    let primary = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| Rect {
            x: m.position().x,
            y: m.position().y,
            w: m.size().width,
            h: m.size().height,
        })
        .unwrap_or(monitors[0]);
    let (cx, cy) = centred_on(primary, w, h);
    let _ = window.set_position(tauri::PhysicalPosition::new(cx, cy));
}

/// Keep a restored size inside the monitor it will open on.
///
/// A size saved on a larger display — or written by the unit bug above — must
/// not produce a window whose controls sit past the edge of the screen. The
/// floor is the configured minimum so clamping can never make it unusable.
fn clamp_to_monitor(window: &tauri::WebviewWindow, w: u32, h: u32) -> (u32, u32) {
    const MIN_W: u32 = 640;
    const MIN_H: u32 = 480;
    let Ok(Some(monitor)) = window.current_monitor() else {
        return (w.max(MIN_W), h.max(MIN_H));
    };
    let bounds = monitor.size();
    (w.clamp(MIN_W, bounds.width), h.clamp(MIN_H, bounds.height))
}

/// Record the window geometry into the user config on close.
fn persist_geometry(window: &tauri::Window) {
    let mut config = UserConfig::load();

    // THE MAXIMIZED STATE IS RECORDED, AND THE MAXIMIZED GEOMETRY IS NOT.
    //
    // A maximized window on Windows reports a NEGATIVE position and a size
    // larger than the work area -- the frame's borders are deliberately off
    // the edge of the screen. Saving those numbers and restoring them as an
    // ordinary window gives a window that fills the display, is not maximized
    // as far as the OS is concerned, and whose resize edges are outside the
    // visible area: it cannot be resized, and the maximize button will not
    // restore it down, because there is nothing to restore down to.
    //
    // That is exactly the reported "opens full screen and is not resizable",
    // and the saved config that produced it read `-13, -13, 2880x1704`.
    //
    // So: when maximized, keep the state and LEAVE THE GEOMETRY ALONE. What is
    // already stored is the last size the window had as an ordinary window,
    // which is exactly what restore-down should give back.
    let maximized = window.is_maximized().unwrap_or(false);
    config.window_maximized = Some(maximized);

    if !maximized {
        let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
            let _ = config.save();
            return;
        };
        // Both are physical pixels, and `apply_saved_geometry` reads them back
        // as physical. The pair is only correct together.
        config.window_x = Some(position.x);
        config.window_y = Some(position.y);
        config.window_w = Some(size.width);
        config.window_h = Some(size.height);
    }
    // Best-effort: a read-only state directory must not stop the app closing.
    let _ = config.save();
}

/// Re-fetch model artifacts this build pins differently from the copy on disk.
///
/// **This is what the `Model auto update` switch gates**, and the reason that
/// switch is allowed to exist: the three network settings that stood here
/// before it were each removed for configuring a request the program never
/// made.
///
/// Detecting the work costs no network -- `models::outdated` compares the pin
/// in models.toml against a marker in the local store -- so the switch is
/// consulted BEFORE anything is fetched, and a user who turned it off gets a
/// program that reaches the network only when they press Download.
///
/// Off the UI thread, and quietly: a startup that blocks on a download is a
/// startup that hangs on a bad connection. Nothing is reported on success;
/// failures land in `model-download-progress` like any other download, because
/// a silent failure to update is the one outcome worth telling someone about.
fn start_model_auto_update(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        // BEFORE THE SWITCH IS CONSULTED, AND WITHOUT THE NETWORK.
        //
        // Writing the missing markers is bookkeeping about files already on
        // disk. It reaches nothing, so it is not something the network switch
        // has any business gating -- and doing it here rather than inside the
        // `if` is what makes a user who turned auto-update OFF still get
        // correct answers from the update check when they turn it back on.
        let _ = openconvert_run::models::backfill_markers();

        if !UserConfig::load().model_auto_update.unwrap_or(true) {
            return;
        }
        if openconvert_run::models::outdated().is_empty() {
            return;
        }
        let (done, failed) =
            openconvert_run::models::update_outdated_with_progress(&mut |id, received, total| {
                emit_progress(&app, id, received, total, "downloading")
            });
        for id in done {
            emit_progress(&app, &id, 0, 0, "done");
        }
        for message in failed {
            eprintln!("openconvert: model auto-update failed -- {message}");
        }
    });
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::new())
        .setup(|app| {
            // FIRST, before anything can route or convert.
            //
            // The engine set does not always sit beside the executable. A .deb
            // puts the binary in /usr/bin and its resources in
            // /usr/lib/<Product>/; a macOS bundle splits Contents/MacOS from
            // Contents/Resources. Only on Windows are the two the same
            // directory -- which is why an installed copy converted nothing on
            // the other two platforms while `cargo run` worked perfectly.
            //
            // This is the one place that knows how this program was packaged,
            // so it is the one place that can answer. `openconvert-run` still
            // never reads PATH; it now searches this directory in addition to
            // its own, and a test asserts the distinction survives.
            match app.path().resource_dir() {
                Ok(dir) => {
                    openconvert_run::engine_dir::set_resource_dir(dir);
                }
                Err(e) => {
                    // Not fatal: beside-the-executable is still searched, which
                    // is every development build and every Windows install.
                    eprintln!("openconvert: no resource directory ({e}); engines must sit beside the executable");
                }
            }
            apply_saved_geometry(app.handle());
            start_model_auto_update(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                persist_geometry(window);
            }
        })
        .invoke_handler(tauri::generate_handler![
            probe_file,
            preview_file,
            get_plan,
            suggest_targets,
            expand_paths,
            export_batch_log,
            save_text_file,
            convert_batch,
            cancel_all,
            undo_last,
            get_config,
            set_config,
            list_history,
            reveal_receipt,
            reveal_output,
            audio_peaks,
            read_text_output,
            preview_audio,
            save_text_copy,
            save_image_corrections,
            list_signatures,
            add_signature,
            delete_signature,
            wipe_history,
            list_receipts,
            receipts_size,
            gpu_device,
            receipts_count,
            delete_receipt,
            delete_all_receipts,
            list_models,
            list_ai_features,
            download_ai_feature,
            download_model,
            set_model_enabled,
            set_model_tier,
            delete_model,
            list_tools,
            run_tool,
            panic_stop,
            pick_files,
        ])
        .run(tauri::generate_context!())
        .expect("the OpenConvert window could not be created");
}

#[cfg(test)]
mod tests {
    //! The command bodies, without a window.
    //!
    //! Every `#[tauri::command]` above is a one-line wrapper over one of these
    //! functions, so testing them tests everything this crate decides. What the
    //! wrappers add — `spawn_blocking`, the managed state, the event emit — is
    //! Tauri's, and spike W9 already proved that half end to end from inside a
    //! real Tauri process on this platform.

    use super::*;
    #[test]
    fn modified_preview_applies_crop_before_rotation() {
        let img = image::DynamicImage::new_rgb8(100, 200);
        let mut png = std::io::Cursor::new(Vec::new());
        img.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let (_, w, h) = downscale_png(
            &png.into_inner(),
            500,
            &["pdf-crop:25,0,25,0".into(), "pdf-turn:90".into()],
        )
        .unwrap();
        assert_eq!((w, h), (200, 50));
    }

    #[test]
    fn tool_choices_survive_desktop_ipc() {
        for tool in openconvert_run::tools::all_tools() {
            let wire = serde_json::to_value(super::tool_json(&tool)).unwrap();
            assert_eq!(
                wire["params"],
                serde_json::to_value(&tool.params).unwrap(),
                "{}",
                tool.id
            );
        }
    }

    /// A 1×1 PNG. Small enough to inline, real enough to route.
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    /// A directory of our own, so a conversion's outputs land somewhere the
    /// test owns and nothing collides between runs.
    fn scratch(name: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "openconvert-desktop-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    /// Default output behaviour, for tests that do not exercise settings.
    fn default_settings() -> OutputSettings {
        OutputSettings {
            destination: "same_folder".into(),
            naming_template: DEFAULT_NAMING_TEMPLATE.into(),
            write_receipts: false,
        }
    }

    fn png_at(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, PNG).expect("write the fixture");
        path
    }

    #[test]
    fn probe_reports_what_the_bytes_say() {
        let dir = scratch("probe");
        let path = png_at(&dir, "photo.png");

        let result = probe_one(&path).expect("a PNG probes");

        assert_eq!(result.file_name, "photo.png");
        assert_eq!(result.detected, "png");
        assert!(!result.mismatched, "the extension agrees with the content");
        assert!(!result.polyglot);
        assert_eq!(result.input_bytes, PNG.len() as u64);

        let props = result.properties.expect("phase 2 read the header");
        assert_eq!(props.kind, "image");
        assert_eq!(props.width, Some(1));
        assert_eq!(props.height, Some(1));
    }

    /// SR-4: the mismatch is surfaced, and routing follows the content.
    #[test]
    fn a_lying_extension_is_reported_not_believed() {
        let dir = scratch("mismatch");
        let path = png_at(&dir, "actually-a-png.jpg");

        let result = probe_one(&path).expect("it still probes");

        assert_eq!(result.detected, "png", "routed by content");
        assert_eq!(result.declared.as_deref(), Some("jpeg"));
        assert!(result.mismatched);
    }

    #[test]
    fn a_missing_file_names_itself() {
        let dir = scratch("missing");
        let message = probe_one(&dir.join("not-here.png")).expect_err("no file, no facts");
        assert!(message.contains("not-here.png"), "got: {message}");
    }

    /// The plan preview is `route()` without `execute()` — so it must produce
    /// steps, and it must leave the directory exactly as it found it.
    #[test]
    fn planning_writes_nothing() {
        let dir = scratch("plan");
        let path = png_at(&dir, "photo.png");
        let before = std::fs::read_dir(&dir).unwrap().count();

        let preview = plan_for(&[path.to_string_lossy().into_owned()], "jpeg");

        assert!(
            preview.executable,
            "PNG to JPEG routes: {:?}",
            preview.warnings
        );
        assert!(!preview.steps.is_empty());
        assert_eq!(preview.total_input_bytes, PNG.len() as u64);
        assert_eq!(
            preview.estimated_duration_ms, 0,
            "0 means not determined; nothing here estimates yet"
        );

        for step in &preview.steps {
            assert!(
                !step.limits_summary.is_empty(),
                "I4: every step carries limits"
            );
            assert!(
                !step.isolation.is_empty(),
                "I10: every step says where it runs"
            );
            assert!(
                matches!(step.class_name.as_str(), "A" | "B" | "C" | "D"),
                "I3: every step carries a class, got {:?}",
                step.class_name
            );
        }

        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            before,
            "the preview created a file"
        );
    }

    #[test]
    fn an_unknown_target_says_so_without_planning() {
        let dir = scratch("bad-target");
        let path = png_at(&dir, "photo.png");

        let preview = plan_for(&[path.to_string_lossy().into_owned()], "nonsense");

        assert!(!preview.executable);
        assert!(preview.warnings.iter().any(|w| w.blocking));
    }

    /// The whole desktop path: detect, probe, route and execute without a
    /// beside-output sidecar. Receipt persistence itself is covered by the
    /// desktop SQLite database tests.
    #[test]
    fn converting_writes_an_output_without_a_sidecar() {
        let dir = scratch("convert");
        let path = png_at(&dir, "photo.png");

        let policy = Policy::default();
        let env = build_environment();
        let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());
        let settings = default_settings();

        let done = convert_one(
            &path,
            parse_target("jpeg").unwrap(),
            &policy,
            &env,
            &mut pool,
            &settings,
            0,
            "2026-08-24",
        )
        .expect("PNG to JPEG converts");

        assert!(done.output.exists(), "the output was written");
        assert!(
            done.receipt.as_os_str().is_empty(),
            "desktop receipts are store-only"
        );
        assert!(!dir.join("photo.jpg.receipt.json").exists());
        assert_eq!(done.output.extension().unwrap(), "jpg");
        assert!(done.output_bytes > 0);
        assert_eq!(done.input_bytes, PNG.len() as u64);
        assert!(
            done.class_applied.starts_with("B "),
            "PNG to JPEG is lossy but standard, got {:?}",
            done.class_applied
        );
        assert!(path.exists(), "the original is never modified");
        assert!(
            done.detail.content_id.len() == 64,
            "the input identity crosses with the receipt"
        );
        assert!(
            !done.detail.steps.is_empty(),
            "every executed step is on the wire"
        );
        for step in &done.detail.steps {
            assert!(!step.engine.is_empty(), "I4: every step names its engine");
            assert!(
                !step.isolation.is_empty(),
                "I10: every step says where it ran"
            );
        }
        assert!(done.replaced.is_none(), "same_folder replaces nothing");
    }

    /// Undo removes exactly what the batch created, and nothing else.
    #[test]
    fn undo_removes_the_outputs_and_leaves_the_input() {
        let dir = scratch("undo");
        let path = png_at(&dir, "photo.png");

        let policy = Policy::default();
        let env = build_environment();
        let mut pool = openconvert_run::pool::WorkerPool::new(policy.worker_reuse());
        let settings = default_settings();
        let done = convert_one(
            &path,
            parse_target("jpeg").unwrap(),
            &policy,
            &env,
            &mut pool,
            &settings,
            0,
            "2026-08-24",
        )
        .expect("it converts");

        std::fs::remove_file(&done.output).expect("removable");

        assert!(!done.output.exists());
        assert!(done.receipt.as_os_str().is_empty());
        assert!(path.exists(), "undo never touches the input");
    }

    /// Every `Properties` variant reaches the wire with its own `kind`.
    ///
    /// A variant added to the enum without an arm here is a field the UI
    /// silently stops showing, which is the failure mode the whole
    /// field-for-field discipline exists to catch.
    #[test]
    fn every_properties_variant_is_rendered() {
        let cases = [
            (
                Properties::Image {
                    width: 4,
                    height: 2,
                    has_alpha: true,
                    frames: 1,
                },
                "image",
            ),
            (
                Properties::Audio {
                    duration_ms: 1000,
                    channels: 2,
                    sample_rate: 44100,
                },
                "audio",
            ),
            (
                Properties::Video {
                    duration_ms: 0,
                    width: 1920,
                    height: 1080,
                    video: None,
                    audio: None,
                },
                "video",
            ),
            (
                Properties::Document {
                    pages: 3,
                    encrypted: false,
                },
                "document",
            ),
            (
                Properties::Archive {
                    entries: 9,
                    depth: 1,
                    declared_total_bytes: None,
                },
                "archive",
            ),
            (
                Properties::Tabular {
                    rows: 10,
                    columns: 4,
                },
                "tabular",
            ),
        ];

        for (props, expected) in cases {
            let json = properties_json(props).expect("a variant renders");
            assert_eq!(json.kind, expected);
        }

        assert!(
            properties_json(Properties::None).is_none(),
            "nothing extracted is null on the wire, not a kind the UI must special-case"
        );
    }

    /// Class strings are what the receipt view shows. They are part of the
    /// contract, not free text.
    #[test]
    fn class_strings_name_the_cost() {
        assert_eq!(class_display(Some(Class::A)), "A (lossless)");
        assert_eq!(class_display(Some(Class::B)), "B (lossy, standard)");
        assert_eq!(class_display(Some(Class::C)), "C (rebuilt)");
        assert_eq!(class_display(Some(Class::D)), "D (generative)");
        assert_eq!(class_display(None), "—");
    }

    /// The forced-sandbox setting confines what it can, and claims no more.
    ///
    /// Two halves, and both matter: routes with a worker must move from
    /// in-process to sandboxed, and routes without one must stay exactly as
    /// they were rather than becoming plans that cannot execute.
    #[test]
    fn forcing_sandbox_confines_only_what_has_a_worker() {
        use openconvert_core::route::RouteTable;

        let relaxed = Policy::default();
        let strict = Policy::default().force_sandbox_everywhere();
        let env = build_environment();

        let isolation = |policy: &Policy, from, to| {
            route(
                openconvert_core::plan::PlanRequest {
                    input: from,
                    target: Target::Format(to),
                    polyglot: false,
                },
                Properties::None,
                policy,
                &env,
            )
            .steps()
            .first()
            .map(|s| s.isolation)
        };

        // A raster conversion: in-process normally, confined when forced.
        assert!(matches!(
            isolation(&relaxed, FormatId::Png, FormatId::Jpeg),
            Some(Isolation::InProcess)
        ));
        assert!(
            matches!(
                isolation(&strict, FormatId::Png, FormatId::Jpeg),
                Some(Isolation::Sandboxed(_))
            ),
            "forcing did not confine a route that has a worker"
        );

        // Tabular has no worker, so forcing must leave it alone rather than
        // planning a step nothing can run.
        assert!(matches!(
            isolation(&strict, FormatId::Csv, FormatId::Json),
            Some(Isolation::InProcess)
        ));

        // And nothing anywhere may become unrunnable.
        for row in RouteTable::v1().all() {
            let plan = route(
                openconvert_core::plan::PlanRequest {
                    input: row.from,
                    target: Target::Format(row.to),
                    polyglot: false,
                },
                Properties::None,
                &strict,
                &env,
            );
            for step in plan.steps() {
                if !matches!(step.isolation, Isolation::Sandboxed(_)) {
                    continue;
                }
                let to = match step.kind {
                    StepKind::Transcode { to, .. } => to,
                    _ => row.to,
                };
                // `for_conversion`, not `for_media`: this must ask the same
                // question the executor asks, or it passes a plan the executor
                // then refuses. Video-to-video is the one confined step with no
                // engine binary — it runs in the video module's own container.
                let ok = EngineBin::for_conversion(row.from, to).is_some()
                    || (row.from.kind() == Some(MediaKind::Video)
                        && to.kind() == Some(MediaKind::Video));
                assert!(ok, "forcing made {} -> {} unrunnable", row.from, row.to);
            }
        }
    }

    /// An environment where every engine the route table names is present.
    ///
    /// `build_environment()` reports what is installed BESIDE THE RUNNING
    /// BINARY, and a test binary lives in `deps/`, where the workers are not
    /// staged. A property test over the route table that uses it therefore
    /// examines only the rows needing no worker -- 70 of them, against a table
    /// of nearly two hundred -- and says nothing about the rest while looking
    /// like it covers everything.
    ///
    /// The engine list is read out of the table rather than typed here, so a
    /// row added with a new requirement is covered without anyone remembering
    /// to add it.
    #[cfg(test)]
    fn environment_with_every_engine() -> Environment {
        use openconvert_core::environment::EngineEntry;
        use openconvert_core::route::{Requirement, RouteTable};

        let mut names: Vec<&'static str> = Vec::new();
        for row in RouteTable::v1().all() {
            for req in row.requires {
                if let Requirement::Engine(name) = req {
                    if !names.contains(name) {
                        names.push(name);
                    }
                }
            }
        }
        let engines = names
            .into_iter()
            .map(|name| EngineEntry {
                name,
                memory_safe: false,
                available: true,
            })
            .collect();
        Environment::new(
            openconvert_os::available::planning_profile(openconvert_os::available::probe()),
            engines,
            RouteTable::v1(),
        )
    }

    /// **Every route in the table can be OFFERED, not just planned.**
    ///
    /// `routable_targets` is what fills the destination dropdown, and it
    /// filters on `plan.is_executable()`. Before `Policy::armed_for` it routed
    /// with the bare default policy, whose auto-class ceiling is B -- so a
    /// Class C or D row was reachable from the command line and simply ABSENT
    /// from the app, with no error anywhere, because a target that does not
    /// route is a target that is not listed.
    ///
    /// That was invisible for as long as every row in the table was A or B.
    /// `pdf -> docx` is the first Class C row and it would have been the first
    /// casualty. This asserts the general property rather than that one pair.
    #[test]
    fn the_dropdown_offers_every_class_the_table_has() {
        use openconvert_core::route::RouteTable;

        let policy = Policy::default();
        let env = environment_with_every_engine();

        let mut missing = Vec::new();
        let mut checked = 0usize;
        let mut classes: Vec<openconvert_core::plan::Class> = Vec::new();
        for row in RouteTable::v1().all() {
            let target = Target::Format(row.to);
            // A row that cannot plan even with its class armed is out of
            // scope here -- the ceiling is what this test is about.
            let armed = route(
                openconvert_core::plan::PlanRequest {
                    input: row.from,
                    target,
                    polyglot: false,
                },
                Properties::None,
                &policy.armed_for(target),
                &env,
            );
            // A row whose destination IS the input format is deliberately
            // not a dropdown entry: `mp3 -> mp3` is a re-encode, which is an
            // operation, not somewhere to convert to.
            if row.to == row.from || !armed.is_executable() {
                continue;
            }
            checked += 1;
            if !classes.contains(&row.class) {
                classes.push(row.class);
            }
            let offered = routable_targets(row.from, Properties::None, &policy, &env)
                .iter()
                .any(|(t, _)| *t == target);
            if !offered {
                missing.push(format!(
                    "{} -> {} (class {:?})",
                    row.from, row.to, row.class
                ));
            }
        }
        assert!(
            missing.is_empty(),
            "{} route(s) plan but are never offered: {missing:?}",
            missing.len()
        );
        // **The controls.** A property over too small a set is true and
        // useless, and the first version of this test examined 70 rows and no
        // Class C row at all -- it passed with the bug still in place.
        assert!(
            checked > 150,
            "only {checked} rows examined; the environment is not offering the engines"
        );
        assert!(
            classes.contains(&openconvert_core::plan::Class::C),
            "no Class C row was examined, which is the case this test exists for"
        );
    }

    /// The control for the test above: the ceiling still exists.
    ///
    /// Arming per named target must not become "arm everything always". A
    /// policy that was NOT asked about a target keeps its default ceiling, so
    /// a Class D row is not reachable by a caller that never named it.
    #[test]
    fn an_unnamed_target_does_not_raise_the_ceiling() {
        use openconvert_core::plan::Class;
        assert_eq!(Policy::default().max_auto_class(), Class::B);
        assert_eq!(
            Policy::default()
                .armed_for(Target::Operation(
                    openconvert_core::target::Operation::StripMetadata
                ))
                .max_auto_class(),
            Class::B,
            "an operation that invents nothing must not raise the ceiling"
        );
    }

    /// The flag only ever tightens.
    #[test]
    fn force_sandbox_is_a_ratchet() {
        let once = Policy::default().force_sandbox_everywhere();
        assert!(once.force_sandbox());
        // Applying it again is idempotent, and there is no setter that clears
        // it — a managed layer can pin it and a user config cannot undo it.
        assert!(once.force_sandbox_everywhere().force_sandbox());
    }

    /// **Every route in the table, and where each of its steps runs.**
    ///
    /// The invariant: a step may only be `InProcess` if the INPUT format is
    /// marked `pure_rust_parser` in the format table. Anything else must be
    /// `Sandboxed`, because a third-party C parser is confined or it does not
    /// run at all (I10, SR-2).
    ///
    /// This walks the real route table against a real environment rather than
    /// asserting on a hand-picked pair, so a row added later cannot quietly
    /// introduce an unconfined parser.
    #[test]
    fn every_route_confines_what_it_must() {
        use openconvert_core::route::RouteTable;

        let policy = Policy::default();
        let env = build_environment();
        let table = RouteTable::v1();

        let mut in_process = Vec::new();
        let mut violations = Vec::new();
        let mut sandboxed = 0usize;

        for row in table.all() {
            let plan = route(
                openconvert_core::plan::PlanRequest {
                    input: row.from,
                    target: Target::Format(row.to),
                    polyglot: false,
                },
                Properties::None,
                &policy,
                &env,
            );

            for step in plan.steps() {
                match step.isolation {
                    Isolation::Sandboxed(_) => sandboxed += 1,
                    Isolation::InProcess => {
                        in_process.push(format!("{} -> {} ({:?})", row.from, row.to, step.kind));
                        // The one permitted justification.
                        if !row.from.has_pure_rust_parser() {
                            violations.push(format!(
                                "{} -> {} runs {:?} IN PROCESS but {} has no pure-Rust parser",
                                row.from, row.to, step.kind, row.from
                            ));
                        }
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "unconfined third-party parsing:
{}",
            violations.join(
                "
"
            )
        );

        // Not an assertion about the number — a record of which rows are
        // in-process, so a change to that set shows up in a diff rather than
        // only in behaviour.
        eprintln!(
            "sandboxed steps: {sandboxed}; in-process steps: {}
{}",
            in_process.len(),
            in_process.join(
                "
"
            )
        );
    }

    /// Every sandboxed step must have something confined to run it.
    ///
    /// Two executors, not one. The generic arm hands the step to an
    /// `EngineBin` worker over the wire protocol; video-to-video goes to the
    /// **video module** instead, which confines FFmpeg in its own
    /// AppContainer because FFmpeg is not our binary and does not speak our
    /// protocol. Both are sandboxed; only one is a worker.
    ///
    /// An earlier version of this test asserted `EngineBin::for_media` covered
    /// everything and reported ten false positives — the video rows, which are
    /// confined perfectly well. Asserting the narrower claim would have had
    /// someone "fix" a path that was already right.
    #[test]
    fn every_sandboxed_route_has_a_worker() {
        let policy = Policy::default();
        let env = build_environment();
        let mut orphans = Vec::new();

        for row in openconvert_core::route::RouteTable::v1().all() {
            let plan = route(
                openconvert_core::plan::PlanRequest {
                    input: row.from,
                    target: Target::Format(row.to),
                    polyglot: false,
                },
                Properties::None,
                &policy,
                &env,
            );
            for step in plan.steps() {
                if !matches!(step.isolation, Isolation::Sandboxed(_)) {
                    continue;
                }
                let to = match step.kind {
                    StepKind::Transcode { to, .. } => to,
                    _ => row.to,
                };
                let has_worker = to.kind().and_then(EngineBin::for_media).is_some();
                let is_video_module = row.from.kind() == Some(MediaKind::Video)
                    && to.kind() == Some(MediaKind::Video);
                if !has_worker && !is_video_module {
                    orphans.push(format!(
                        "{} -> {} plans a sandboxed step with no executor for {to}",
                        row.from, row.to
                    ));
                }
            }
        }

        assert!(
            orphans.is_empty(),
            "sandboxed steps with no engine to run them:
{}",
            orphans.join(
                "
"
            )
        );
    }

    /// RFC 4648 test vectors. A hand-rolled encoder is only acceptable if it
    /// is checked against the spec's own examples rather than against itself.
    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// Every byte value round-trips, so the alphabet has no gap or typo.
    #[test]
    fn base64_covers_every_byte() {
        let all: Vec<u8> = (0..=255u8).collect();
        let encoded = base64(&all);
        assert_eq!(encoded.len(), 344, "256 bytes is 344 base64 characters");
        assert!(encoded.ends_with('='), "256 is not a multiple of 3");
        assert!(
            encoded
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='),
            "encoder emitted a character outside the alphabet"
        );
    }

    /// The preview is capped on its longest edge, and aspect ratio survives.
    #[test]
    fn downscale_caps_the_longest_edge() {
        let wide = image::RgbaImage::from_pixel(800, 200, image::Rgba([10, 20, 30, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(wide)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("fixture encodes");

        let (bytes, w, h) = downscale_png(png.get_ref(), 100, &[]).expect("it scales");
        assert_eq!(w, 100, "long edge is the cap");
        assert_eq!(h, 25, "4:1 stays 4:1");
        assert!(!bytes.is_empty());

        // Already inside the cap: left alone rather than upscaled.
        let (_, w2, h2) = downscale_png(png.get_ref(), 4000, &[]).expect("it passes through");
        assert_eq!((w2, h2), (800, 200));
    }

    /// Invert and greyscale act on pixels, and act only when asked.
    #[test]
    fn preview_ops_change_the_pixels() {
        use image::GenericImageView;

        let src = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 40, 40, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(src)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("fixture encodes");

        let read = |bytes: &[u8]| {
            image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
                .expect("decodes")
                .get_pixel(0, 0)
        };

        let (plain, _, _) = downscale_png(png.get_ref(), 64, &[]).expect("no ops");
        assert_eq!(read(&plain).0[0], 200, "an empty op list changes nothing");

        let (inverted, _, _) =
            downscale_png(png.get_ref(), 64, &["image-invert".to_string()]).expect("invert");
        let px = read(&inverted).0;
        assert_eq!(px[0], 55, "200 inverts to 55");
        assert_eq!(px[3], 255, "alpha is not inverted");

        let (grey, _, _) =
            downscale_png(png.get_ref(), 64, &["image-greyscale".to_string()]).expect("greyscale");
        let g = read(&grey).0;
        assert_eq!(g[0], g[1], "greyscale means the channels agree");
        assert_eq!(g[1], g[2], "greyscale means the channels agree");
        assert_ne!(g[0], 200, "and it is a luminance, not the red channel");

        // An unknown op is ignored rather than fatal: the rail may name an
        // operation this build's renderer does not implement yet.
        let (unknown, _, _) =
            downscale_png(png.get_ref(), 64, &["image-sepia".to_string()]).expect("tolerated");
        assert_eq!(read(&unknown).0[0], 200);
    }

    #[test]
    fn a_preview_of_a_missing_file_names_it() {
        let dir = scratch("preview-missing");
        let msg = render_preview(&dir.join("gone.png"), 1, 512, &[]).expect_err("no file");
        assert!(msg.contains("gone.png"), "got: {msg}");
    }

    /// A9/`09` §3: a crafted filename gets no control over the UI.
    #[test]
    fn a_hostile_filename_is_escaped_before_it_crosses() {
        let raw = "invoice\u{202e}fdp.exe";
        let rendered = DisplayName::new(raw);
        assert!(
            !rendered.as_str().contains('\u{202e}'),
            "the override survived"
        );
        assert!(
            rendered.altered(),
            "and the UI is told the name was altered"
        );
    }

    /// A dropped folder must contribute its convertible files and nothing
    /// else: without this the whole drop failed with "could not detect",
    /// because a directory is not a file.
    #[test]
    fn a_dropped_folder_expands_to_its_convertible_files() {
        let root = std::env::temp_dir().join(format!("tx-drop-expand-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("nested")).unwrap();
        std::fs::write(root.join("a.png"), b"x").unwrap();
        std::fs::write(root.join("b.JPG"), b"x").unwrap();
        std::fs::write(root.join("notes.txt.receipt.json"), b"{}").unwrap();
        std::fs::write(root.join("build.log"), b"x").unwrap();
        std::fs::write(root.join("nested").join("c.webp"), b"x").unwrap();

        let mut out = ExpandedDrop {
            files: Vec::new(),
            folders: 0,
            skipped: 0,
            truncated: false,
        };
        walk_drop(&root, 0, &mut out);
        out.files.sort();

        let names: Vec<String> = out
            .files
            .iter()
            .map(|f| {
                std::path::Path::new(f)
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, vec!["a.png", "b.JPG", "c.webp"], "got {names:?}");
        // `build.log` is skipped; the sidecar is not counted as a candidate
        // the user might have wanted.
        assert!(!out.truncated);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Extensions decide only what a FOLDER contributes. A file the user
    /// dropped by hand is an explicit request, and detection — not its
    /// extension — settles what it is (SR-4).
    #[test]
    fn extension_filtering_applies_to_folders_only() {
        assert!(is_droppable(std::path::Path::new("x/a.png")));
        assert!(is_droppable(std::path::Path::new("x/a.PNG")));
        assert!(!is_droppable(std::path::Path::new("x/a.log")));
        assert!(!is_droppable(std::path::Path::new("x/a.jpg.receipt.json")));
        assert!(!is_droppable(std::path::Path::new("x/noext")));
    }

    #[test]
    fn targets_parse_by_name_or_extension() {
        assert_eq!(parse_target("jpeg").unwrap(), parse_target("jpg").unwrap());
        assert_eq!(parse_target(".PNG").unwrap(), parse_target("png").unwrap());
        assert!(matches!(
            parse_target("strip-metadata").unwrap(),
            Target::Operation(openconvert_core::target::Operation::StripMetadata)
        ));
        assert!(parse_target("definitely-not-a-format").is_err());
    }

    /// Suggestions are filtered through the route table before scoring, so
    /// nothing offered can be refused by the plan preview a moment later.
    #[test]
    fn every_suggestion_is_a_target_that_routes() {
        let dir = scratch("suggest");
        let path = png_at(&dir, "photo.png");
        let paths = vec![path.to_string_lossy().into_owned()];

        let groups = predictions_for(&paths);
        let group = groups.first().expect("one group for one PNG");
        assert_eq!(group.input, "png");
        assert_eq!(group.kind, "image");
        assert!(!group.why.is_empty(), "a suggestion always says why");

        assert!(
            !group.suggestions.is_empty(),
            "PNG routes somewhere on any machine that can run this test"
        );

        for s in &group.suggestions {
            assert_ne!(
                s.target, group.input,
                "the guesser offered the source format"
            );
            assert_ne!(
                s.target, "strip-metadata",
                "metadata is not a format choice"
            );
            let preview = plan_for(&paths, &s.target);
            assert!(
                preview.executable,
                "{} was offered and then refused by the plan preview: {:?}",
                s.target, preview.warnings
            );
            assert_ne!(
                s.class_name, "—",
                "{} was offered without a fidelity cost",
                s.target
            );
        }
    }

    /// A mixed drop is several decisions, and the backend is what splits it.
    #[test]
    fn a_mixed_drop_is_grouped_by_what_the_bytes_say() {
        let dir = scratch("grouped");
        let a = png_at(&dir, "one.png");
        let b = png_at(&dir, "two.png");
        let c = dir.join("notes.txt");
        std::fs::write(&c, b"not a format we know").expect("write");

        let paths = vec![
            a.to_string_lossy().into_owned(),
            c.to_string_lossy().into_owned(),
            b.to_string_lossy().into_owned(),
        ];
        let groups = predictions_for(&paths);

        assert_eq!(groups.len(), 2, "two formats, two groups: {groups:?}");
        let png = groups
            .iter()
            .find(|g| g.input == "png")
            .expect("a png group");
        assert_eq!(png.paths.len(), 2, "both PNGs landed together");
    }

    /// The naming template accepts exactly its four tokens, needs `{name}`,
    /// and is not a path.
    #[test]
    fn naming_templates_validate_before_they_render() {
        for good in [
            "{name}.{ext}",
            "{date}_{name}.{ext}",
            "converted-{index}-{name}.{ext}",
        ] {
            assert!(validate_naming_template(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "  ",
            "{name}/{ext}",
            "..\\{name}",
            "c:{name}",
            "{nane}.{ext}",
            "{name",
            "{name}.{ext}.{oops}",
            // Losing the original stem loses the file's identity.
            "{ext}",
            "{date}.{index}",
        ] {
            assert!(validate_naming_template(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn naming_templates_render_through_the_canonical_module() {
        let ctx = |name, index| openconvert_run::naming::RenderContext {
            name,
            ext: "png",
            date: "2026-08-24",
            index,
        };
        assert_eq!(
            openconvert_run::naming::render("{name}.{ext}", &ctx("photo", 3)).unwrap(),
            "photo.png"
        );
        // The shell's own policy on top of the canonical tokens.
        assert!(validate_naming_template("shot-{name}-{index}.{ext}").is_ok());
        assert!(validate_naming_template("{ext}").is_err());
    }

    /// Replace-originals: the input survives in the quarantine and Undo can
    /// put it back byte-for-byte.
    #[test]
    fn replace_originals_quarantine_and_restore_round_trip() {
        let dir = scratch("replace");
        let original_bytes: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        let original = dir.join("keep.png");
        std::fs::write(&original, &original_bytes).expect("fixture"); // openconvert-lint: allow -- test scratch
        let output = dir.join("keep.png.out");

        // A different-named output: the output keeps its own name; the
        // original lands in quarantine.
        std::fs::write(&output, b"converted bytes").unwrap(); // openconvert-lint: allow -- test scratch
        let quarantined =
            replace_original_in(&output, &original, "keep.png.out", &dir.join("quarantine"))
                .expect("replaced");
        assert!(!original.exists(), "the vacated name is free");
        assert!(
            !dir.join(format!("keep.png.replacing-{}", std::process::id()))
                .exists(),
            "the holding name is cleaned up"
        );
        assert_eq!(
            std::fs::read(&quarantined).expect("readable"),
            original_bytes,
            "the quarantine holds every byte"
        );

        restore_original(&quarantined, &original).expect("restored");
        assert_eq!(std::fs::read(&original).expect("readable"), original_bytes);
        assert!(!quarantined.exists());
    }

    /// Same-format replacement: the output takes the original's exact name.
    #[test]
    fn a_same_format_replacement_takes_over_the_name() {
        let dir = scratch("replace-same");
        let original = dir.join("shot.jpg");
        std::fs::write(&original, b"original jpeg bytes").unwrap(); // openconvert-lint: allow -- test scratch

        // Simulate strip-metadata on the same format: the pipeline wrote its
        // O_EXCL output under a suffixed name in the SAME directory.
        let suffixed = dir.join("shot (2).jpg");
        std::fs::write(&suffixed, b"cleaned jpeg bytes").unwrap(); // openconvert-lint: allow -- test scratch

        let quarantined =
            replace_original_in(&suffixed, &original, "shot.jpg", &dir.join("quarantine"))
                .expect("replaced");
        assert_eq!(
            std::fs::read(&original).expect("readable"),
            b"cleaned jpeg bytes",
            "the cleaned output now owns the original's name"
        );
        assert!(!suffixed.exists());
        assert_eq!(
            std::fs::read(&quarantined).expect("readable"),
            b"original jpeg bytes",
            "and the original waits in the quarantine"
        );
    }

    #[test]
    fn a_receipt_database_failure_removes_output_and_restores_original() {
        let dir = scratch("receipt-rollback");
        let original = dir.join("shot.jpg");
        let staged = dir.join("shot (2).jpg");
        std::fs::write(&original, b"original jpeg bytes").unwrap(); // openconvert-lint: allow -- test scratch
        std::fs::write(&staged, b"cleaned jpeg bytes").unwrap(); // openconvert-lint: allow -- test scratch

        let quarantined =
            replace_original_in(&staged, &original, "shot.jpg", &dir.join("quarantine"))
                .expect("replaced");
        let replaced = (quarantined.clone(), original.clone());
        let error = commit_receipt_with(
            r#"{"output":"shot.jpg","steps":[]}"#,
            "shot.jpg",
            true,
            &original,
            Some(&replaced),
            |_, _| Err("simulated database failure".to_string()),
        )
        .expect_err("database failure must fail the conversion");

        assert!(error.contains("simulated database failure"));
        assert_eq!(
            std::fs::read(&original).expect("original restored"),
            b"original jpeg bytes"
        );
        assert!(!staged.exists(), "the newly-created output is gone");
        assert!(
            !quarantined.exists(),
            "the quarantine was consumed by restore"
        );
    }

    #[test]
    fn a_tool_receipt_database_failure_removes_its_new_output() {
        let dir = scratch("tool-receipt-rollback");
        let output = dir.join("inverted.png");
        std::fs::write(&output, b"new tool output").unwrap(); // openconvert-lint: allow -- test scratch

        let error = commit_receipt_with(
            r#"{"output":"inverted.png","steps":[]}"#,
            "photo.png",
            true,
            &output,
            None,
            |_, _| Err("simulated database failure".to_string()),
        )
        .expect_err("database failure must fail the tool run");

        assert!(error.contains("simulated database failure"));
        assert!(!output.exists(), "the tool output was compensated");
    }

    #[test]
    fn today_is_a_date() {
        let t = today();
        assert_eq!(t.len(), 10);
        let bytes = t.as_bytes();
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[7], b'-');
        assert!(bytes
            .iter()
            .enumerate()
            .all(|(i, b)| { matches!(i, 4 | 7) || b.is_ascii_digit() }));
    }

    /// A patch carries one key, and must leave every other key alone.
    ///
    /// The UI sends exactly one field per change, so an assignment rather than
    /// a move-if-present silently reset everything else: changing the theme
    /// erased the output destination, the naming template and the receipt
    /// preference. This is the regression test for that.
    #[test]
    fn a_partial_patch_leaves_every_other_key_alone() {
        let mut config = UserConfig::default();
        merge_patch(
            &mut config,
            ConfigPatch {
                output_destination: Some("downloads".into()),
                naming_template: Some("{name}-{date}.{ext}".into()),
                write_receipts: Some(false),
                ..ConfigPatch::default()
            },
        )
        .expect("the first patch applies");

        // A second patch touching only the theme.
        merge_patch(
            &mut config,
            ConfigPatch {
                theme: Some("dark".into()),
                ..ConfigPatch::default()
            },
        )
        .expect("the second patch applies");

        assert_eq!(config.theme.as_deref(), Some("dark"));
        assert_eq!(
            config.output_destination.as_deref(),
            Some("downloads"),
            "an unrelated key was erased"
        );
        assert_eq!(
            config.naming_template.as_deref(),
            Some("{name}-{date}.{ext}"),
            "an unrelated key was erased"
        );
        assert_eq!(
            config.write_receipts,
            Some(false),
            "an unrelated key was erased"
        );
    }

    /// Worker reuse only ever ratchets up.
    #[test]
    fn worker_reuse_reads_the_config_and_never_lowers() {
        use openconvert_core::policy::WorkerReuse;

        let mut config = UserConfig::default();
        assert_eq!(
            policy_from(&config).worker_reuse(),
            WorkerReuse::Balanced,
            "the default is the policy default"
        );

        config.worker_reuse = Some("isolated".into());
        assert_eq!(policy_from(&config).worker_reuse(), WorkerReuse::Isolated);

        // Asking for the weaker value cannot lower an already-raised policy —
        // `raise_worker_reuse` takes a maximum, and the ConfigJson reports the
        // effective value rather than the requested one.
        config.worker_reuse = Some("balanced".into());
        assert_eq!(
            policy_from(&config).worker_reuse(),
            Policy::default().worker_reuse(),
            "balanced means do-not-raise, not lower-to-balanced"
        );
    }

    /// Downloads is a real destination, refused only when the folder is absent.
    #[test]
    fn downloads_is_an_accepted_destination() {
        let mut config = UserConfig::default();
        merge_patch(
            &mut config,
            ConfigPatch {
                output_destination: Some("downloads".into()),
                ..ConfigPatch::default()
            },
        )
        .expect("downloads is accepted");
        assert_eq!(ConfigJson::from(&config).output_destination, "downloads");

        assert!(
            merge_patch(
                &mut config,
                ConfigPatch {
                    output_destination: Some("dropbox".into()),
                    ..ConfigPatch::default()
                },
            )
            .is_err(),
            "an unknown destination is still refused"
        );
    }

    /// Config patches validate before they touch the file.
    #[test]
    fn config_patches_refuse_values_outside_their_enums() {
        let mut config = UserConfig::default();

        assert!(
            merge_patch(
                &mut config,
                ConfigPatch {
                    theme: Some("midnight".into()),
                    ..ConfigPatch::default()
                }
            )
            .is_err(),
            "unknown themes refuse"
        );
        assert!(config.theme.is_none(), "a refused patch changes nothing");

        assert!(
            merge_patch(
                &mut config,
                ConfigPatch {
                    naming_template: Some("{name}/../{ext}".into()),
                    ..ConfigPatch::default()
                }
            )
            .is_err(),
            "path traversal refuses at validation, not at write time"
        );

        assert!(merge_patch(
            &mut config,
            ConfigPatch {
                theme: Some("dark".into()),
                model_auto_update: Some(false),
                write_receipts: Some(false),
                output_destination: Some("desktop".into()),
                naming_template: Some("{date}-{name}.{ext}".into()),
                worker_reuse: Some("isolated".into()),
                force_sandbox: Some(true),
                use_gpu: Some(false),
                worker_memory_mb: Some(8192),
                ai_setup_done: Some(true),
            }
        )
        .is_ok());
        let effective = ConfigJson::from(&config);
        assert_eq!(effective.theme, "dark");
        assert_eq!(effective.worker_memory_mb, 8192);
        assert!(
            !effective.use_gpu,
            "turning the GPU off must survive the round trip through Policy"
        );
        assert!(!effective.write_receipts);
        assert!(
            !effective.model_auto_update,
            "the one network preference has to survive a round trip;              the last three that did not were removed for it"
        );
    }

    /// The window that opened onto a monitor nobody has.
    ///
    /// A saved position is not a fact about the desktop — it is a fact about
    /// the desktop as it was the last time the app closed. Restoring it
    /// unchecked is how the program came up on a screen that had since been
    /// unplugged: no window on any display, no error, and no way back except
    /// editing the config by hand.
    #[test]
    fn a_saved_position_is_only_restored_when_it_is_still_on_a_screen() {
        // One laptop display at the origin, and nothing else.
        let laptop = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };

        let on_screen = Rect {
            x: 100,
            y: 80,
            w: 900,
            h: 680,
        };
        assert!(
            is_visible_on(on_screen, &[laptop]),
            "a window inside the only display is visible"
        );

        // Where the second monitor used to be. This is the exact shape of the
        // bug: perfectly valid coordinates that name no pixel that exists.
        let stranded = Rect {
            x: 2200,
            y: 300,
            w: 900,
            h: 680,
        };
        assert!(
            !is_visible_on(stranded, &[laptop]),
            "a window at the coordinates of a monitor that is gone is not visible"
        );
        // ...and with that monitor plugged back in, it is fine again.
        let external = Rect {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert!(
            is_visible_on(stranded, &[laptop, external]),
            "the same position is valid again once the display is back"
        );

        // A sliver hanging off the edge is not "visible" in any useful sense:
        // there is nothing left to grab and drag back.
        let sliver = Rect {
            x: 1900,
            y: 1060,
            w: 900,
            h: 680,
        };
        assert!(
            !is_visible_on(sliver, &[laptop]),
            "a corner poking onto the desktop is not a recoverable window"
        );

        // Straddling two screens is visible, and belongs to neither.
        let straddling = Rect {
            x: 1700,
            y: 200,
            w: 900,
            h: 680,
        };
        assert!(
            is_visible_on(straddling, &[laptop, external]),
            "overlap is summed across displays, not taken from the best one"
        );

        // No monitors reported at all is not evidence of anything, and the
        // caller leaves the saved position alone in that case.
        assert!(
            !is_visible_on(on_screen, &[]),
            "with no displays to check against there is no overlap to find"
        );
    }

    /// Where a stranded window lands instead.
    #[test]
    fn a_stranded_window_is_centred_on_the_primary_display() {
        let primary = Rect {
            x: 0,
            y: 0,
            w: 1920,
            h: 1080,
        };
        assert_eq!(centred_on(primary, 900, 680), (510, 200));

        // A primary display that is not at the origin (the OS puts the second
        // monitor left of the first) still gets its own coordinates back.
        let offset = Rect {
            x: -2560,
            y: 0,
            w: 2560,
            h: 1440,
        };
        assert_eq!(centred_on(offset, 900, 680), (-1730, 380));

        // A window larger than the display pins to the top-left corner rather
        // than centring to a negative offset that hides its own controls.
        let tiny = Rect {
            x: 0,
            y: 0,
            w: 800,
            h: 600,
        };
        assert_eq!(centred_on(tiny, 1200, 900), (0, 0));
    }

    // -----------------------------------------------------------------------
    // Prediction signals: decay, and the folder pattern
    // -----------------------------------------------------------------------

    /// A habit fades, and it fades at the stated rate.
    ///
    /// Without this, history is a lifetime tally and it takes as many
    /// conversions to leave a format as it took to adopt one -- so somebody who
    /// moved to WebP last month keeps being offered JPEG.
    #[test]
    fn history_decays_at_the_stated_half_life() {
        const DAY: f32 = 86_400.0;
        assert!((decay(0.0) - 1.0).abs() < 1e-6, "today counts fully");
        assert!(
            (decay(HALF_LIFE_DAYS * DAY) - 0.5).abs() < 1e-3,
            "one half-life should halve it, got {}",
            decay(HALF_LIFE_DAYS * DAY)
        );
        assert!(
            (decay(2.0 * HALF_LIFE_DAYS * DAY) - 0.25).abs() < 1e-3,
            "two half-lives should quarter it"
        );

        // Monotonic: older is never worth more.
        let mut prev = f32::INFINITY;
        for d in 0..400 {
            let v = decay(d as f32 * DAY);
            assert!(v <= prev + 1e-6, "weight rose with age at day {d}");
            prev = v;
        }
    }

    /// A journal from the future cannot manufacture evidence.
    ///
    /// Clock changes, restored backups and copied profiles all produce mtimes
    /// ahead of now. `duration_since` fails on those and the age falls back to
    /// zero, but the clamp is what makes that safe rather than incidental.
    #[test]
    fn decay_is_bounded_over_hostile_ages() {
        for age in [
            -1.0,
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::MAX,
            0.0,
        ] {
            let v = decay(age);
            assert!(v.is_finite(), "decay({age}) was {v}");
            assert!((0.0..=1.0).contains(&v), "decay({age}) was {v}");
        }
    }

    /// The folder pattern counts OTHER files, and pairs them by stem.
    ///
    /// This is the signal that replaced a same-stem check floored at 0.90. It
    /// has to see `a.heic`/`a.jpg` and `b.heic`/`b.jpg` as two examples, ignore
    /// the file being predicted for, and ignore a `.jpg` with no `.heic`
    /// beside it.
    #[test]
    fn the_folder_pattern_pairs_by_stem_and_excludes_itself() {
        let dir = std::env::temp_dir().join(format!("tx-folder-pattern-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        for name in [
            "a.heic", "a.jpg", // a converted pair
            "b.heic", "b.jpg",  // another
            "c.heic", // the file being predicted for, not yet converted
            "d.jpg",  // an output with no source: not a pair
            "e.heic", // a source with no output: not a pair
        ] {
            std::fs::write(dir.join(name), b"x").expect("fixture");
        }

        let subject = dir.join("c.heic");
        assert_eq!(
            folder_pattern(&subject, FormatId::Heic, FormatId::Jpeg),
            2,
            "expected the two converted pairs, and only those"
        );

        // The subject does not count itself once it HAS been converted.
        std::fs::write(dir.join("c.jpg"), b"x").expect("fixture");
        assert_eq!(
            folder_pattern(&subject, FormatId::Heic, FormatId::Jpeg),
            2,
            "a file must not be evidence about itself"
        );

        // A target nobody in the folder has used scores nothing.
        assert_eq!(folder_pattern(&subject, FormatId::Heic, FormatId::Webp), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A folder with nothing in it, and a path with no parent, are both zero
    /// rather than a panic.
    #[test]
    fn the_folder_pattern_is_total() {
        let missing = Path::new("Z:/no/such/folder/file.heic");
        assert_eq!(folder_pattern(missing, FormatId::Heic, FormatId::Jpeg), 0);
        assert_eq!(
            folder_pattern(missing, FormatId::Unknown, FormatId::Jpeg),
            0,
            "a format with no row has no extension to match"
        );
    }
    #[test]
    fn native_preview_reads_indirect_page_geometry_and_requested_resolution() {
        openconvert_run::engine_dir::set_resource_dir(std::path::PathBuf::from(env!(
            "CARGO_MANIFEST_DIR"
        )));
        let dir = scratch("indirect-preview");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 /MediaBox 4 0 R >>",
            "<< /Type /Page /Parent 2 0 R >>",
            "[0 0 5 0 R 400]",
            "300",
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let path = dir.join("indirect.pdf");
        std::fs::write(&path, bytes).unwrap();
        let result = render_preview(&path, 1, 800, &[]).unwrap();
        assert_eq!(result.page_geometry_error, None);
        assert_eq!(result.page_width_pt, Some(300.0));
        assert_eq!(result.page_height_pt, Some(400.0));
        assert_eq!(result.width.max(result.height), 800);
    }
    #[test]
    fn native_preview_renders_large_multipage_images_at_zoom_resolutions() {
        openconvert_run::engine_dir::set_resource_dir(std::path::PathBuf::from(env!(
            "CARGO_MANIFEST_DIR"
        )));
        let image = image::RgbImage::from_fn(1600, 2000, |x, y| {
            image::Rgb([(x * 3 + y) as u8, (x + y * 2) as u8, (x * y) as u8])
        });
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
            .encode_image(&image)
            .unwrap();
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R 4 0 R 5 0 R] /Count 3 >>".to_vec(),
        ];
        for _ in 0..3 {
            objects.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] /Resources << /XObject << /Scan 6 0 R >> >> /Contents 7 0 R >>".to_vec());
        }
        let mut stream=format!("<< /Type /XObject /Subtype /Image /Width 1600 /Height 2000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",jpeg.len()).into_bytes();
        stream.extend(jpeg);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
        let content = b"q 600 0 0 800 0 0 cm /Scan Do Q";
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}\nendstream",
                content.len(),
                String::from_utf8_lossy(content)
            )
            .into_bytes(),
        );
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            bytes.extend(object);
            bytes.extend_from_slice(b"\nendobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(b"xref\n0 8\n0000000000 65535 f \n");
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!("trailer\n<< /Size 8 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        let dir = scratch("large-preview");
        let path = dir.join("scan.pdf");
        std::fs::write(&path, bytes).unwrap();
        for (page, px) in [(1, 800), (3, 1600), (2, 3200)] {
            let start = std::time::Instant::now();
            let result = render_preview(&path, page, px, &[]).unwrap();
            assert_eq!(result.page_count, 3);
            assert_eq!(result.page_geometry_error, None);
            assert_eq!(result.width.max(result.height), px);
            eprintln!(
                "Native image PDF preview page {page}, {px}px: {:?}",
                start.elapsed()
            );
        }
    }
    #[test]
    fn compression_preview_reports_actual_sizes_for_each_level() {
        openconvert_run::engine_dir::set_resource_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let dir = scratch("compression-preview");
        let path = dir.join("gradient.png");
        let img = image::RgbaImage::from_fn(80, 60, |x, y| {
            image::Rgba([(x * 3) as u8, (y * 4) as u8, 37, ((x + y) * 2) as u8])
        });
        img.save(&path).unwrap();
        let original = std::fs::metadata(&path).unwrap().len();
        for quality in ["85", "65", "20"] {
            let preview = render_preview(&path, 1, 160, &[format!("compress:{quality}")]).unwrap();
            assert_eq!(preview.original_bytes, Some(original));
            assert!(preview
                .compressed_bytes
                .is_some_and(|size| size > 0 && size <= original));
            assert!(preview.data_uri.starts_with("data:image/png;base64,"));
            eprintln!(
                "Compression preview {quality}: {original} -> {:?}",
                preview.compressed_bytes
            );
        }
    }

    #[test]
    fn greyscale_preview_preserves_cutout_transparency() {
        let img = image::RgbaImage::from_pixel(8, 8, image::Rgba([90, 180, 25, 64]));
        let mut source = std::io::Cursor::new(Vec::new());
        img.write_to(&mut source, image::ImageFormat::Png).unwrap();
        let (out, _, _) = downscale_png(source.get_ref(), 16, &["image-greyscale".into()]).unwrap();
        let decoded = image::load_from_memory(&out).unwrap().to_rgba8();
        assert!(decoded
            .pixels()
            .all(|p| p[3] == 64 && p[0] == p[1] && p[1] == p[2]));
    }
}
