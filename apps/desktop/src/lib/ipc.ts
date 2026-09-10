/**
 * The whole contract with the Rust backend (INTERFACES.md §11).
 *
 * Every interface here mirrors a `#[derive(Serialize)]` struct in
 * `src-tauri/src/main.rs` carrying `#[serde(rename_all = "camelCase")]`.
 * **A field added on one side and not the other is silent data loss**, so the
 * two files are edited together or not at all.
 *
 * Rust `Option<T>` serialises as `T | null` — never `undefined`, and never an
 * absent key. The optional-property form (`x?: number`) is used only where the
 * Rust side is `Option<T>` inside `PropertiesJson`, which is also `| null`;
 * both spellings are accepted at a read site that checks for a value.
 *
 * This module is the only place in the frontend that names a command string.
 * Everything else calls a typed function, so a renamed command is a compile
 * error rather than a runtime `undefined`.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Format-specific facts, flattened. `kind` carries the discriminant. */
export interface PropertiesJson {
  kind: "image" | "audio" | "video" | "document" | "archive" | "tabular" | "unknown";
  width: number | null;
  height: number | null;
  hasAlpha: boolean | null;
  frames: number | null;
  durationMs: number | null;
  channels: number | null;
  sampleRate: number | null;
  videoCodec: string | null;
  audioCodec: string | null;
  pages: number | null;
  encrypted: boolean | null;
  entries: number | null;
  depth: number | null;
  rows: number | null;
  columns: number | null;
}

/** What one file is. */
export interface ProbeResult {
  /** The path as given. Addressing only — never rendered. */
  path: string;
  /** Rendered through `DisplayName`. Safe to show. */
  fileName: string;
  detected: string;
  declared: string | null;
  mismatched: boolean;
  polyglot: boolean;
  /** True when rendering `fileName` had to escape or truncate something. */
  nameAltered: boolean;
  inputBytes: number;
  properties: PropertiesJson | null;
}

export type ClassName = "A" | "B" | "C" | "D";

/** One step of a plan, already rendered by the backend. */
export interface PlanStep {
  fileName: string;
  /** e.g. `Transcode { from: heic, to: jpeg }`. */
  kind: string;
  className: ClassName;
  /** `"in-process"` or `"sandboxed (elevated)"`. */
  isolation: string;
  /** `"1024 MiB memory, 256 Mpx decode, 120s wall"`. */
  limitsSummary: string;
  engineName: string;
}

export interface Warning {
  fileName: string;
  blocking: boolean;
  message: string;
}

/** What *would* happen. `route()` without `execute()`. */
export interface PlanPreview {
  executable: boolean;
  steps: PlanStep[];
  warnings: Warning[];
  /** **0 means "not determined"**, never "instant". Nothing estimates yet. */
  estimatedDurationMs: number;
  totalInputBytes: number;
}

/** What happened to one file. */
export interface ConversionResult {
  fileName: string;
  outputPath: string;
  receiptPath: string;
  success: boolean;
  errorMessage: string | null;
  durationMs: number;
  inputBytes: number;
  outputBytes: number;
  /** `"A (lossless)"`, `"B (lossy, standard)"`, … */
  classApplied: string;
  removedMetadata: string[];
  /** Blake3 hex of the bytes converted. Empty for failures. */
  contentId: string;
  /** The receipt's own account of the run, when there is one. */
  receiptDetail: ReceiptDetail | null;
}

/** One executed step, as the receipt recorded it. */
export interface StepReceipt {
  kind: string;
  class: string;
  /** Engine name. Versions are absent from this build's receipts. */
  engine: string;
  isolation: string;
  limitsSummary: string;
}

/** The receipt, carried with the result so the done screen renders it. */
export interface ReceiptDetail {
  version: number;
  tool: string;
  /** Input identity — the hash of the bytes actually converted. */
  contentId: string;
  detected: string;
  /** What the filename claimed, when it disagreed with the content. */
  declaredMismatch: string | null;
  class: string | null;
  outputName: string;
  outputBytes: number;
  steps: StepReceipt[];
}

/** One candidate target, scored by the backend's predictor. */
export interface Suggestion {
  /** Pass this back as `targetFormat`. */
  target: string;
  score: number;
  /** The worst class the best route for this pair costs, e.g. `"B (lossy, standard)"`. */
  className: string;
  armed: boolean;
}

/** What to offer for one group of same-format files. */
export interface Prediction {
  input: string;
  kind: PropertiesJson["kind"];
  paths: string[];
  suggestions: Suggestion[];
  /** One line saying why the top suggestion is the top suggestion. */
  why: string;
}

/**
 * Emitted while a model downloads.
 *
 * `totalBytes` is 0 when the source declares no length — the bar shows
 * indeterminate rather than inventing a denominator.
 */
export interface ModelProgress {
  id: string;
  receivedBytes: number;
  totalBytes: number;
  phase: "downloading" | "verifying" | "done" | "failed";
  /** Present on `failed`; already safe to display. */
  message: string | null;
}

/**
 * A rendered preview of one file.
 *
 * **The backend rasterises; the webview only displays.** The bytes here are a
 * PNG this app produced in a confined worker, never the original file — so the
 * webview's decoder never sees attacker-chosen input, which is the whole reason
 * `img-src 'self' data:` is the only image source the CSP permits.
 */
export interface FilePreview {
  originalBytes?: number | null;
  compressedBytes?: number | null;
  /** `data:image/png;base64,…`, ready for an `<img src>`. */
  dataUri: string;
  width: number;
  height: number;
  /** Total pages for a paged document; 1 for everything else. */
  pageCount: number;
  /** Which page this render is, 1-based. */
  page: number;
  /**
   * Physical page size in PDF points (1/72 in), when the renderer reported it.
   *
   * Null until `oc-pdf` returns the MediaBox. A4 and A3 share the same 1:√2
   * ratio, so a paper size cannot be recovered from the raster — the interface
   * shows the measured aspect instead of naming a size it cannot know.
   */
  pageGeometryError?: string | null;
  pageWidthPt: number | null;
  pageHeightPt: number | null;
  pageRotation?: number | null;
}

/** Emitted per file while a batch runs. */
export interface ProgressEvent {
  index: number;
  total: number;
  fileName: string;
  phase: "started" | "done" | "failed" | "cancelled";
}

// ---------------------------------------------------------------------------
// Settings, history, receipts, models, tools — INTERFACES.md §11.
// ---------------------------------------------------------------------------

/** The effective configuration. Defaults are filled in by the backend. */
export interface Config {
  /**
   * How aggressively worker processes are reused between files.
   *
   * `balanced` reuses one sandboxed worker across files that share a
   * **trusted** provenance; anything carrying mark-of-the-web, a quarantine
   * xattr, or arriving from a network/removable volume always gets its own
   * regardless (SR-20). `isolated` gives every file its own, at roughly 38 ms
   * more per file on Windows.
   *
   * The axis is trust, not folder: two files in one folder do not share a
   * worker because they are neighbours, they share one because both are
   * trusted.
   */
  workerReuse: "balanced" | "isolated";
  /**
   * Confine every step that has a worker, including the pure-Rust ones.
   *
   * Defence in depth, not completeness. Routing decides isolation from the
   * format table, and the in-process set it leaves is memory-safe code we
   * compile ourselves; this confines it anyway, at a process launch per file.
   *
   * **It cannot reach everything.** CSV/JSON and Matroska stream copy have no
   * worker to be confined into, so they stay in-process however this is set.
   */
  forceSandbox: boolean;
  /**
   * Whether the first-run AI chooser has been answered.
   *
   * Asked once, including when the answer was "none of them": a prompt that
   * returns because the user declined teaches people to dismiss it unread.
   */
  aiSetupDone: boolean;
  /**
   * True when a managed policy layer pinned `isolated`.
   *
   * `Policy::raise_worker_reuse` is a ratchet — a later layer may raise the
   * setting and may never lower it — so the control is shown disabled rather
   * than accepting a change the backend would discard.
   */
  workerReuseLocked: boolean;
  defaultFormat: string | null;
  theme: "system" | "light" | "dark";
  /**
   * Whether conversions may use a GPU where one measurably helps.
   *
   * On by default. The heavy models — upscaling, transcription, OCR — are
   * several times faster on a GPU; the small ones are slower on it and are
   * never sent there. Anything that cannot run on the GPU, including a model
   * too large for the card's memory, falls back to the CPU on its own.
   *
   * A ratchet, like {@link Config.forceSandbox}: this can turn the GPU off,
   * and no layer below can turn it back on.
   */
  useGpu: boolean;
  /**
   * Memory one conversion may use, in MB.
   *
   * The effective value, after the policy has bounded it -- never below the
   * 1024 MB default, never above {@link Config.workerMemoryMaxMb}.
   *
   * This is a limit that protects, not a performance dial: it is what stops a
   * decompression bomb or a hostile image header from exhausting the machine,
   * and ordinary conversion never approaches it. It is adjustable because a
   * few models genuinely need more -- the best background-removal model peaks
   * near 6.5 GB on one small image -- and refusing them silently is worse than
   * letting someone raise the number knowingly.
   *
   * Unlike {@link Config.workerReuse} and {@link Config.forceSandbox}, which
   * are ratchets, this one lowers again.
   */
  workerMemoryMb: number;
  /** The largest this build accepts, in MB. */
  workerMemoryMaxMb: number;
  /** Re-fetch model artifacts this build pins differently. Default on. */
  modelAutoUpdate: boolean;
  writeReceipts: boolean;
  outputDestination: "same_folder" | "downloads" | "desktop" | "replace_source";
  namingTemplate: string;
}

/** One `set_config` patch. Absent keys mean unchanged. */
export type ConfigPatch = Partial<
  Pick<
    Config,
    | "workerReuse"
    | "forceSandbox"
    | "useGpu"
    | "workerMemoryMb"
    | "aiSetupDone"
    | "theme"
    | "modelAutoUpdate"
    | "writeReceipts"
    | "outputDestination"
    | "namingTemplate"
  >
>;

/** One past conversion, from the batch journals. */
export interface HistoryEntry {
  outputPath?: string;
  tool?: string;
  request?: {paths: string[]; target: string; params?: Record<string,string | number | boolean>} | null;
  sourceName: string;
  outputName: string;
  contentId: string;
  outcome: "completed" | "failed" | "skipped";
  reason: string | null;
  /** Size of the output in bytes; 0 when nothing was written. */
  outputBytes: number;
  /** Full path to the receipt sidecar, or null for a failure. */
  receiptPath: string | null;
  /** Batch-file mtime in seconds — the journals carry no per-line clock. */
  whenSecs: number;
}

/** One stored receipt, summarised for the settings list. */
export interface ReceiptEntry {
  /** Store id — pass back to deleteReceipt. */
  id: string;
  /** ISO timestamp, from the record when it carries one, mtime otherwise. */
  date: string;
  sourceName: string | null;
  outputName: string;
  outputBytes: number;
}

/**
 * One AI capability, as a person would choose it.
 *
 * Not a model file: nobody wants "paddleocr-det", they want to read text out of
 * a scan, and that takes three artifacts. `sizeBytes` is summed from the
 * registry rows the capability needs, so the number shown is the number
 * fetched.
 */
export interface AiFeature {
  id: string;
  title: string;
  /** What it does, in one line. */
  does: string;
  /** Total packed size of every artifact it needs. */
  sizeBytes: number;
  /** Whether every artifact is already in the local store. */
  downloaded: boolean;
  /**
   * Whether anything in the app can actually reach it.
   *
   * Listed either way — hiding a capability would make the chooser and the
   * registry disagree — but never offered for download when false.
   */
  usable: boolean;
  licences: string[];
  /** The tool these artifacts serve. Several features may share one. */
  tool: string | null;
  /** `small` | `better` | `best`. */
  tier: string;
  /**
   * Whether the first-run screen offers this tier.
   *
   * Settings shows every tier; the first-run screen shows the ones worth
   * fetching before the program has been used. Same field the installer's
   * licence page filters on, so the two lists cannot disagree.
   */
  offeredAtInstall: boolean;
  /** Every artifact present AND switched on. A tier is all-or-nothing. */
  ready: boolean;
  /** The tier a run would pick right now. */
  active: boolean;
  models: string[];
}

/** One declared model with its per-user state. */
export interface ModelInfo {
  id: string;
  title: string;
  purpose: string;
  sizeBytes: number;
  licence: string;
  downloaded: boolean;
  enabled: boolean;
  /** Why this model cannot be downloaded, or null when it can.
   *
   * Present so the button can be disabled with the reason beside it, rather
   * than armed and certain to fail. */
  blocked: string | null;
}

/** One permitted value of a `choice` parameter. */
export interface ToolOption {
  /** Sent to the handler verbatim. */
  value: string;
  label: string;
}

/**
 * One parameter of one tool; the workspace renders these generically.
 *
 * The control follows from `kind`, so a tool gains a slider or a set of buttons
 * by describing itself differently — not by this file learning its name.
 */
export interface ToolParam {
  id: string;
  title: string;
  kind: "number" | "text" | "choice" | "range";
  required: boolean;
  /** Present for `choice`. */
  options?: ToolOption[];
  /** Present for `range`. */
  min?: number;
  max?: number;
  step?: number;
  /** What the tool does when the parameter is absent. Shown pre-selected. */
  default?: string;
}

/** What one tool looks like; the menu renders this generically. */
export interface ToolDescriptor {
  id: string;
  category: string;
  title: string;
  available: boolean;
  unavailableReason: string | null;
  multiInput: boolean;
  /** The workspace can show what it does, but nothing can be written yet. */
  previewOnly: boolean;
  params: ToolParam[];
}

// ---------------------------------------------------------------------------
// The typed invoke surface. Nothing else crosses.
// ---------------------------------------------------------------------------

export function probeFile(path: string): Promise<ProbeResult> {
  return invoke("probe_file", { path });
}

export function getPlan(paths: string[], targetFormat: string): Promise<PlanPreview> {
  return invoke("get_plan", { paths, targetFormat });
}

export function suggestTargets(paths: string[]): Promise<Prediction[]> {
  return invoke("suggest_targets", { paths });
}

/**
 * What the interface showed for one file, and what the user did with it.
 *
 * Sent with the conversion so the journal can record the **counterfactual**:
 * not only what ran, but what the app had predicted would run. Without it
 * there is no way to compute whether the suggestion engine is right, and
 * nothing to calibrate `ARM_THRESHOLD` against.
 *
 * It is the shell that sends this, not the backend that recomputes it,
 * because the question is whether the user accepted what they were *shown*.
 */
export interface Choice {
  /** The file, matched by path against the batch. */
  path: string;
  /** The target the app ranked first, as a format name. */
  suggested: string;
  /** That suggestion's score, 0..1. */
  suggestedScore: number;
  /** Where the chosen target sat in the ranking; -1 if it was not in it. */
  chosenRank: number;
}

export function convertBatch(
  paths: string[],
  targetFormat: string,
  choices?: Choice[],
): Promise<ConversionResult[]> {
  return invoke("convert_batch", { paths, targetFormat, choices });
}

/** What a drop expanded to, once folders were walked. */
export interface ExpandedDrop {
  /** Files to convert, in a stable order. */
  files: string[];
  /** How many dropped entries were folders. */
  folders: number;
  /** Files inside those folders that no format row claims. */
  skipped: number;
  /** True when the backend's cap stopped the walk. */
  truncated: boolean;
}

/**
 * Expand a drop into the files it means.
 *
 * A dropped FOLDER arrives as a single path and everything downstream expects
 * files, so without this the whole drop failed with "could not detect" - a
 * poor answer to a reasonable gesture. Dropped files pass through untouched;
 * only folder CONTENTS are filtered by extension.
 */
export function expandPaths(paths: string[]): Promise<ExpandedDrop> {
  return invoke("expand_paths", { paths });
}

/** One row of an exported batch log. */
export interface LogRow {
  fileName: string;
  ok: boolean;
  output: string;
  error: string;
  durationMs: number;
  inputBytes: number;
  outputBytes: number;
  classApplied: string;
}

/**
 * Write the batch outcome to a file the user picks.
 *
 * The save dialog runs host-side (`rfd`), the same route `pickFiles` takes:
 * the webview never gets a filesystem capability, which is what the plugin
 * gate in `xtask/src/desktop.rs` enforces. Resolves to the path written, or
 * `null` when the dialog was dismissed.
 */
export function exportBatchLog(rows: LogRow[]): Promise<string | null> {
  return invoke("export_batch_log", { rows });
}

export function cancelAll(): Promise<void> {
  return invoke("cancel_all");
}

export function undoLast(): Promise<void> {
  return invoke("undo_last");
}

export function getConfig(): Promise<Config> {
  return invoke("get_config");
}

export function setConfig(patch: ConfigPatch): Promise<Config> {
  return invoke("set_config", { patch });
}

/** One stored image in the signature library. */
export interface Signature {
  /** What the user called it, after the backend sanitised it. */
  name: string;
  /** Full path — what `run_tool` is given as the `image` parameter. */
  path: string;
  /** `data:image/png;base64,...`, for the picker and the placement ghost. */
  dataUri: string;
}

/** Every stored signature, sorted by name. */
export function listSignatures(): Promise<Signature[]> {
  return invoke("list_signatures");
}

/** Copy a PNG into the library under `name`. Decoded and re-encoded first. */
export function addSignature(name: string, source: string): Promise<Signature> {
  return invoke("add_signature", { name, source });
}

/** Remove one stored signature. False when it was not there. */
export function deleteSignature(name: string): Promise<boolean> {
  return invoke("delete_signature", { name });
}

/** Read back a `.txt` output a run just produced. The backend checks it. */
export function readTextOutput(path: string): Promise<string> {
  return invoke("read_text_output", { path });
}
export function previewAudio(path: string): Promise<string> { return invoke("preview_audio", { path }); }
export function saveImageCorrections(path:string, original:string, strokes:string):Promise<string>{return invoke("save_image_corrections",{path,original,strokes});}
export function saveTextCopy(path: string, text: string, extension: string): Promise<string> { return invoke("save_text_copy", { path, text, extension }); }

/** Peak amplitude per bucket, 0–255, for drawing a real waveform.
 *
 * The values come from the audio, decoded in the confined worker. The stage
 * used to draw a shape from a seeded RNG and label it with the user's
 * filename. */
export function audioPeaks(path: string, buckets = 96): Promise<number[]> {
  return invoke("audio_peaks", { path, buckets });
}

/** Show a receipt in the OS file manager. The backend checks the path. */
export function revealReceipt(path: string): Promise<void> {
  return invoke("reveal_receipt", { path });
}

/** Show a just-created output in its containing folder. */
export function revealOutput(path: string): Promise<void> {
  return invoke("reveal_output", { path });
}

export function listHistory(limit?: number): Promise<HistoryEntry[]> {
  return invoke("list_history", { limit: limit ?? null });
}

export function wipeHistory(): Promise<number> {
  return invoke("wipe_history");
}

export function listReceipts(limit?: number): Promise<ReceiptEntry[]> {
  return invoke("list_receipts", { limit: limit ?? null });
}

export function receiptsSize(): Promise<number> {
  return invoke("receipts_size");
}

/**
 * What the graphics card is called, or `null`.
 *
 * `null` covers three things that look the same on screen: no card, a platform
 * that cannot say, and a read that failed. The caller renders the switch with
 * nothing beside it, which is what the screen showed before this existed.
 *
 * **A name is not a promise the card will be used.** Whether DirectML loads
 * and whether a model fits are decided per run. The name says which hardware
 * is installed; the switch says whether it has been asked for.
 */
export function gpuDevice(): Promise<string | null> {
  return invoke("gpu_device");
}

export function receiptsCount(): Promise<number> {
  return invoke("receipts_count");
}

export function deleteReceipt(id: string): Promise<void> {
  return invoke("delete_receipt", { id });
}

export function deleteAllReceipts(): Promise<number> {
  return invoke("delete_all_receipts");
}

export function listModels(): Promise<ModelInfo[]> {
  return invoke("list_models");
}

/** Every AI capability, with the size it costs and what it does. */
export function listAiFeatures(): Promise<AiFeature[]> {
  return invoke("list_ai_features");
}

/** Fetch every artifact one capability needs, and switch it on. */
export function downloadAiFeature(id: string): Promise<void> {
  return invoke("download_ai_feature", { id });
}

export function downloadModel(id: string): Promise<void> {
  return invoke("download_model", { id });
}

export function setModelEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke("set_model_enabled", { id, enabled });
}

/**
 * Choose which tier of a tool's models to run.
 *
 * `"auto"` resolves to the best tier that is ready, which is the default and
 * what the app did before the choice existed.
 */
export function setModelTier(tool: string, tier: string): Promise<void> {
  return invoke("set_model_tier", { tool, tier });
}

export function deleteModel(id: string): Promise<void> {
  return invoke("delete_model", { id });
}

export function listTools(): Promise<ToolDescriptor[]> {
  return invoke("list_tools");
}

export function runTool(
  id: string,
  paths: string[],
  params: Record<string, number | string | boolean>,
): Promise<ConversionResult[]> {
  return invoke("run_tool", { id, paths, params });
}

export function panicStop(): Promise<void> {
  return invoke("panic_stop");
}

/** Open the platform file dialog host-side. `null` when cancelled. */
export function pickFiles(kind?: "image" | "audio" | "pdf"): Promise<string[] | null> {
  return invoke("pick_files", { kind: kind ?? null });
}

/**
 * Render a preview of one file, as a PNG data URI.
 *
 * `page` is 1-based and only meaningful for paged documents. `maxPx` caps the
 * longest edge so a 48 Mpx photo does not cross the IPC boundary whole.
 *
 * `ops` are the tool ids applied so far. The render shows **the pending
 * result**, not the source — previewing an edit by describing it and letting
 * the UI mime the effect would make the preview a second implementation of the
 * engine, free to disagree with it.
 */
export function previewFile(
  path: string,
  page = 1,
  maxPx = 1024,
  ops: string[] = [],
): Promise<FilePreview> {
  return invoke("preview_file", { path, page, maxPx, ops });
}

/**
 * Save plain text through the host's save dialog.
 *
 * Resolves to the chosen path, or `null` when the dialog was dismissed —
 * cancelling is a normal outcome and shows nothing.
 */
export function saveTextFile(suggestedName: string, text: string): Promise<string | null> {
  return invoke("save_text_file", { suggestedName, text });
}

/** Subscribe to model download progress. Returns the unlisten function. */
export function onModelProgress(handler: (e: ModelProgress) => void): Promise<UnlistenFn> {
  return listen<ModelProgress>("model-download-progress", (event) => handler(event.payload));
}

/** Subscribe to per-file progress. Returns the unlisten function. */
export function onProgress(handler: (e: ProgressEvent) => void): Promise<UnlistenFn> {
  return listen<ProgressEvent>("conversion-progress", (event) => handler(event.payload));
}

/** Work completed inside a tool's current file. */
export function onToolProgress(handler: (e: { index: number; total: number; fraction: number }) => void): Promise<UnlistenFn> {
  return listen("tool-progress", (event) => handler(event.payload as { index: number; total: number; fraction: number }));
}

/**
 * Render an `Err(String)` for display.
 *
 * The frontend **never parses an error string for control flow** — every
 * message the backend returns has already been through `DisplayName`, and the
 * only correct thing to do with one is show it.
 */
export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
}
