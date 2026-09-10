<!--
  A tool workspace: the file on the left, what you can do to it on the right.

  One component for every media kind rather than three, because the frame is
  identical — stage, command rail, destination, save — and only the stage's
  contents differ. A picture editor, a PDF editor and an audio editor that
  disagree about where the Save button lives are three things to learn.

  **The stage renders the app's own preview of the file, not the file.** In the
  browser harness there is no real input, so the stage draws a representative
  document; in the desktop build the same slots take the preview the backend
  hands back. Either way nothing here decodes anything: the webview is outside
  the trust boundary, and a decoder in it would be the whole boundary gone.
-->
<script lang="ts">
  import { onMount, untrack } from "svelte";
  import {
    listTools,
    previewFile,
    pickFiles,
    listAiFeatures,
    downloadAiFeature,
    setModelTier,
    listSignatures,
    addSignature,
    deleteSignature,
    probeFile,
    audioPeaks,
    readTextOutput,
    revealOutput,
    onProgress,
    onToolProgress,
    errorText,
    type ConversionResult,
    type FilePreview,
    type ProbeResult,
    type AiFeature,
    type Signature,
    type ToolDescriptor,
  } from "../lib/ipc";
  import TextEditor from "./TextEditor.svelte";
  import DropOverlay from "./DropOverlay.svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import FileRow from "./FileRow.svelte";
  import Dropdown from "./Dropdown.svelte";
  import { drop } from "../lib/drop.svelte";
  import SignatureCapture from "./SignatureCapture.svelte";
  import ImageWorkflowControls from "./ImageWorkflowControls.svelte";
  import WavePlayer from "./WavePlayer.svelte";
  import AudioResults from "./AudioResults.svelte";
  import PdfViewer from "./PdfViewer.svelte";
  import OutputReview from "./OutputReview.svelte";
  import ColourPalette from "./ColourPalette.svelte";
  import { EditHistory } from "../lib/stores/edit-history.svelte";
  import { job } from "../lib/stores/job.svelte";
  import BatchFileList from "./BatchFileList.svelte";
  import { undo } from "../lib/stores/undo.svelte";

  let {
    tool,
    paths = [],
    destination = "downloads",
    onclose,
    onrun,
  }: {
    tool: ToolDescriptor;
    paths?: string[];
    destination?: string;
    onclose: () => void;
    onrun: (
      id: string,
      paths: string[],
      params: Record<string, number | string | boolean>,
    ) => Promise<ConversionResult[]>;
  } = $props();

  /** Which stage to draw. Derived from the tool's own category. */
  const kind = $derived(
    tool.category.toLowerCase() === "pdf"
      ? "pdf"
      : tool.category.toLowerCase() === "audio"
        ? "audio"
        : tool.category.toLowerCase() === "video"
          ? "audio"
          : "image",
  );

  /**
   * Tools whose parameters the workspace supplies rather than rendering as
   * fields — see the settings panels in the rail.
   *
   * The registry still declares them, because that is the contract with
   * `run_tool` and what a CLI caller has to pass. This set is about who fills
   * them in.
   */

  /**
   * The PDF tools `pdf-compose` actually performs.
   *
   * Merge, reorder and sign are stages of one document and genuinely compose:
   * three worker calls would write three files and three receipts describing
   * documents that no longer exist by the end. Everything else is its own
   * operation with its own parameters, and sending it through compose is
   * sending a request compose has no field for.
   */
  let composeFormat = $state("png");
  const COMPOSES = new Set(["pdf-merge", "pdf-reorder", "pdf-stamp"]);

  /**
   * THE MODEL TIERS THIS TOOL HAS, and which one a run would pick.
   *
   * Background removal has three, and until now the only place to see or
   * change that was the Settings screen -- so the choice between a 4 MB model
   * that takes a second and a 224 MB one that takes a minute lived two screens
   * away from the picture it applies to.
   *
   * It matters more now than it did: the two outer tiers were taken off the
   * first-run screen, because three rows with the same title is not a choice
   * and 224 MB is not a tick box. This is where they went. A tier that is not
   * downloaded is offered with its size, and picking it fetches it.
   *
   * `active` is what a run would ACTUALLY use, not the stored preference --
   * the preference may be `auto`, and `auto` resolves to the best tier that is
   * ready. Showing the preference would mean a selector that reads "auto"
   * while the tool quietly uses something specific.
   */
  let tiers = $state<AiFeature[]>([]);

  let tiersFor = "";
  let reviewing = $state(false);
  let reviewingInput = $state<string | null>(null);
  let tierBusy = $state<string | null>(null);

  async function loadTiers() {
    const id = current.id;
    if (tiersFor !== id) { tiers = []; tiersFor = id; }
    if (id === "image-upscale" || id.endsWith("-compress")) return;
    try {
      const all = await listAiFeatures();
      if (current.id === id) tiers = all.filter((f) => f.tool === id && f.usable);
    } catch {
      // A tool with no tiers renders no selector, and so does a tool whose
      // tiers could not be read. Neither is worth an error over a picture.
      // Retain the current choices if a refresh fails.
    }
  }

  $effect(() => {
    // Keyed on the tool, so switching in the rail re-asks.
    const id = current.id;
    void id;
    untrack(() => void loadTiers());
  });

  /** Pick a tier, fetching it first if this machine does not have it. */
  async function chooseTier(f: AiFeature) {
    if (tierBusy !== null) return;
    const id = current.id;
    tierBusy = f.id;
    error = null;
    try {
      if (!f.downloaded) await downloadAiFeature(f.id);
      await setModelTier(id, f.tier);
      saved = null; savedOutput = null; runResults = [];
      if (current.id !== id) return;
      await loadTiers();
      await loadPreview();
    } catch (e) {
      error = errorText(e);
    } finally {
      tierBusy = null;
    }
  }

  function tierLabel(tier: string): string {
    return tier === "fast" || tier === "small" ? "Small" : tier === "standard" || tier === "better" ? "Better" : "Best";
  }

  function tierSize(bytes: number): string {
    const m = bytes / 1_048_576;
    return m >= 100 ? `${Math.round(m)} MB` : `${m.toFixed(1)} MB`;
  }

  /** Sibling tools in the same category — the command rail. */
  let siblings = $state<ToolDescriptor[]>([]);
  /** Null until the user picks a sibling; `current` falls back to the prop.
   *  Deriving rather than seeding `$state` from a prop means reopening the
   *  workspace on a different tool actually changes what is selected. */
  let picked = $state<ToolDescriptor | null>(null);
  const current = $derived(picked ?? tool);
  let error = $state<string | null>(null);

  /**
   * The workspace's own error clears itself after five seconds.
   *
   * The shell's banner has done this since it was written -- three seconds,
   * every source cleared -- and this one never did. So a refusal here stayed
   * in the footer until something else replaced it, which meant a message
   * about a file the user had already swapped out sat under the picture of
   * its replacement, and the only way to be rid of it was to close the tool.
   *
   * Five and not three: this sits beside Save rather than across the top of
   * the window, and it is read at a glance downward rather than as the thing
   * that just appeared. Nothing is lost either way -- a failed run is on its
   * own row and in its receipt, both of which persist.
   */
  const ERROR_LINGER_MS = 5000;
  $effect(() => {
    if (error === null) return;
    const timer = setTimeout(() => (error = null), ERROR_LINGER_MS);
    return () => clearTimeout(timer);
  });
  let busy = $state(false);

  /**
   * What the running job has finished so far.
   *
   * **The backend has always emitted this and nothing here listened.** Every
   * tool run goes through `run_tool_batch`, which emits a `started` and a
   * `done` per file on `conversion-progress`; the conversion screen subscribes
   * and the workspace did not. So a five-minute transcription set `busy`,
   * greyed the controls, and then showed nothing at all until it was over --
   * which is indistinguishable from a hung window, and was reported as one.
   */
  let progress = $state<{ done: number; total: number } | null>(null);

  /**
   * Whether the ring can show a real fraction.
   *
   * A batch reports one event per file, so the fraction is measured. A SINGLE
   * file reports 0 then 1, and nothing in between: whisper does not report
   * intra-file progress, so there is no honest percentage to draw. The ring
   * sweeps instead of filling -- it says work is happening, which is true,
   * rather than a number, which would not be.
   */

  const progressFraction = $derived(
    progress && progress.total > 0 ? Math.min(progress.done / progress.total, 1) : 0,
  );
  let saved = $state<string | null>(null);
  /** The file the last save wrote, so "Show in folder" has something to open. */
  let savedOutput = $state<string | null>(null);

  /**
   * WHAT THE RUN ACTUALLY DID, in the units the tool is about.
   *
   * "Saved to Downloads" is where, not what. For compression the whole
   * question is how much smaller, and the app knew -- `ConversionResult`
   * carries `inputBytes` and `outputBytes` and nothing read them. For
   * enlargement it is how much bigger, in pixels.
   *
   * Summed across the batch, because a batch is one answer: forty photographs
   * went from 180 MB to 42 MB. Per-file numbers for forty files is a table
   * nobody asked for.
   */
  let runResults = $state<ConversionResult[]>([]);
  let lastRun = $state<{ inBytes: number; outBytes: number } | null>(null);
  let estimates = $state<Record<string, string>>({});
  let estimateBusy = $state(false);
  $effect(() => {
    const id = current.id;
    const quality = values.quality || current.params.find(p=>p.id==="quality")?.default || "65";
    const files = [...runPaths];
    if (combineMode || !id.endsWith("-compress") || files.length === 0) { estimates = {}; estimateBusy = false; return; }
    let stale = false;
    estimates = {};
    estimateBusy = true;
    const timer = setTimeout(async () => {
      estimateBusy = true;
      await renderLanes(files, async (path) => {
        try {
          const result = await previewFile(path, 1, 500, [`compress:${quality}`]);
          if (stale || result.originalBytes == null || result.compressedBytes == null) return;
          const percent = Math.round((1-result.compressedBytes / Math.max(1,result.originalBytes))*100);
          estimates = { ...estimates, [path]: `${fileSize(result.originalBytes)} → ${fileSize(result.compressedBytes)} · ${percent}% smaller` };
        } catch (e) { if (!stale) estimates = { ...estimates, [path]: errorText(e) }; }
      }, () => stale);
      if (!stale) estimateBusy = false;
    }, 350);
    return () => { stale = true; clearTimeout(timer); };
  });
  let resultsFor = $state<Record<string, string>>({});

  /** Bytes, rounded the way a file manager rounds them. */
  function fileSize(bytes: number): string {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${bytes} bytes`;
  }

  /**
   * The one-line outcome, or nothing.
   *
   * Nothing is the common case and the right default: most tools have no
   * number worth printing, and a row that is sometimes there and sometimes an
   * empty string is a row that moves the footer.
   *
   * The enlarged dimensions are COMPUTED, from the preview's size and the
   * factor the tool ran with, rather than measured off the output. Reading the
   * real file would mean rendering it again for one label. The factor is not a
   * guess -- `run_upscale` refuses anything but the factor it has a model for,
   * so if the run succeeded, that is the factor it used.
   */
  /** Take the outputs back, and put the stage back to where it was. */
  async function undoSave() {
    await undo.invoke();
    if (!undo.done) return;
    resultsFor = {};
    transcripts = {};
    saved = null;
    savedOutput = null;
    runResults = [];
    reviewing = false;
    reviewingInput = null;
  }
  let chosenDest = $state<string | null>(null);
  const dest = $derived(chosenDest ?? destination);

  /** Which operations have been previewed, so the stage can show their effect. */
  let applied = $state<Set<string>>(new Set());
  /**
   * Tools whose output is TEXT rather than a picture or a recording.
   *
   * Two of them, and they behave identically from here down: run the tool, read
   * the `.txt` it wrote, show it with Copy and Save. Keeping the set in one
   * place is what stopped the OCR half being a second copy of the
   * transcription half with a different heading.
   */
  const TEXT_TOOLS: Record<string, { heading: string; verb: string }> = {
    "audio-transcribe": { heading: "Transcript", verb: "transcribe this recording" },
    // OCR reached the menu again on 2026-09-05, and this is the entry it
    // needed: one line, because the map was kept as a map while the tool was
    // withdrawn rather than collapsing back into transcription-specific code.
    "pdf-text": { heading: "Text", verb: "extract text from this PDF" },
    "image-ocr": { heading: "Text", verb: "read the text in this image" },
  };

  /** The text tool currently selected, or null. */
  const textTool = $derived(
    Object.keys(TEXT_TOOLS).find((id) => applied.has(id)) ?? null,
  );

  /**
   * The text panel is open exactly when a text tool is the selected one.
   *
   * It used to be its own `$state`, set when transcribe was picked and never
   * cleared when something else was — so selecting "remove background noise"
   * left a Transcript heading sitting under an audio operation that produces
   * no text. Derived, it cannot disagree with the rail.
   */
  const transcriptOpen = $derived(textTool !== null && kind !== "audio");
  let pdfPage = $state(1);

  // --- audio facts ----------------------------------------------------------
  //
  // The audio stage used to draw a waveform from a seeded random number
  // generator and print "3:14 · 48 kHz · stereo" underneath it, for every file
  // regardless of what it was. Both are now measured: the peaks come from
  // `oc-audio` decoding the recording in the worker, and the duration, rate and
  // channel count come from the same probe the rest of the app uses.

  /**
   * Peaks and probe results, PER FILE.
   *
   * Audio used to be one file: one `peaks`, one `probed`, one big centred
   * waveform for `activePaths[0]`, and any other file the user had chosen was
   * invisible — converted on Save, with nothing on screen admitting it
   * existed. Both are maps keyed by path now, so a list of recordings shows a
   * list of recordings.
   */
  let peaksFor = $state<Record<string, number[]>>({});
  let probedFor = $state<Record<string, ProbeResult>>({});
  /** Guards a stale read landing on a file the user has since replaced. */
  let audioToken = 0;

  /** How many recordings are measured on open. */
  const AUDIO_MEASURE_LIMIT = 40;

  /** Read every open recording: its facts, then its shape. */
  async function loadAudio() {
    if (kind !== "audio" || activePaths.length === 0) return;
    const token = ++audioToken;
    const wanted = activePaths.slice(0, AUDIO_MEASURE_LIMIT);

    // Sequential, not `Promise.all`. Each peak read decodes a whole recording
    // in the confined worker; forty at once is forty workers, and the list
    // filling in from the top is a better answer than the window locking while
    // they all finish.
    for (const path of wanted) {
      if (token !== audioToken) return;
      if (probedFor[path] === undefined) {
        try {
          const p = await probeFile(path);
          if (token !== audioToken) return;
          probedFor = { ...probedFor, [path]: p };
        } catch (e) {
          // A probe that fails is not fatal to the workspace — the tools still
          // run. That row's facts line simply has nothing to say.
          error = errorText(e);
        }
      }
      if (peaksFor[path] === undefined) {
        try {
          const measured = await audioPeaks(path, 96);
          if (token !== audioToken) return;
          peaksFor = { ...peaksFor, [path]: measured };
        } catch {
          // A recording this build cannot decode leaves the resting line,
          // which claims nothing, rather than a shape invented to fill it.
        }
      }
    }
  }

  /** The facts line for one recording: duration, rate, channels. */
  function factsFor(path: string): string | null {
    const p = probedFor[path]?.properties;
    if (!p || p.kind !== "audio") return null;
    const bits: string[] = [];
    if (p.durationMs !== null && p.durationMs > 0) {
      const total = Math.round(p.durationMs / 1000);
      bits.push(`${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`);
    }
    if (p.sampleRate !== null && p.sampleRate > 0) {
      bits.push(`${(p.sampleRate / 1000).toFixed(p.sampleRate % 1000 === 0 ? 0 : 1)} kHz`);
    }
    if (p.channels !== null && p.channels > 0) {
      bits.push(p.channels === 1 ? "mono" : p.channels === 2 ? "stereo" : `${p.channels} channels`);
    }
    return bits.length > 0 ? bits.join(" · ") : null;
  }

  /** The rendered page or image, from the backend. Null until it arrives. */
  /** Probe the open file so its size can be shown, once per path. */
  async function probeSource() {
    await renderLanes(activePaths.filter((path) => !probedFor[path]), async (path) => {
      const p = await probeFile(path);
      if (!activePaths.includes(path)) return;
      probedFor = { ...probedFor, [path]: p };
    }, () => false);
  }

  /** Bytes, rounded the way a file manager rounds them. */
  function sizeOf(bytes: number): string {
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
    if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
    if (bytes >= 1024) return `${Math.round(bytes / 1024)} KB`;
    return `${bytes} bytes`;
  }

  /** Whether the current tool is about the file's size rather than its pixels. */
  const aboutSize = $derived(current.id === "image-compress" || current.id === "pdf-compress");

  /**
   * `1.9 MB` before a run, `1.9 MB -> 540 KB (72% smaller)` after.
   *
   * This is the caption UNDER THE DOCUMENT, replacing the pixel dimensions for
   * the compression tools -- the plan's words were "show old size -> new size
   * instead of pixel dimensions", and the footer's after-save line was only
   * half of it: it appears after a save and is gone the moment the next file
   * is opened, so the number a person is trying to change was never on screen
   * while they were choosing how to change it.
   */
  const sizeLine = $derived.by(() => {
    const before = probedFor[activePaths[0] ?? ""]?.inputBytes ?? 0;
    if (saved && lastRun && lastRun.inBytes > 0 && lastRun.outBytes > 0) {
      const pct = Math.round((1 - lastRun.outBytes / lastRun.inBytes) * 100);
      const both = `${sizeOf(lastRun.inBytes)} → ${sizeOf(lastRun.outBytes)}`;
      return pct > 0 ? `${both} (${pct}% smaller)` : both;
    }
    return estimates[sourcePath] ?? (estimateBusy ? `${before > 0 ? sizeOf(before) + " → " : ""}Checking compressed size…` : before > 0 ? sizeOf(before) : null);
  });

  let shot = $state<FilePreview | null>(null);
  let shotError = $state<string | null>(null);
  let loadingShot = $state(false);
  let previewGeneration = 0;
  const pdfPages = $derived(shot?.pageCount ?? 1);

  /**
   * Ask the backend to render the current state.
   *
   * Audio has no raster preview — the waveform is drawn from the samples, not
   * from a picture of them — so it is skipped rather than requested and thrown
   * away.
   */
  async function loadPreview() {
    const generation = ++previewGeneration;
    const path = sourcePath;
    const page = pdfPage;
    // Nothing is open, so there is nothing to render. Asking anyway is what
    // produced "IMG_4821.heic is not there any more" over an empty stage.
    if (kind === "audio") {
      // Audio has no raster preview, but it does have a shape and facts.
      void loadAudio();
      return;
    }
    if (activePaths.length === 0) return;
    // THE FILE'S SIZE, for the tools whose whole job is changing it.
    //
    // The workspace knew the preview's PIXEL dimensions and nothing about the
    // bytes, so Compress -- the one tool a person opens to make a file smaller
    // -- could only report `960 x 660`, which does not change when it works.
    // The probe is cheap, cached per path, and the audio side has been doing
    // exactly this since it was written.
    void probeSource();
    // Batch has no stage to render into, and a preview of the RESULT per file
    // is a model run per file to answer a question the screen does not ask.
    // The list's own thumbnails are a different thing: no operations, one
    // small render each, so the rows show which picture they are.
    if ((kind === "image" && batchMode) || current.id === "pdf-compress") {
      return;
    }
    loadingShot = true;
    shotError = null;
    try {
      const rendered = await previewFile(path, page, 1024, previewOps);
      if (generation !== previewGeneration || sourcePath !== path) return;
      shot = rendered;
      // Page cards need one render each; kicked off rather than awaited so the
      // main sheet appears immediately and the strip fills in behind it.
      if (kind === "pdf") void loadThumbs();
      // Signing needs sharp pages, and only signing does.
      if (kind === "pdf" && applied.has("pdf-stamp")) void loadSheets();
      if (arrangingDocuments) void loadMergeThumbs();
      // The untouched original, for the comparison. Fetched alongside rather
      // than kept from an earlier call: `applied` may already have been
      // non-empty the first time this ran (a tool opened from the menu
      // previews itself), so "the last one" is not reliably the original.
      if (kind === "image") {
        const before = previewOps.length === 0 ? shot : await previewFile(path, page, 1024, []);
        if (generation === previewGeneration) shotBefore = before;
      }
    } catch (e) {
      if (generation !== previewGeneration) return;
      shot = null;
      shotError = errorText(e);
    } finally {
      if (generation === previewGeneration) loadingShot = false;
    }
  }

  // --- the image viewport ---------------------------------------------------
  //
  // Zoom, pan, and the comparison. All three are view state: nothing here
  // changes a pixel or reaches the backend. The stage used to be a bare `<img>`
  // scaled to fit, which is fine for looking at a photograph and useless for
  // checking whether background removal cut into someone's hair.

  /** Scale, 1 = one image pixel per CSS pixel. */
  let zoom = $state(1);
  /** Pan offset in CSS pixels, from the centred position. */
  let pan = $state({ x: 0, y: 0 });
  /** True while the image is being dragged. */
  let panning = $state(false);
  let panPointer = $state<number | null>(null);
  let panStart = { x: 0, y: 0, px: 0, py: 0 };
  /** True while the pointer is held on the compare button. */
  let comparing = $state(false);

  /** The original, for the before/after comparison. Null until it arrives. */
  let shotBefore = $state<FilePreview | null>(null);

  const ZOOM_MIN = 0.1;
  const ZOOM_MAX = 8;
  const zoomPercent = $derived(Math.round(zoom * 100));

  /**
   * The canvas element, so a zoom can be anchored at a point on screen.
   *
   * Read on demand rather than observed: it is wanted only during a gesture,
   * and a layout read at that moment is one the browser has already done.
   */
  let canvasEl = $state<HTMLElement | null>(null);
  const canvasBox = $derived.by(() => canvasEl?.getBoundingClientRect() ?? null);

  /** Back to fit: scale 1 (CSS does the fitting) and no offset. */
  function fitView() {
    zoom = 1;
    pan = { x: 0, y: 0 };
  }

  function zoomBy(factor: number) {
    zoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom * factor));
    if (zoom === 1) pan = { x: 0, y: 0 };
  }

  /**
   * Wheel zooms; the modifier is not required.
   *
   * A wheel over an image in a viewer is a zoom, not a scroll — there is
   * nothing under the stage to scroll to. `preventDefault` stops the page
   * moving behind it.
   */
  /**
   * Wheel and trackpad pinch.
   *
   * A two-finger pinch arrives as a wheel event with `ctrlKey` set — that is
   * how every browser reports it, and it is the only way to tell a pinch from
   * a scroll. Both zoom here (there is nothing on this surface to scroll), but
   * a pinch is CONTINUOUS: it sends many small deltas, and answering each with
   * the same fixed 12% step made it lurch. Scaling the factor by the delta is
   * what makes the image track the fingers.
   */
  /**
   * Zoom about a point on screen, keeping what is under it under it.
   *
   * **This is what makes a pinch feel like it moves the picture.** Zooming
   * about the centre magnifies the middle of the stage and leaves whatever you
   * were pointing at drifting off the edge, so reaching a detail meant zooming,
   * then panning, then zooming again. Anchoring at the fingers means the pixel
   * you are pinching on stays put and everything else moves around it -- which
   * is the same gesture every map and image viewer has.
   *
   * The image is drawn as `translate(pan) scale(zoom)` about the canvas centre,
   * so for a screen offset `s` from that centre the fixed-point condition is
   * `pan' = pan + (s - pan) * (1 - factor)`.
   */
  function zoomAt(factor: number, clientX: number, clientY: number) {
    const before = zoom;
    zoomBy(factor);
    // `zoomBy` clamps, so the factor actually applied is not always the one
    // asked for -- at either limit it is 1, and the pan must not move.
    const applied = zoom / before;
    if (applied === 1) return;

    const box = canvasBox;
    if (!box) return;
    const sx = clientX - (box.left + box.width / 2);
    const sy = clientY - (box.top + box.height / 2);
    pan = {
      x: pan.x + (sx - pan.x) * (1 - applied),
      y: pan.y + (sy - pan.y) * (1 - applied),
    };
  }

  /**
   * One zoom step, anchored where the step is FOR.
   *
   * The buttons called `zoomBy`, which magnifies about the canvas centre. On
   * the colour picker that is the wrong anchor: the point you have aimed at
   * drifts outward as the picture grows, so zooming in to check a pixel is the
   * gesture that loses it -- and this is the one tool with no drag-to-pan to
   * recover with, because the primary pointer belongs to sampling.
   *
   * Anchoring on the crosshair keeps the sampled pixel exactly where it is and
   * grows the picture around it. That is what "zoom in to look closer" means,
   * and it is the same fixed-point maths the pinch already uses.
   *
   * Everywhere else the centre is right, and nothing changes.
   */
  function zoomStep(factor: number) {
    const box = picking ? stageImg?.getBoundingClientRect() : null;
    if (box && box.width > 0 && box.height > 0) {
      zoomAt(factor, box.left + cross.x * box.width, box.top + cross.y * box.height);
      return;
    }
    zoomBy(factor);
  }

  function wheelZoom(event: WheelEvent) {
    event.preventDefault();
    if (event.ctrlKey) {
      // Pinch. Fingers apart is a negative deltaY, so this is already the
      // right way round: apart magnifies, together shrinks. The exponent keeps
      // it smooth and symmetric -- pinching back out by the same amount returns
      // to the same zoom.
      //
      // Anchored at the pointer, not the centre: see `zoomAt`.
      zoomAt(Math.exp(-event.deltaY / 120), event.clientX, event.clientY);
      return;
    }
    // TWO FINGERS MOVE THE PICTURE. They used to zoom it.
    //
    // A plain wheel here was a second way to zoom, which left NO way to move a
    // zoomed image while the colour picker is active -- the picker owns the
    // primary pointer, because a click on the image means "read this pixel",
    // so drag-to-pan is not available to it and the image could be magnified
    // and then not navigated. The one tool that most needs to reach a
    // particular pixel was the one that could not.
    //
    // Pinch zooms, two fingers move: the mapping every image viewer and map
    // uses, and unambiguous because a pinch is reported with `ctrlKey` and a
    // scroll is not. The buttons and the keyboard still zoom, so a mouse with
    // one wheel loses nothing.
    pan = { x: pan.x - event.deltaX, y: pan.y - event.deltaY };
  }

  function startPan(event: PointerEvent) {
    // The colour picker owns the PRIMARY pointer while it is active: a click
    // there means "read this pixel", and stealing it for a pan would break the
    // one gesture that tool has. The middle button is not the picker's, so it
    // pans everywhere -- and two fingers do too, in `wheelZoom`.
    const middle = event.button === 1;
    if (!middle && (picking || event.button !== 0)) return;
    panning = true;
    panPointer = event.pointerId;
    panStart = { x: event.clientX, y: event.clientY, px: pan.x, py: pan.y };
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Panning still works; the pointer is simply not pinned.
    }
  }

  function movePan(event: PointerEvent) {
    if (!panning || event.pointerId !== panPointer) return;
    pan = {
      x: panStart.px + (event.clientX - panStart.x),
      y: panStart.py + (event.clientY - panStart.y),
    };
  }

  function endPan(event: PointerEvent) {
    if (event.pointerId !== panPointer) return;
    panning = false;
    panPointer = null;
  }

  /** True while the colour picker is the selected tool. Declared here rather
   *  than beside the rest of the picker because `canCompare` below depends on
   *  it, and Svelte's `$derived` is not hoisted. */
  const picking = $derived(applied.has("image-pick"));

  /** What the stage is showing right now: the original while comparing. */
  const visible = $derived(comparing && shotBefore ? shotBefore : shot);
  /** Whether a comparison is possible — something has been applied, and we
   *  have the original to compare it against. */
  const canCompare = $derived(
    // NOT WHILE PICKING. The colour picker changes no pixels, so there is no
    // "after" to compare an "original" against; the button sat there offering
    // to toggle between two identical images.
    !picking && applied.size > 0 && shotBefore !== null && shot !== null,
  );

  const params = $derived(current.id === "image-upscale" ? [] : current.params.filter(p => !(current.id === "pdf-protect" && values.direction === "remove" && p.id === "owner_password")));
  let values = $state<Record<string, string>>({});

  $effect(() => {
    // Changing the requested output makes the previous Save result stale.
    JSON.stringify(values);
    void composeFormat;
    void splitMode; void splitEvery; JSON.stringify(splitNames);
    JSON.stringify(pageTurns);
    JSON.stringify(pageCrops);
    order.join(",");
    activePaths.join("\n");
    [...applied].join(",");
    [...excludedPaths].join("\n");
    untrack(() => { saved = null; lastRun = null; resultsFor = {}; savedOutput = null; runResults = []; reviewing = false; });
  });

  $effect(() => {
    // Start at the tool's own default so the control shows what will actually
    // happen if the user changes nothing.
    values = Object.fromEntries(current.params.map((p) => [p.id, p.default ?? ""]));
  });

  onMount(() => {
    if (kind === "pdf") void refreshSignatures();
    void listTools()
      .then((all) => {
        siblings = all.filter((t) => t.category === tool.category);
      })
      .catch((e) => {
        error = errorText(e);
      });

    // Picking a tool from the menu is a request for that operation, so preview
    // it straight away rather than making the user ask twice. Tools with
    // required parameters wait — there is nothing to preview until they are
    // filled in, and guessing a trim range would be worse than an empty stage.
    //
    // UNLESS THE WORKSPACE FILLS THEM IN ITSELF. `pdf-stamp` declares `image`
    // and `page` as required, and they still are — on the wire. They are no
    // longer TYPED, though: the signature panel supplies both, and the page
    // comes from whichever page was clicked. Judged by the old rule, choosing
    // "Place image on page" from the menu opened a workspace with no signature
    // panel and no way to reach one, because the tool it was opened for was
    // never selected.
    if (tool.available) void preview(tool);
    else void loadPreview();
  });

  /** The file the stage is showing. Falls back to a sample when nothing was
   *  carried in, so a tool opened from the menu has something to work on. */
  /** Picked inside the workspace; takes precedence over the carried-in drop. */
  let chosenPaths = $state<string[] | null>(null);
  const activePaths = $derived(chosenPaths ?? paths);
  let excludedPaths = $state<Set<string>>(new Set());
  /** True when the stage is showing a stand-in rather than a real file. */
  const noFile = $derived(activePaths.length === 0);
  /** "an image file", "a PDF file" — the article has to match the word. */
  const article = $derived(/^[aeiou]/i.test(tool.category) ? "an" : "a");

  // No stand-in name. An invented "IMG_4821.heic" in the header is a claim
  // about the user's file, and the empty state already says nothing is open.
  const sourcePath = $derived(activePaths[0] ?? "");

  /** The last path component, whichever separator the platform uses.
   *
   * Windows paths arrive with backslashes and the harness uses forward ones;
   * taking only one of them turned `C:/Users/you/report.pdf` into the whole
   * path as a "filename". */
  function baseName(path: string): string {
    return path.slice(Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/")) + 1);
  }

  const fileName = $derived(baseName(sourcePath));

  /**
   * Tools that cannot share a Save with anything else.
   *
   * Not a preference — a fact about what each one produces.
   *
   * - **Every audio tool.** "Transcribe" produces text and "remove background
   *   noise" produces audio; one Save cannot write both, so lighting them
   *   together promises something the button underneath cannot do.
   * - **`image-pick`.** It reports a colour and writes no file at all. Lit
   *   beside "upscale", Save means one of two unrelated things.
   * - **`pdf-stamp`.** Placing an image is positioned against a specific page
   *   of a specific document; merging or reordering underneath it moves that
   *   page somewhere else, so the coordinates the user chose stop meaning what
   *   they meant when they chose them.
   *
   * Everything else composes. Invert then greyscale is a coherent request, and
   * merge + reorder is the combined PDF flow this app was built around.
   */
  const SOLO_TOOLS = new Set(["image-pick", "pdf-stamp"]);

  /**
   * Whether `id` refuses to share a Save.
   *
   * **IMAGES ARE ONE AT A TIME NOW.** They composed: two lit buttons meant one
   * job through `image-compose`, which worked and which nothing on screen
   * said. Two tools lit, one Save, and no way to tell whether Save meant both
   * of them or the one clicked last -- and with each tool's settings now
   * expanding under it in the rail, two lit tools is also two panels open for
   * an operation the user did not ask twice for.
   *
   * The capability did not go anywhere: it is `image-compose`, in the rail as
   * "Combine edits", with the operations as tick boxes. Composing is now
   * something you ask for rather than something that happens.
   *
   * PDF still composes, deliberately: merge plus reorder plus a signature is
   * the combined flow this app was built around, and those are stages of one
   * document rather than alternative things to do to a picture.
   */
  function solo(id: string): boolean {
    // COMBINING SUSPENDS THE ONE-AT-A-TIME RULE, for the tools it covers.
    // That is the whole of what the mode does to the rail.
    if (combineMode && COMBINABLE.has(id)) return false;
    return kind === "audio" || kind === "image" || !COMPOSES.has(id) || SOLO_TOOLS.has(id);
  }

  /** Preview an operation. Nothing is written until Save. */
  async function preview(t: ToolDescriptor) {
    if (t.id !== current.id) resetForTool(t);
    picked = t;
    if (!t.available) return;
    busy = true;
    error = null;
    // NO ARTIFICIAL DELAY.
    //
    // This slept 420ms before doing anything, with a comment saying the delay
    // "stands in" for a backend round trip — written when the stage was a
    // mock. `loadPreview()` below IS the round trip now, and it sets `busy`
    // for exactly as long as it takes. The sleep was 420ms added to every
    // single tool selection, for nothing.
    const on = applied.has(t.id);
    if (on) {
      // Selecting the lit one turns it off, and "nothing selected" stays a
      // reachable state on every category.
      const next = new Set(applied);
      next.delete(t.id);
      applied = next;
    } else if (solo(t.id) || t.id === "pdf-merge") {
      // A solo tool takes the screen to itself.
      applied = new Set([t.id]);
    } else {
      // Adding a composing tool evicts any solo tool that was lit, rather than
      // refusing the click: the user asked for the new one, and saying so by
      // turning the old one off is clearer than doing nothing.
      const next = new Set([...applied].filter((id) => !solo(id)));
      next.add(t.id);
      applied = next;
    }
    busy = false;
    await loadPreview();
  }

  /**
   * Forget what belonged to the tool being left behind.
   *
   * **Switching tools inside the workspace never remounted anything.** The
   * shell keys the component on `activeTool.id`, which is the tool it was
   * OPENED with — clicking a different tool in the rail sets `picked` and
   * changes nothing else, so a signature placed on page 4, a rebuilt page
   * order, a transcript and a merge view all survived into a tool that knew
   * nothing about them. That is the "UI is bugged after switching" report.
   *
   * Remounting on every rail click would fix it and cost more than it saves:
   * it would also throw away the open file and any page order the user had
   * built, which is a worse bug than the one being fixed. So this clears what
   * genuinely does not survive the move, and keeps what does — the file, and a
   * page order when both tools are PDF tools that operate on one.
   */
  function resetForTool(next: ToolDescriptor) {
    tiers = [];
    tiersFor = "";
    reviewing = false;
    reviewingInput = null;
    runResults = [];
    modifying = null;
    saved = null;
    lastRun = null;
    error = null;
    transcriptCopied = null;

    // Batch belongs to the tools that can be batched. Carrying it into the
    // colour picker would leave a list of files and no way to look at any of
    // them, on a tool whose whole job is looking.
    if (!BATCHABLE.has(next.id)) batchMode = false;

    // A transcript belongs to the run that produced it. Leaving it up under a
    // different operation invites Copy on text the current tool did not make.
    // (Per file: the audio workspace is a list, and each row holds its own.)
    if (!(next.id in TEXT_TOOLS)) transcripts = {};

    // A placement is coordinates on a page of the open document. Every tool
    // except the one that placed it can move that page.
    if (next.id !== "pdf-stamp") {
      placedSignatures = [];
      placement = null;
      placing = false;
    }

    // The merged two-stage view is `pdf-merge`'s own screen.
    if (next.id !== "pdf-merge") {
      merged = false;
      documentPreview = null;
      documentPreviewToken += 1;
    }

    // Reorder's input mode and its half-typed field are its own.
    if (next.id !== "pdf-reorder") {
      numbering = "";
      orderError = null;
    }
  }

  async function turnTo(n: number) {
    pdfPage = Math.min(Math.max(1, n), pdfPages);
    await loadPreview();
  }

  /**
   * What Save actually runs.
   *
   * FOR A PDF, ONE CALL THAT DOES EVERYTHING SELECTED — not one call per lit
   * tool. Merging, reordering and signing are three tools in the rail because
   * they are three things to think about, but a person who has done all three
   * wants ONE document, not three files in a chain where the first two are
   * scaffolding. `pdf-compose` takes the lot and comes back with one output
   * and one receipt.
   *
   * The parameters describe what was selected; the worker skips the stages it
   * was given nothing for. So merge alone, reorder alone and sign alone all go
   * down this same path and behave exactly as they did.
   */
  function saveJob(): { id: string; paths: string[]; params: Record<string, string> } {
    const combining = [...applied].filter((id) => COMBINABLE.has(id));
    if (combineMode && combining.length > 0) {
      // SEVERAL OPERATIONS, ONE JOB, ONE RECEIPT.
      //
      // The set used to be read off the rail, because the rail could hold more
      // than one lit tool. It is one-at-a-time now, so the set comes from the
      // tick boxes under this tool instead -- same call, same fixed order in
      // `tools.rs`, asked for out loud.
      //
      // `image-compose` declares `multi_input: false`, so the host runs it
      // once per path. In batch that is exactly right: every picture gets the
      // same operations, its own file and its own receipt. Single-file mode
      // sends one, because that is what is on the stage.
      return {
        id: "image-compose",
        paths: batchMode ? runPaths : activePaths.slice(0, 1),
        params: { ops: combining.join(","), format: composeFormat },
      };
    }

    if (kind !== "pdf") {
      return {
        id: current.id,
        paths: kind === "image" && !batchMode ? activePaths.slice(0, 1) : runPaths,
        params: { ...(values as Record<string, string>), ...stampParams() },
      };
    }

    // ONLY THREE PDF TOOLS COMPOSE. Everything else runs as itself.
    //
    // **This was the bug that made most of the PDF rail do nothing.** Every
    // PDF save went out as `pdf-compose` regardless of which tool was lit, and
    // compose understands exactly three things: merge, reorder (with the page
    // turns and crops that ride along with it) and a signature. So Split,
    // Password and Compress each sent compose a request with none of its
    // parameters set -- and compose, asked to do nothing, wrote nothing or
    // refused.
    //
    // It was survivable while the rail held only the three tools compose
    // knows. Advertising Split and Password put two more through the same
    // funnel, and Compress had been going through it the whole time.
    //
    // `every_advertised_tool_saves.rs` is the gate that would have caught it:
    // it runs each advertised tool the way the interface asks for it and looks
    // at the disk afterwards.
    if (!COMPOSES.has(current.id)) {
      return {
        id: current.id,
        paths: current.id === "pdf-compress" ? runPaths : activePaths.slice(0, 1),
        params: current.id === "pdf-split" ? { boundaries: splitBoundaries, output_names: JSON.stringify(splitParts.map((_,i)=>splitNames[i]??"")) } : { ...(values as Record<string, string>) },
      };
    }

    // Documents in the order the board has them — which is the document order
    // for a merge, and a single document otherwise.
    const docs =
      applied.has("pdf-merge") && activePaths.length > 1
        ? (merged ? mergedDocumentOrder : order)
            .map((slot) => activePaths[slot])
            .filter((x): x is string => x !== undefined)
        : activePaths.slice(0, 1);

    const params: Record<string, string> = {};
    // A page order is only sent when reordering is actually selected AND the
    // arrangement differs from the document. Sending `1,2,3` for a document
    // nobody rearranged would make every save a reorder in the receipt.
    if (applied.has("pdf-reorder") && reordered) {
      params.order = order.map((i) => i + 1).join(",");
    }
    // The page-level edits ride the same compose as the order and the
    // signature: one worker call, one output, one receipt. Sent only when
    // there is something to send, so a document nobody turned carries no
    // `turns` key and its receipt does not mention turning.
    if (applied.has("pdf-reorder")) {
      if (turnsParam) params.turns = turnsParam;
      if (cropsParam) params.crops = cropsParam;
    }
    if (applied.has("pdf-stamp")) {
      const stamps = [...placedSignatures.map(s => s.params), ...(placement && signature ? [stampParams()] : [])];
      if (stamps.length) params.stamps = JSON.stringify(stamps);
    }
    return { id: "pdf-compose", paths: docs, params };
  }

  async function save() {
    busy = true;
    error = null;
    // Cleared here rather than on completion: a ring left showing the last
    // run's total would start the next one at 3 of 3.
    progress = null;
    runResults = [];
    try {
      const job = saveJob();
      const results = await onrun(job.id, job.paths, { ...job.params, destination: dest });
      resultsFor = Object.fromEntries(results.map((r, i) => [job.paths[i], r.success
        ? `${fileSize(r.inputBytes)} → ${fileSize(r.outputBytes)}${r.inputBytes === r.outputBytes ? " · unchanged" : ""}`
        : r.errorMessage ?? "Could not compress this file"]));
      runResults = results;
      const ok = results.some((r) => r.success);
      saved = ok ? dest : null;
      const done = results.filter((r) => r.success);
      lastRun = done.length
        ? {
            inBytes: done.reduce((n, r) => n + r.inputBytes, 0),
            outBytes: done.reduce((n, r) => n + r.outputBytes, 0),
          }
        : null;
      // The first output is what "Show in folder" opens. A batch reveals its
      // first file, which puts the folder on screen with the rest beside it.
      savedOutput = results.find((r) => r.success && r.outputPath)?.outputPath ?? null;

      // A TOOL WHOSE OUTPUT IS TEXT HAS TO SHOW IT.
      //
      // Transcription writes a `.txt` and always did; the panel below it was
      // never given the path, so it sat at "0 segments" over a Copy button
      // that copied an empty string. The run is the expensive part and it has
      // just happened — reading the result back costs one small file read.
      if (ok && current.id in TEXT_TOOLS) {
        // EVERY file that produced text, not the first one. The backend runs a
        // single-input tool once per path and returns the results in that
        // order, so a run over five recordings comes back with five outputs —
        // and reading only `results.find(...)` showed one transcript and threw
        // four away without saying so.
        const next: Record<string, string> = { ...transcripts };
        for (const [index, result] of results.entries()) {
          const path = job.paths[index];
          if (path === undefined || !result.success || !result.outputPath || !/\.(txt|vtt|srt)$/i.test(result.outputPath)) continue;
          try {
            next[path] = (await readTextOutput(result.outputPath)).trim();
          } catch (e) {
            // The file is on disk either way; only that row is empty.
            error = errorText(e);
          }
        }
        transcripts = next;
      }
      // A RUN THAT RETURNED FALSE IS A FAILURE, and it used to be indicated by
      // the footer going back to "Nothing is written until you save." — which
      // is what it says before you press anything. The one outcome that has to
      // be unmistakable was the one that looked like nothing had happened.
      error = ok ? null : "That did not produce a file. Nothing was written.";

      // The undo window is the backend's, and offering it here is what lets
      // the toast stay off a workspace: one Undo, in the place the Save was.
      if (ok) undo.offer(results.filter((r) => r.success).length);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
      progress = null;
    }
  }

  /**
   * The stamp's parameters, converted from screen space to PDF user space.
   *
   * TWO CONVERSIONS, AND BOTH MATTER.
   *
   * The Y AXIS IS FLIPPED. The webview measures down from the top; a PDF
   * measures up from the bottom. A signature dropped a fifth of the way down
   * the page has to be sent as four-fifths of the way up it, or every
   * signature lands mirrored across the middle of the page — which looks
   * plausible enough on a symmetrical layout to ship unnoticed.
   *
   * And the ANCHOR MOVES. On screen the placement is its centre, because that
   * is where the cursor was. `stamp()` takes the lower-left corner. So half
   * the width comes off X, and half the height off Y — and the height is not
   * ours to choose: the worker derives it from the image's aspect ratio, so we
   * derive it the same way from the same image.
   *
   * Page size comes from the render when the renderer reported it, and falls
   * back to US Letter when it did not. A page whose size is unknown is placed
   * on the assumption every PDF viewer makes about an unmarked page.
   */
  function stampParams(): Record<string, string> {
    if (current.id !== "pdf-stamp" || !placement || !signature) return {};
    const shape = pageDimensions[placement.page];
    const pageW = shape?.width ?? shot?.pageWidthPt ?? 612;
    const pageH = shape?.height ?? shot?.pageHeightPt ?? 792;
    const widthPt = placement.width * pageW;
    const heightPt = widthPt * signatureAspect;
    return {
      image: signature.path,
      page: String(placement.page),
      x: (placement.x * pageW - widthPt / 2).toFixed(2),
      y: ((1 - placement.y) * pageH - heightPt / 2).toFixed(2),
      width: widthPt.toFixed(2),
    };
  }

  /**
   * Height ÷ width of the selected signature, measured from the image itself.
   *
   * The worker derives the stamp's height from the image's aspect ratio, so
   * the placement maths has to derive it the same way from the same image —
   * otherwise the box drawn on screen and the box drawn in the PDF are
   * different shapes, and a signature that looked centred lands low.
   */
  let signatureAspect = $state(0.35);

  /**
   * Measure the selected signature's proportions.
   *
   * This used to hang off an `onload` on each row's thumbnail, with a guard so
   * only the selected row's measurement won. The list is a dropdown now and
   * there are no thumbnails to load, so it decodes the data URI directly —
   * which is also less fragile: one measurement, of the one image that
   * matters, taken when the selection changes rather than whenever a picture
   * happens to arrive.
   */
  $effect(() => {
    const chosen = signature;
    if (!chosen) return;
    const img = new Image();
    img.onload = () => {
      if (img.naturalWidth > 0) signatureAspect = img.naturalHeight / img.naturalWidth;
    };
    img.src = chosen.dataUri;
  });

  const destinations = [
    { id: "downloads", label: "Downloads" },
    { id: "desktop", label: "Desktop" },
    { id: "same_folder", label: "Same folder" },
  ];

  /**
   * A glyph per tool, keyed by the registry's own id.
   *
   * An unknown id falls back to a dot rather than an empty box: a tool this
   * build has not seen a picture for is still a tool, and the rail must not
   * lose its shape when the registry grows.
   */
  /**
   * The mark beside each tool in the rail.
   *
   * DRAWN ICONS, NOT TEXT GLYPHS. `Icon.svelte`'s header records why: glyphs
   * rendered in the UI font are how the gear came to look like a blob and
   * "show in folder" came to look like a parallelogram, which it was --
   * U+25B1. This map then repeated it, with twelve glyphs whose weight, size
   * and baseline all came from the font rather than from a 24x24 grid, and
   * whose meaning came from whatever the font happened to draw.
   *
   * `image-ocr` was never in the map at all, so it fell through to the "·"
   * fallback. That is the "read text has a dot for an icon" report, and it is
   * the same bug as the other eleven, only louder.
   *
   * Compression is deliberately the same mark on images and PDFs: one
   * operation, two kinds of file.
   */
  const ICONS: Record<string, IconName> = {
    "image-remove-background": "eraser",
    "image-upscale": "scaling",
    "image-invert": "contrast",
    "image-greyscale": "droplet-off",
    "image-compress": "minimize-2",
    "image-pick": "pipette",
    "image-ocr": "scan-text",
    // Lucide `layers`: several operations stacked into one result, which is
    // what this tool is. It had no entry when it was first advertised and
    // rendered as a blank -- the gap the comment above predicts, arriving
    // three minutes after the comment was written.
    "image-compose": "layers",
    "audio-denoise": "audio-waveform",
    "audio-transcribe": "file-text",
    "pdf-split": "square-split-vertical",
    "pdf-merge": "file-stack",
    "pdf-protect": "shield",
    "pdf-reorder": "arrow-up-down",
    "pdf-stamp": "pen-line",
    "pdf-compress": "minimize-2",
    "pdf-text": "file-text",
  };

  /** Put the stage back to empty so another file can be chosen. */
  function clearFile() {
    chosenPaths = [];
    shot = null;
    shotBefore = null;
    fitView();
    shotError = null;
    applied = new Set();
    saved = null;
    savedOutput = null;
    pdfPage = 1;
    // Everything below describes the document that just left. A placement
    // measured against page 4 of a document nobody has open any more is the
    // kind of state that reappears three files later on the wrong page.
    thumbs = {};
    thumbsFor = "";
    thumbToken += 1;
    sheets = {};
    sheetsFor = "";
    sheetToken += 1;
    audioToken += 1;
    peaksFor = {};
    probedFor = {};
    transcripts = {};
    transcriptCopied = null;
    merged = false;
    mergedThumbs = {};
    mergedPages = [];
    mergedDocumentOrder = [];
    mergeThumbs = {};
    mergeThumbToken += 1;
    documentPreview = null;
    documentPreviewToken += 1;
    placement = null;
    placedSignatures = [];
    runResults = [];
    placing = false;
    numbering = "";
    orderError = null;
  }

  /**
   * Whether choosing files here ADDS to the list or replaces it.
   *
   * Merge always accumulated, because a merge of one document is not a merge.
   * Audio now accumulates too: denoise and transcribe both run once per file,
   * the backend has always looped, and the only thing stopping a list of
   * recordings was this screen replacing the selection every time it was
   * asked for another one. Images and single-document PDF work still replace,
   * because those stages show one file and mean it.
   */
  /**
   * Batch: a list of files and no preview.
   *
   * The stage exists to show one picture and what an operation does to it.
   * Over forty photographs that is the wrong question -- there is no single
   * picture to show, rendering a preview of each costs a model run apiece, and
   * the decision ("remove the background from these") was already made before
   * the files were chosen. So batch drops the stage entirely and shows the
   * list, the way the home screen does.
   */
  /**
   * ARRIVING WITH SEVERAL FILES IS THE BATCH GESTURE.
   *
   * This started `false` always, so queueing ten photographs and opening
   * Invert colours put the workspace in single-file mode looking at the first
   * one, with nine files loaded and invisible and a "Batch" button the user
   * had to find before anything they had already chosen would happen.
   *
   * `$state(...)` and not `$derived`: it is the STARTING position, and the
   * user overrides it with the very control beside it.
   */
  // `untrack` says "the initial value, deliberately" -- without it Svelte
  // warns that a prop is being read outside a derived, which is the right
  // warning for the mistake this is not.
  let batchMode = $state(untrack(() => paths.length > 1));
  const runPaths = $derived((batchMode || current.id === "pdf-compress") ? activePaths.filter(path => !excludedPaths.has(path)) : activePaths);

  /**
   * Combining: several operations, one file, one receipt.
   *
   * **A MODE, NOT A TOOL.** It has been both. Originally the rail simply
   * composed -- two lit buttons meant one job -- which worked and which
   * nothing on screen said. Then it became a rail entry with tick boxes, which
   * said it but put the operations in a second place, so the rail listed seven
   * tools and one of them contained four of the others.
   *
   * Beside Single file and Batch is where it belongs, because it answers the
   * same kind of question they do: not *what* to do but *how this run works*.
   * Turning it on makes the rail multi-select and the tools are chosen the way
   * every other tool is chosen.
   */
  let combineMode = $state(false);

  /** Whether this tool can take part in a combined run. */
  const COMBINABLE = new Set([
    "image-remove-background",
    "image-upscale",
    "image-invert",
    "image-greyscale",
  ]);

  /** Combining is only offered where there is more than one thing to combine. */
  const canCombine = $derived(kind === "image");

  const accumulates = $derived(current.multiInput || kind === "audio" || batchMode || current.id === "pdf-compress");

  /**
   * The image operations that make sense over a folder of files.
   *
   * All four are per-file and parameterless: run the same thing on each
   * picture and write each result beside it. Nothing here needs to see the
   * result to decide anything, which is what makes a preview optional rather
   * than the point. The colour picker is deliberately absent -- it produces no
   * file, so "do it to forty images" means nothing.
   */
  /**
   * What the STAGE should show.
   *
   * The lit set IS the operation set again, which is the whole simplification
   * the mode buys. It stopped being true when combining was a tool: the lit
   * set was then the single word "image-compose", which the preview cannot
   * apply, so a second list -- the tick boxes -- had to be consulted to find
   * out what the picture was supposed to look like.
   */
  const previewOps = $derived([...applied]);

  const BATCHABLE = new Set([
    "image-remove-background",
    "image-upscale",
    "image-invert",
    "image-greyscale",
    // COMPRESSION IS THE MOST BATCH-SHAPED TOOL HERE. Nobody shrinks one
    // photograph; they shrink the folder they are about to email. It carries a
    // quality parameter, which the other four do not, and that is fine -- the
    // parameter is chosen once and applies to every file in the run, which is
    // exactly what a batch of one setting means.
    "image-compress",
  ]);

  /** Whether this tool can be pointed at a list rather than one picture. */
  const canBatch = $derived(kind === "image" && BATCHABLE.has(current.id));


  /**
   * Take files into the workspace, however they arrived.
   *
   * Shared by the file dialog and by a drop on the window, deliberately: the
   * two gestures mean the same thing, and the drop path being a separate copy
   * of this is how they would come to disagree about whether a second file
   * adds or replaces.
   */
  async function addPaths(picked: string[]) {
    if (picked.length === 0) return;
    if (accumulates && activePaths.length > 0) {
      // Already-listed files are not added twice; choosing the same folder
      // again is something people do to check it arrived.
      const known = new Set(activePaths);
      chosenPaths = [...activePaths, ...picked.filter((p) => !known.has(p))];
    } else {
      chosenPaths = picked;
    }
    // NEW FILES MEAN A NEW JOB, so the last one's confirmation goes.
    //
    // The after-save row replaces Save and now stands until Undo or the next
    // run -- which left no way to start that next run: adding files kept the
    // finished job's "Show in folder / Undo" and there was no Save button on
    // screen at all. Choosing more files IS the next run beginning.
    saved = null;
    savedOutput = null;
    lastRun = null;
    await loadPreview();
  }

  /** Ask the host for a file to work on, from inside the workspace. */
  async function openFile() {
    try {
      // The dialog offers this workspace's own kind first.
      const picked = await pickFiles(kind as "image" | "audio" | "pdf");
      if (picked) await addPaths(picked);
    } catch (e) {
      error = errorText(e);
    }
  }

  /**
   * THE WORKSPACE TAKES DROPS TOO.
   *
   * It did not. The only OS drag-drop subscription in the app lived in the
   * shell and fed the conversion batch, so dropping a picture onto the open
   * image editor did nothing at all — the one gesture the stage's own "or drop
   * one on the window" prompt invited. Registering here puts this workspace on
   * top of the shell's target for as long as it is open.
   */
  onMount(() => {
    let stop: (() => void) | undefined;
    let stopUnits: (() => void) | undefined;
    let disposed = false;
    void onToolProgress((event) => {
      if (!busy || !Number.isFinite(event.fraction) || event.total <= 0) return;
      const done = event.index + Math.min(0.99, Math.max(0, event.fraction));
      progress = { done: Math.max(progress?.done ?? 0, done), total: event.total };
    }).then((unlisten) => { if (disposed) unlisten(); else stopUnits = unlisten; });
    void onProgress((event) => {
      if (!busy) return;
      // `started` for file n means n are finished, `done` means n + 1 are.
      const done = event.phase === "done" ? event.index + 1 : event.index;
      progress = { done: Math.max(progress?.done ?? 0, done), total: event.total };
    }).then((unlisten) => {
      if (disposed) unlisten(); else stop = unlisten;
    });
    return () => { disposed = true; stop?.(); stopUnits?.(); };
  });

  onMount(() =>
    drop.register({
      enabled: () => !busy,
      onFiles: (paths) => {
        if (addingSignature && paths[0]) { void loadSignatureFile(paths[0]); return; }
        void addPaths(paths).catch((e) => {
          error = errorText(e);
        });
      },
    }),
  );

  /** True while a FILE drag is over the window and this workspace would take
   *  it. Named apart from `dragOver` below, which is the reorder board's
   *  in-page card drag and has nothing to do with the OS channel. */
  const fileDragOver = $derived(drop.dragging && !busy);

  /**
   * Drop one recording from the list. Nothing on disk is touched.
   *
   * Removing the last one empties the workspace back to its open prompt rather
   * than leaving a list with an Add tile and no explanation.
   */
  function removeAudio(path: string) {
    const next = activePaths.filter((p) => p !== path);
    chosenPaths = next;
    const { [path]: _peaks, ...restPeaks } = peaksFor;
    const { [path]: _probe, ...restProbes } = probedFor;
    const { [path]: _text, ...restText } = transcripts;
    peaksFor = restPeaks;
    probedFor = restProbes;
    transcripts = restText;
    if (transcriptCopied === path) transcriptCopied = null;
  }

  /**
   * The transcript, as the engine produced it.
   *
   * PLAIN TEXT, NOT TIMESTAMPED SEGMENTS. This was `{ t, s }[]` — a timestamp
   * and a line — and the panel rendered a two-column list of them. Whisper's
   * adapter (`transcribe_to_text`) returns one string with no timing at all,
   * and its own receipt says so: "speaker identity, timing, tone" are among
   * the things it lists as NOT carried across. The interface modelled data the
   * engine cannot produce, which is why nothing could ever fill it.
   *
   * Empty until a run fills it. There is deliberately no sample text: a page
   * of plausible speech under the heading "Transcript" is the worst thing this
   * screen could show, because it is indistinguishable from a real result.
   *
   * **Keyed by input path**, because transcription now runs over a list. One
   * shared string meant the second recording overwrote the first, and the
   * panel showed one transcript with no indication which file it came from.
   */
  let transcripts = $state<Record<string, string>>({});

  /**
   * The first couple of sentences of a transcript, for the row.
   *
   * Cut at a sentence boundary rather than a character count: a paragraph
   * sliced mid-word reads as damage, and the point of the excerpt is to
   * recognise the recording, which the first sentence or two does.
   */


  /** The text produced for one file, or the empty string. */
  function transcriptFor(path: string): string {
    return transcripts[path] ?? "";
  }


  // --- colour picker -------------------------------------------------------
  // Reads a pixel out of the rendered preview. Nothing is written and nothing
  // is sent: the sample comes from the PNG already on screen.
  let swatch = $state<{ hex: string; rgb: string } | null>(null);
  let stageImg = $state<HTMLImageElement | null>(null);

  /** Where the crosshair is, as a fraction of the image. Centre by default. */
  let cross = $state({ x: 0.5, y: 0.5 });

  /**
   * The loupe: the picker's one setting.
   *
   * Everything else the picker used to offer has gone. The before/after toggle
   * compared an image with itself, and the 1 / 3x3 / 5x5 "sample size" asked
   * for a number in image pixels about a preview scaled to the stage — a
   * question with no visible consequence, answered blind.
   *
   * On, the magnifier shows the neighbourhood around the crosshair at large
   * scale with hard pixel edges, and the sample is the single pixel under the
   * centre, because you can see exactly which one that is. Off, the sample is
   * averaged over 3x3, because aiming without magnification is imprecise and
   * an average is the forgiving answer.
   */
  let magnifier = $state(true);

  /** True while the pointer is held down on the image, sampling as it moves. */
  let sampling = $state(false);
  let samplePointer = $state<number | null>(null);
  /** Where the loupe sits on screen, in viewport pixels. */
  let loupeAt = $state({ x: 0, y: 0 });

  /** How many image pixels across the loupe shows. Odd, so one is the centre. */
  const LOUPE_PIXELS = 11;
  /** The loupe's diameter on screen. */
  const LOUPE_SIZE = 132;

  /**
   * Read the pixel under the crosshair, averaged over the chosen sample size.
   *
   * A single pixel is the wrong answer on anything photographic — sensor noise
   * moves it by several counts between neighbours — so the picker offers 3x3
   * and 5x5 and means them.
   */
  function sampleAtCross() {
    if (!stageImg || stageImg.naturalWidth === 0) return;
    // One pixel when the loupe is showing which one; a 3x3 average when it is
    // not. See `magnifier`.
    const size = magnifier ? 1 : 3;
    const half = Math.floor(size / 2);
    const cx = Math.min(Math.floor(cross.x * stageImg.naturalWidth), stageImg.naturalWidth - 1);
    const cy = Math.min(Math.floor(cross.y * stageImg.naturalHeight), stageImg.naturalHeight - 1);
    const x = Math.max(0, Math.min(cx - half, stageImg.naturalWidth - size));
    const y = Math.max(0, Math.min(cy - half, stageImg.naturalHeight - size));

    const c = document.createElement("canvas");
    c.width = size;
    c.height = size;
    const ctx = c.getContext("2d", { willReadFrequently: true });
    if (!ctx) return;
    ctx.drawImage(stageImg, x, y, size, size, 0, 0, size, size);
    const px = ctx.getImageData(0, 0, size, size).data;
    let sr = 0;
    let sg = 0;
    let sb = 0;
    const n = size * size;
    for (let i = 0; i < px.length; i += 4) {
      sr += px[i];
      sg += px[i + 1];
      sb += px[i + 2];
    }
    const [r, g, b] = [Math.round(sr / n), Math.round(sg / n), Math.round(sb / n)];
    const hex = `#${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
    swatch = { hex, rgb: `rgb(${r}, ${g}, ${b})` };
  }

  /** Aim at a viewport point, and read. */
  function aimAt(clientX: number, clientY: number) {
    if (!stageImg) return;
    const box = stageImg.getBoundingClientRect();
    // The preview is scaled to fit, so displayed pixels and image pixels are
    // different grids; normalise through the box rather than assuming 1:1.
    cross = {
      x: Math.min(Math.max((clientX - box.left) / box.width, 0), 0.999),
      y: Math.min(Math.max((clientY - box.top) / box.height, 0), 0.999),
    };
    loupeAt = { x: clientX, y: clientY };
    sampleAtCross();
  }

  /**
   * THE CROSSHAIR IS DRAGGED, NOT CLICKED.
   *
   * Clicking read one pixel and stopped, so finding the colour you actually
   * wanted meant clicking, looking at the swatch, clicking again, looking
   * again. Holding and moving reads continuously, which is how every other
   * eyedropper works and the only way the magnifier below is any use: you
   * watch the loupe and let go when the right pixel is under the centre.
   */
  function startSampling(event: PointerEvent) {
    if (!picking || event.button !== 0) return;
    event.preventDefault();
    sampling = true;
    samplePointer = event.pointerId;
    aimAt(event.clientX, event.clientY);
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Capture is an optimisation; releasing outside the image simply ends
      // the drag through the pointerup handler instead.
    }
  }

  function moveSampling(event: PointerEvent) {
    if (!sampling || event.pointerId !== samplePointer) return;
    aimAt(event.clientX, event.clientY);
  }

  function endSampling(event: PointerEvent) {
    if (event.pointerId !== samplePointer) return;
    sampling = false;
    samplePointer = null;
  }

  /**
   * Keyboard: arrows aim, Enter reads.
   *
   * "This pixel" has no natural keyboard gesture, so the crosshair gives it
   * one. A 1% step is fine work; Shift moves 10% to cross the image quickly.
   */
  function aim(event: KeyboardEvent) {
    if (!picking) return;
    const step = event.shiftKey ? 0.1 : 0.01;
    const clamp = (n: number) => Math.min(Math.max(n, 0), 0.999);
    switch (event.key) {
      case "ArrowLeft":
        cross = { ...cross, x: clamp(cross.x - step) };
        break;
      case "ArrowRight":
        cross = { ...cross, x: clamp(cross.x + step) };
        break;
      case "ArrowUp":
        cross = { ...cross, y: clamp(cross.y - step) };
        break;
      case "ArrowDown":
        cross = { ...cross, y: clamp(cross.y + step) };
        break;
      case "Enter":
      case " ":
        sampleAtCross();
        break;
      default:
        return;
    }
    event.preventDefault();
    event.stopPropagation();
  }

  /**
   * Copy the colour, in both notations.
   *
   * Hex alone was copied, and which one you need depends entirely on where it
   * is going: CSS and design tools take either, a lot of code wants the
   * channels. Both on two lines costs nothing to paste and saves going back to
   * read the other one off the screen.
   */


  /**
   * The page's shape, named only when it can be.
   *
   * With physical dimensions the paper size is named outright. Without them
   * only the ratio is knowable — and A4 and A3 share it — so the aspect is
   * reported instead of a guess.
   */
  const pageShape = $derived.by(() => {
    if (!shot) return null;
    const wPt = shot.pageWidthPt;
    const hPt = shot.pageHeightPt;
    if (wPt !== null && hPt !== null && wPt > 0 && hPt > 0) {
      const portrait = hPt >= wPt;
      const [shortMm, longMm] = [Math.min(wPt, hPt), Math.max(wPt, hPt)].map(
        (pt) => (pt / 72) * 25.4,
      );
      const papers: [string, number, number][] = [
        ["A5", 148, 210],
        ["A4", 210, 297],
        ["A3", 297, 420],
        ["A2", 420, 594],
        ["Letter", 216, 279],
        ["Legal", 216, 356],
        ["Tabloid", 279, 432],
      ];
      const hit = papers.find(
        ([, w, h]) => Math.abs(shortMm - w) <= 3 && Math.abs(longMm - h) <= 3,
      );
      const orient = portrait ? "portrait" : "landscape";
      return hit
        ? `${hit[0]} ${orient}`
        : `${Math.round(shortMm)} × ${Math.round(longMm)} mm ${orient}`;
    }
    const ratio = shot.height / shot.width;
    const iso = Math.abs(ratio - Math.SQRT2) < 0.02 || Math.abs(ratio - 1 / Math.SQRT2) < 0.02;
    const orient = shot.height >= shot.width ? "portrait" : "landscape";
    return iso ? `A-series ${orient} (1:√2)` : `${shot.width} × ${shot.height} ${orient}`;
  });

  /**
   * Why Save is refused, or null when it is allowed.
   *
   * A pixel operation has no `Target` to route through, and SR-11 means an
   * output with no receipt must not exist — only `route()` builds the plan a
   * receipt is made from. Saying so is better than a Save button that fails.
   */
  const savingBlocked = $derived.by(() => {
    // A DISABLED SAVE HAS TO SAY WHY. Combining with nothing chosen is a mode
    // that is on and has been asked to do nothing; without this the button is
    // simply grey.
    if (combineMode && applied.size === 0) {
      return "Choose the edits to combine.";
    }
    // A REQUIRED FIELD NOBODY HAS FILLED IN. Split needs its boundaries and
    // Password needs a password; without this the button is simply grey, and
    // the note beside it used to be about rearranging pages.
    const missing = current.params.find(
      (p) => p.required && !(current.id === "pdf-protect" && values.direction === "remove" && p.id === "password") && !(values[p.id] ?? "").trim(),
    );
    if (missing && applied.has(current.id) && current.id !== "pdf-stamp") {
      return `${missing.title} is needed first.`;
    }
    const blocked = siblings.filter((t) => applied.has(t.id) && t.previewOnly);
    if (blocked.length === 0) return null;
    return `${blocked.map((t) => t.title).join(" and ")} can be previewed but not saved yet.`;
  });

  // --- transcript -----------------------------------------------------------
  /** Which file's Copy was last pressed, so only that row confirms. */
  let transcriptCopied = $state<string | null>(null);

  let copiedColour = $state("");
  let colourCopyToken = 0;
  async function copyColour(value: string) {
    try {
      await navigator.clipboard.writeText(value);
      copiedColour = value;
      const token = ++colourCopyToken;
      setTimeout(() => { if (token === colourCopyToken) copiedColour = ""; }, 1800);
    } catch (e) { error = errorText(e); }
  }

  async function copyTranscript(path: string) {
    try {
      await navigator.clipboard.writeText(transcriptFor(path));
      transcriptCopied = path;
    } catch {
      // Clipboard access can be refused; the text is on screen regardless.
      transcriptCopied = null;
    }
  }

  // Matched against the registry's real ids. These used to test for fixture
  // spellings ("rmbg", "split") that exist nowhere in `tools.rs`, so the stage
  // showed the right thing in the harness and nothing in the desktop build.
  const bgRemoved = $derived(applied.has("image-remove-background"));
  // --- the PDF board --------------------------------------------------------
  //
  // One card per page (reorder) or per document (merge), dragged into the order
  // the output will have. The order lives here as an array of indices rather
  // than by mutating the source list, so "reset" is one assignment and the
  // original order is never lost.

  /** Current order. Indices into the page or document list. */
  let order = $state<number[]>([]);
  /** Which card is being dragged, by its position in `order`. */
  let dragFrom = $state<number | null>(null);
  /** Where it would land. */
  let dragOver = $state<number | null>(null);

  /**
   * POINTER EVENTS, NOT HTML5 DRAG-AND-DROP — and this is forced, not a
   * preference.
   *
   * The board used `draggable="true"` with `dragstart` / `dragover` / `drop`,
   * and nothing moved. The cause is in Tauri's own configuration doc for
   * `dragDropEnabled`: *"Disabling it is required to use HTML5 drag and drop on
   * the frontend on Windows."* The window sets it to `true` because the whole
   * point of the app is dropping files onto it from Explorer — the webview
   * hands OS drops to Rust, and in doing so swallows the in-page drag events.
   *
   * So the two features are mutually exclusive as written, and the file drop is
   * the one that cannot move. Pointer events are unaffected by that
   * interception: they are ordinary mouse and touch input, they work the same
   * on every platform, and they cost the keyboard nothing — the arrow keys
   * below still move a card, which is what makes this reachable without a
   * mouse at all.
   */
  let pointerId = $state<number | null>(null);
  let dragStart = { x: 0, y: 0 };
  let dragOffset = $state({ x: 0, y: 0 });
  /** Merge state is declared before board-derived values that consult it. */
  let merged = $state(false);
  let mergedPages = $state<number[]>([]);
  let mergedThumbs = $state<Record<number, string>>({});
  let mergedDocumentOrder = $state<number[]>([]);
  /** Document thumbnails are keyed by path so removing a card cannot shift a
   * late render onto the document that inherited its numeric slot. */
  let mergeThumbs = $state<Record<string, string>>({});
  let mergeThumbToken = 0;
  /**
   * The open full-document preview: its pages, and the shape of a page.
   *
   * EACH PAGE CARRIES ITS OWN RATIO, for two reasons.
   *
   * The pages load lazily, and a page with no declared shape has zero height
   * until its image arrives -- so the scroll container reports a length that
   * grows as you scroll, and the scrollbar, the only thing telling you how
   * long the document is, lies until you have already read it.
   *
   * And a document may MIX shapes. One ratio taken from page one and applied
   * to all of them squashes every landscape page in a portrait document into
   * portrait -- which is what a spreadsheet or a slide appended to a report
   * looks like, and is the reported "horizontal pages get squished".
   */
  /** One page of an open document preview: the render, and its own shape. */
  let documentPreview = $state<{ path: string; from: number; to?: number } | null>(null);
  let documentPreviewToken = 0;

  /** Begin a drag from `position`, capturing the pointer so it cannot escape. */
  function grab(event: PointerEvent, position: number) {
    // Left button or touch only; a right-click is a context menu, not a drag.
    if (event.button !== 0) return;

    // STATE FIRST, CAPTURE SECOND, and the order is the bug this had.
    //
    // `setPointerCapture` throws `NotFoundError` when the pointer id is not
    // currently active. Called before the assignments, one throw left
    // `dragFrom` null and the whole drag silently did nothing -- which is
    // exactly the symptom being fixed here, reintroduced one line further
    // down. Capture is an optimisation anyway: it keeps events coming to this
    // element if the pointer leaves it, and `track` finds the target with
    // `elementFromPoint` regardless. So a failure to capture must not cost the
    // drag.
    pointerId = event.pointerId;
    dragFrom = position;
    dragOver = position;
    dragStart = { x: event.clientX, y: event.clientY };
    dragOffset = { x: 0, y: 0 };
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Dragging still works; the pointer is simply not pinned to this card.
    }
  }

  /**
   * Track which card the pointer is over.
   *
   * `elementFromPoint` rather than each card's own `pointerover`: the pointer
   * is CAPTURED by the card that started the drag, so every subsequent event
   * goes there and the cards underneath never hear about it. Asking the
   * document what is under the cursor is how a captured drag learns where it
   * is.
   */
  function track(event: PointerEvent) {
    if (dragFrom === null || event.pointerId !== pointerId) return;
    event.preventDefault();
    dragOffset = {
      x: event.clientX - dragStart.x,
      y: event.clientY - dragStart.y,
    };
    const under = document.elementFromPoint(event.clientX, event.clientY);
    const card = under?.closest<HTMLElement>("[data-slot]");
    const slot = card?.dataset.slot;
    if (slot !== undefined) dragOver = Number(slot);
  }

  /** Drop where the pointer ended up. */
  function release(event: PointerEvent) {
    if (dragFrom === null || event.pointerId !== pointerId) return;
    const target = event.currentTarget as HTMLElement;
    try {
      if (target.hasPointerCapture(event.pointerId)) {
        target.releasePointerCapture(event.pointerId);
      }
    } catch {
      // Never captured, or already released. Neither changes the drop.
    }
    if (dragOver !== null) moveCard(dragFrom, dragOver);
    dragFrom = null;
    dragOver = null;
    pointerId = null;
    dragOffset = { x: 0, y: 0 };
  }

  /** A cancelled drag (Esc, a system gesture) leaves the order untouched. */
  function cancelDrag() {
    dragFrom = null;
    dragOver = null;
    pointerId = null;
    dragOffset = { x: 0, y: 0 };
  }

  /** Cards on the board: pages of one document, or the documents themselves. */
  /**
   * Whether the board is still asking about DOCUMENTS rather than pages.
   *
   * **Keyed on what is selected, not on which tool was clicked last.** It used
   * to be `current.id === "pdf-merge"`, so lighting Reorder while Merge was on
   * swapped the board out from under the user mid-decision: the documents they
   * had just put in order vanished and the pages of the first one appeared.
   *
   * Merging is a stage now. It holds until the user presses the button at the
   * bottom of the board, which performs the merge and then hands the pages to
   * the reorder view — the sequence they were trying to describe by selecting
   * both.
   */
  /**
   * MERGE SHOWS ITS BOARD FROM THE FIRST DOCUMENT, not the second.
   *
   * This required `activePaths.length > 1`, so opening Merge with one file
   * fell through to the ordinary stage -- a single page with Previous and
   * Next under it. Nothing on that screen was about merging, and nothing on it
   * offered a second document; the board carries an "add" card, and the board
   * was the thing not being shown.
   *
   * One document is not a merge and Save still refuses, saying so. But
   * refusing is a different thing from hiding the interface for the operation
   * the user just chose.
   */
  const arrangingDocuments = $derived(
    kind === "pdf" && applied.has("pdf-merge") && !merged && activePaths.length > 0,
  );

  const board = $derived.by(() => {
    if (arrangingDocuments) {
      return activePaths.map((p, i) => ({
        key: p,
        label: baseName(p),
        sub: "document",
        index: i,
      }));
    }
    return Array.from({ length: pdfPages }, (_, i) => ({
      key: `page-${i}`,
      label: `Page ${i + 1}`,
      sub: "",
      index: i,
    }));
  });

  /** The board length the current `order` was built for. */
  let orderBuiltFor = $state(-1);
  let boardIdentity = $state("");

  // Rebuild the order when the BOARD changes — a new document dropped in, or a
  // different page count.
  //
  // NOT `order.length !== board.length`, which is what this said and which
  // made every removal impossible. Dropping a page makes those two differ BY
  // DESIGN; the effect then saw the mismatch, decided the board had changed
  // and rebuilt the order from scratch — so the page came straight back, and
  // `1,6-4` reverted to 1-6 between one frame and the next. Two different
  // questions had one comparison standing in for both.
  $effect(() => {
    if (merged) return;
    const n = board.length;
    const identity = arrangingDocuments ? `documents:${activePaths.join("\n")}` : `pages:${sourcePath}`;
    if (orderBuiltFor !== n || boardIdentity !== identity) {
      if (boardIdentity !== identity) {
        resetPageEdits();
        editHistory.clear();
        selectedPages = new Set();
        pageDimensions = {};
      }
      boardIdentity = identity;
      orderBuiltFor = n;
      order = Array.from({ length: n }, (_, i) => i);
    }
  });

  /** Move a card, closing the gap behind it. */
  function moveCard(from: number, to: number) {
    if (from === to || from < 0 || to < 0) return;
    rememberEdit();
    const next = [...order];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    order = next;
    // The order field is the same decision by another route, so keep them
    // agreed rather than letting the board and the text disagree.
    values.order = order.map((i) => i + 1).join(",");
  }

  /** True when the output would differ from the document as it arrived.
   *
   * A REMOVAL COUNTS. This compared position against index only, so dropping
   * the last page left `[0,1,2,3,4]` — which matches `v === i` at every
   * position — and the board reported "original order" while holding five of
   * six pages, with no Reset offered to undo it. */
  const reordered = $derived(
    order.length !== (merged ? mergedPages.reduce((sum, n) => sum + n, 0) : board.length) ||
      order.some((v, i) => v !== i),
  );

  /**
   * Show every page of the document as its own card.
   *
   * **REORDER'S SCREEN, AND ONLY REORDER'S.** This was
   * `merge || reorder`, so lighting Merge alone laid every page of the
   * document out on the board -- which is what reordering looks like. On a
   * single multi-page PDF that is all you saw, because `arrangingDocuments`
   * needs two documents, so Merge fell straight through to the page board and
   * read as "merge turns reorder on by itself".
   *
   * Merge's own screen is the DOCUMENT board (`arrangingDocuments`), and after
   * the step it takes, the merged page list (`merged`). Neither is this.
   */
  const allPages = $derived(applied.has("pdf-reorder"));

  // ==========================================================================
  // PDF: how you want to reorder
  // ==========================================================================
  //
  // Three ways of saying the same thing, one at a time. They are not three
  // features — they are three input methods for one operation, and which one
  // is right depends entirely on the document. Dragging is unbeatable for
  // eight pages and unusable for two hundred; `1-5,10-8` is the reverse.
  //
  // The MODE decides what appears under the tool list. Drag shows nothing,
  // because dragging happens on the board and a field repeating it would be a
  // second place for the same answer to live.

  type ReorderMode = "drag" | "numbering" | "shortcuts";
  /**
   * PER-PAGE TURNS AND CROPS, keyed by the page's position in the OUTPUT.
   *
   * Not by its position in the source: the board reorders, and a turn recorded
   * against "page 3 of the original" would follow the wrong page the moment
   * anything moved. The compose parameters are read the same way -- page
   * numbers there are positions in the final document, which is also what the
   * signature's `page` has always meant.
   *
   * Turns are degrees clockwise, always a multiple of 90.
   *
   * **NOT 45 DEGREES.** The feedback asked for 45 per press, and a PDF page
   * cannot be turned by 45: `/Rotate` takes quarter circles, and the engine
   * refuses anything else outright rather than rounding it. Getting there
   * would mean rasterising the page and pasting it back as an image, which
   * turns a lossless page rearrangement into a picture of one. Ninety per
   * press, four presses back to where it started.
   */
  type PageEdit = { order: number[]; turns: Record<number,number>; crops: Record<number,[number,number,number,number]>; placement: typeof placement; signature: Signature | null; placedSignatures: typeof placedSignatures };
  const editHistory = new EditHistory<PageEdit>();
  let selectedPages = $state<Set<number>>(new Set());
  function editSnapshot(): PageEdit { return JSON.parse(JSON.stringify({ order, turns: pageTurns, crops: pageCrops, placement, signature, placedSignatures })); }
  function rememberEdit() { editHistory.record(editSnapshot()); }
  async function refreshPageThumbnails() {
    const path = sourcePath;
    await renderLanes(order, async (source) => {
      const rendered = await previewFile(path,source+1,220,pagePreviewOps(source+1));
      if (path === sourcePath) thumbs = { ...thumbs,[source+1]:rendered.dataUri };
    },()=>path!==sourcePath);
  }



  async function applyPageEditsToAll() {
    if (modifying === null) return;
    const crop = [...cropFor(modifying)];
    const turn = turnFor(modifying);
    rememberEdit();
    pageCrops = Object.fromEntries(order.map(source => [source + 1, [...crop] as [number, number, number, number]]));
    pageTurns = Object.fromEntries(order.map(source => [source + 1, turn]));
    await finishModify();
    await refreshPageThumbnails().catch(e => error = errorText(e));
  }



  let pageTurns = $state<Record<number, number>>({});
  /** Percentages off each edge: left, bottom, right, top. */
  let pageCrops = $state<Record<number, [number, number, number, number]>>({});
  /** Which output position is open in the modify panel, or null. */
  let modifying = $state<number | null>(null);
  let modifyShot = $state<FilePreview | null>(null);
  let modifyError = $state<string | null>(null);
  let pageDimensions = $state<Record<number, { width: number; height: number; rotation: number }>>({});
  async function openModify(position: number) {
    modifying = position;
    modifyError = null;
    modifyShot = null;
    const path = sourcePath;
    const page = (order[position] ?? position) + 1;
    try {
      const rendered = await previewFile(path, page, SHEET_PX, []);
      if (sourcePath !== path || modifying !== position) return;
      if (!rendered.pageWidthPt || !rendered.pageHeightPt) throw new Error(rendered.pageGeometryError ?? "This page has no usable dimensions. Try another page or document.");
      modifyShot = rendered;
      pageDimensions = { ...pageDimensions, [page]: { width: rendered.pageWidthPt, height: rendered.pageHeightPt, rotation: rendered.pageRotation ?? 0 } };
    } catch (e) { if (sourcePath === path && modifying === position) modifyError = errorText(e); }
  }

  function pagePreviewOps(page: number): string[] {
    const crop = pageCrops[page];
    return [...(crop ? [`pdf-crop:${crop.join(",")}`] : []), ...(pageTurns[page] ? [`pdf-turn:${pageTurns[page]}`] : [])];
  }

  async function finishModify() {
    const position = modifying;
    modifying = null;
    if (position === null) return;
    const page = (order[position] ?? position) + 1;
    const path = sourcePath;
    try {
      const rendered = await previewFile(path, page, 220, pagePreviewOps(page));
      if (sourcePath === path) thumbs = { ...thumbs, [page]: rendered.dataUri };
      sheetsFor = "";
    } catch (e) { error = errorText(e); }
  }

  const MODIFY_EDGES = ["left", "bottom", "right", "top"] as const;

  function turnFor(position: number): number {
    return pageTurns[(order[position] ?? position) + 1] ?? 0;
  }

  function cropFor(position: number): [number, number, number, number] {
    return pageCrops[(order[position] ?? position) + 1] ?? [0, 0, 0, 0];
  }

  /** Turn the open page another quarter, wrapping at four. */
  function turnPage(position: number) {
    rememberEdit();
    const next = (turnFor(position) + 90) % 360;
    const copy = { ...pageTurns };
    // Zero is "not turned", and a key whose value is zero would put
    // `3:0` into the compose parameters -- a stage that does nothing, in a
    // receipt that says it happened.
    if (next === 0) delete copy[(order[position] ?? position) + 1];
    else copy[(order[position] ?? position) + 1] = next;
    pageTurns = copy;
  }

  function setCrop(position: number, edge: number, value: number) {
    const current: [number, number, number, number] = [...cropFor(position)];
    current[edge] = Math.min(49, Math.max(0, value));
    const copy = { ...pageCrops };
    if (current.every((v) => v === 0)) delete copy[(order[position] ?? position) + 1];
    else copy[(order[position] ?? position) + 1] = current;
    pageCrops = copy;
  }

  /** `page:degrees;…`, or the empty string when nothing is turned. */
  const turnsParam = $derived(
    order.map((source, position) => pageTurns[source + 1] ? `${position + 1}:${pageTurns[source + 1]}` : "")
      .filter(Boolean)
      .join(";"),
  );

  /**
   * `page:l,b,r,t;…` in POINTS, or the empty string.
   *
   * The panel works in percentages because a slider from 0 to 49 % of a page
   * is a control anyone can use; the engine's `crop` takes points, because a
   * PDF box is in points. The conversion needs the page's own size, and the
   * one this workspace has is the RENDERED preview's -- which is the same
   * aspect ratio and a different scale. A percentage of the rendered width is
   * the same percentage of the real width, so the ratio is what travels.
   */
  const cropsParam = $derived.by(() => {
    return order.map((source, position) => {
      const crop = pageCrops[source + 1];
      const dimensions = pageDimensions[source + 1];
      if (!crop || !dimensions) return "";
      let [l, b, r, t] = crop;
      for (let angle = 0; angle < dimensions.rotation; angle += 90) [l, b, r, t] = [t, l, b, r];
      const { width: w, height: h } = dimensions;
      return `${position + 1}:${[l*w/100, b*h/100, r*w/100, t*h/100].map(n => n.toFixed(1)).join(",")}`;
    }).filter(Boolean).join(";");
  });

  /**
   * Dragging a crop edge on the page itself.
   *
   * **FOUR SLIDERS WERE THE WRONG INSTRUMENT.** Cropping is a spatial
   * decision -- this much off that side -- and the panel asked it as four
   * numbers, on a 78 px card, while the page they applied to was somewhere
   * else on screen at a size where nothing could be judged. Nobody crops a
   * photograph that way; they drag its edges, and this now does too.
   *
   * The state underneath is unchanged: `pageCrops` still holds four
   * percentages per page in `MODIFY_EDGES` order, and `cropsParam` still turns
   * them into points. Only the instrument changed.
   */
  let cropDrag = $state<{ edge: number; second?: number; pointer: number } | null>(null);
  let cropBox = $state<DOMRect | null>(null);

  /** The pointer, as a fraction of the page, clamped to it. */
  function cropFraction(event: PointerEvent): { x: number; y: number } | null {
    const box = cropBox;
    if (!box || box.width === 0 || box.height === 0) return null;
    return {
      x: Math.min(1, Math.max(0, (event.clientX - box.left) / box.width)),
      y: Math.min(1, Math.max(0, (event.clientY - box.top) / box.height)),
    };
  }

  function startCropDrag(event: PointerEvent, edge: number, page: number, second?: number) {
    rememberEdit();
    void page;
    const el = event.currentTarget as HTMLElement;
    cropBox = el.parentElement?.getBoundingClientRect() ?? null;
    cropDrag = { edge, second, pointer: event.pointerId };
    try {
      el.setPointerCapture(event.pointerId);
    } catch {
      // Dragging still works; the pointer is simply not pinned.
    }
  }

  function moveCropDrag(event: PointerEvent, page: number) {
    if (cropDrag === null || event.pointerId !== cropDrag.pointer) return;
    let at = cropFraction(event);
    if (!at) return;
    for (let angle = 0; angle < turnFor(page); angle += 90) at = { x: at.y, y: 1 - at.x };
    // Percentages FROM each side, which is what `pageCrops` holds: left and
    // bottom count inward from their own edge, right and top from theirs.
    const per = [
      at.x * 100,
      (1 - at.y) * 100,
      (1 - at.x) * 100,
      at.y * 100,
    ];
    setCrop(page, cropDrag.edge, Math.round(per[cropDrag.edge] ?? 0));
    if (cropDrag.second !== undefined) setCrop(page, cropDrag.second, Math.round(per[cropDrag.second] ?? 0));
  }

  function endCropDrag(event: PointerEvent) {
    if (cropDrag === null || event.pointerId !== cropDrag.pointer) return;
    cropDrag = null;
    cropBox = null;
  }

  /** Arrow keys move a handle, because a drag is not an interface everyone has. */
  function nudgeCrop(event: KeyboardEvent, edge: number, page: number) {
    const step = event.shiftKey ? 5 : 1;
    const grow = event.key === "ArrowRight" || event.key === "ArrowDown";
    const shrink = event.key === "ArrowLeft" || event.key === "ArrowUp";
    if (!grow && !shrink) return;
    rememberEdit();
    event.preventDefault();
    const now = cropFor(page)[edge] ?? 0;
    setCrop(page, edge, now + (grow ? step : -step));
  }

  let reorderMode = $state<ReorderMode>("drag");
  let orderError = $state<string | null>(null);
  /** What the user typed in numbering mode, before it is parsed. */
  let numbering = $state("");

  /**
   * Parse `1,2,3` and ranges like `1-5` or `10-8`.
   *
   * A DESCENDING range is not an error — `10-8` means pages ten, nine, eight,
   * in that order, and it is the shortest way to say "reverse this section".
   * Rejecting it would make the syntax a typo checker rather than a language.
   *
   * Returns one-based page numbers in the order given. Repeats are allowed:
   * `1,1,2` duplicates page one, which is a legitimate thing to ask for.
   */
  function parseNumbering(text: string, pages: number): number[] {
    const out: number[] = [];
    for (const raw of text.split(",")) {
      const part = raw.trim();
      if (part === "") continue;
      const range = /^(\d+)\s*-\s*(\d+)$/.exec(part);
      if (range) {
        const a = Number(range[1]);
        const b = Number(range[2]);
        if (a < 1 || b < 1 || a > pages || b > pages) {
          throw new Error(`this document has ${pages} pages, so ${part} is out of range`);
        }
        const step = a <= b ? 1 : -1;
        for (let n = a; step > 0 ? n <= b : n >= b; n += step) out.push(n);
        continue;
      }
      if (!/^\d+$/.test(part)) {
        throw new Error(`${part} is not a page number or a range`);
      }
      const n = Number(part);
      if (n < 1 || n > pages) {
        throw new Error(`this document has ${pages} pages, so there is no page ${n}`);
      }
      out.push(n);
    }
    if (out.length === 0) throw new Error("that leaves no pages at all");
    return out;
  }

  /** Apply what is in the numbering field to the board. */
  function applyNumbering() {
    try {
      const pages = parseNumbering(numbering, board.length);
      rememberEdit();
      order = pages.map((n) => n - 1);
      values.order = pages.join(",");
      orderError = null;
    } catch (e) {
      orderError = e instanceof Error ? e.message : String(e);
    }
  }

  /** Drop one page from the output. The source document is untouched. */
  function removeAt(position: number) {
    rememberEdit();
    if (order.length <= 1) return;
    const removed = order[position];
    order = order.filter((_, i) => i !== position);
    selectedPages = new Set([...selectedPages].filter(source => source !== removed));
    values.order = order.map((i) => i + 1).join(",");
  }

  /**
   * The shortcuts, one button each.
   *
   * Each acts on the CURRENT order rather than the original, so they compose:
   * remove the blank versos, then reverse what is left.
   */
  const SHORTCUTS: { id: string; label: string; run: () => number[] }[] = [
    { id: "reverse", label: "Reverse", run: () => [...order].reverse() },
    // Odd and even are the page numbers a PERSON sees — 1, 3, 5 — not indices.
    // Getting this backwards would silently delete the wrong half, and the two
    // are indistinguishable once the file is saved.
    { id: "drop-odd", label: "Drop odd", run: () => order.filter((i) => (i + 1) % 2 === 0) },
    { id: "drop-even", label: "Drop even", run: () => order.filter((i) => (i + 1) % 2 === 1) },
  ];

  /**
   * Put the board back to the document's own order.
   *
   * Rebuilt from `board` rather than from a remembered snapshot: the board is
   * derived from the open document, so it IS the original, and a stored copy
   * would be one more thing to keep in step with a merge.
   */
  /** Turns and crops belong to the arrangement, and go back with it. */
  function resetPageEdits() {
    pageTurns = {};
    pageCrops = {};
    modifying = null;
  }

  function resetOrder() {
    rememberEdit();
    resetPageEdits();
    order = board.map((_, i) => i);
    values.order = "";
    numbering = "";
    orderError = null;
    selectedPages = new Set();
    void refreshPageThumbnails().catch(e => error = errorText(e));
  }

  function runShortcut(id: string) {
    rememberEdit();
    const shortcut = SHORTCUTS.find((x) => x.id === id);
    if (!shortcut) return;
    const next = shortcut.run();
    if (next.length === 0) {
      orderError = "that would leave the document with no pages";
      return;
    }
    order = next;
    values.order = order.map((i) => i + 1).join(",");
    orderError = null;
  }

  // ==========================================================================
  // PDF: per-page thumbnails
  // ==========================================================================
  //
  // The board drew `shot.dataUri` on every card — the SAME rendered page,
  // repeated. A twelve-page document showed twelve copies of page one, which
  // makes the one thing the board exists for, recognising a page in order to
  // move it, impossible.
  //
  // So each card gets its own render. They arrive one at a time and out of
  // order; a card with nothing yet shows its placeholder rather than the wrong
  // page, because a wrong thumbnail is worse than a missing one.

  /** Rendered pages, keyed by one-based page number. */
  let thumbs = $state<Record<number, string>>({});
  /** Guards two passes racing when the document changes underneath one. */
  let thumbToken = 0;

  /** The document and page count `thumbs` was last built for. */
  let thumbsFor = $state("");

  /**
   * Full-size page renders, for signing.
   *
   * **Separate from `thumbs` on purpose.** The board's cards are 90 px wide, so
   * `thumbs` are rendered at 220 px — deliberately small, because rendering two
   * hundred pages at full size to fill thumbnails is minutes of work for
   * pictures nobody can see. The signing column reused those same 220 px
   * images inside a 560 px sheet: the page came out about a third of the
   * available width, and placing a signature on it meant aiming at a blurry
   * upscale of a thumbnail.
   *
   * These are loaded only when the stamp tool is actually selected, so the
   * cost is paid by the flow that needs it and by nothing else.
   */
  /**
   * The long edge, in pixels, of a page rendered for signing.
   *
   * 1024 was chosen when the sheet column was narrow. It is not enough now:
   * an A4 page at 1024 px tall is about 87 dpi, and the reported symptom was
   * that the page is too coarse to place a signature against and does not
   * survive being zoomed. 1600 is roughly 136 dpi -- sharp at the sizes this
   * column uses and still sharp at 2x, which is what zoom needs.
   *
   * Still only rendered when the signing tool is selected, so nothing else
   * pays for it.
   */
  const SHEET_PX = 1600;

  let sheets = $state<Record<number, string>>({});
  let sheetsFor = $state("");
  let sheetToken = 0;

  /** Render every page at signing size, once per document. */
  async function loadSheets() {
    if (kind !== "pdf" || activePaths.length === 0) return;
    // THE ZOOM IS PART OF THE KEY, because it changes what is rendered.
    //
    // The pages were rendered once at a fixed size and then scaled up by CSS,
    // so magnifying showed the same pixels bigger -- a blurrier page, which is
    // the opposite of what magnifying is for. Re-rendering at the displayed
    // size means 2x is genuinely twice the detail, out of pdfium, at the
    // resolution it will actually be drawn.
    const key = `${sourcePath}#${pdfPages}#${signZoom}`;
    if (sheetsFor === key) return;
    sheetsFor = key;

    const token = ++sheetToken;
    sheets = {};
    const stale = () => token !== sheetToken;
    const pages = Array.from({ length: pdfPages }, (_, i) => i + 1);
    await renderLanes(
      pages,
      async (page) => {
        // Capped: 3x of 1600 is 4800px a side, which is a real render and a
        // real allocation per page. The cap is generous enough that the
        // highest zoom is still sharper than the one below it.
        const px = Math.min(Math.round(SHEET_PX * signZoom), 3200);
        const rendered = await previewFile(sourcePath, page, px, pagePreviewOps(page));
        if (stale()) return;
        sheets = { ...sheets, [page]: rendered.dataUri };
        if (rendered.pageWidthPt && rendered.pageHeightPt) pageDimensions[page] = { width: rendered.pageWidthPt, height: rendered.pageHeightPt, rotation: 0 };
      },
      stale,
    );
  }

  /**
   * One render per page of the open document.
   *
   * **This used to be guarded on `thumbs` being empty — by its caller, while
   * this function is the only thing that fills it.** So it raced itself: a
   * second `loadPreview()` arriving before the first page came back saw an
   * empty map, started a second run, bumped the token, and the first run
   * aborted mid-loop. Selecting a tool is enough to trigger that, which is why
   * the board so often ended up holding page one and eleven placeholders.
   *
   * Keyed on the document and its page count instead. The same document does
   * not get rendered twice, a different one always does, and re-entry while a
   * run is in flight is a no-op rather than a restart.
   */
  async function loadThumbs() {
    if (kind !== "pdf" || activePaths.length === 0) return;
    const key = `${sourcePath}#${pdfPages}`;
    if (thumbsFor === key) return;
    thumbsFor = key;

    const token = ++thumbToken;
    thumbs = {};
    const stale = () => token !== thumbToken;
    // Small on purpose: these are 90px cards, and asking for 1024px renders of
    // two hundred pages is minutes of work for pictures nobody can see.
    const pages = Array.from({ length: pdfPages }, (_, i) => i + 1);
    await renderLanes(
      pages,
      async (page) => {
        const rendered = await previewFile(sourcePath, page, 220, []);
        if (stale()) return;
        thumbs = { ...thumbs, [page]: rendered.dataUri };
        if (rendered.pageWidthPt && rendered.pageHeightPt) pageDimensions[page] = { width: rendered.pageWidthPt, height: rendered.pageHeightPt, rotation: 0 };
      },
      stale,
    );
  }

  // ==========================================================================
  // PDF: signatures
  // ==========================================================================

  let signatures = $state<Signature[]>([]);
  let signature = $state<Signature | null>(null);
  /** True while the "add a signature" dialog is open. */
  let addingSignature = $state(false);
  let newSignatureSource = $state<string | null>(null);
  let newSignaturePreview = $state<string | null>(null);
  let newSignatureName = $state("");
  let signatureError = $state<string | null>(null);
  /**
   * True once "Place signature" is pressed: the signature follows the cursor
   * until a page is clicked. Cancelled by Escape or by pressing it again.
   */
  /**
   * How much the signing column magnifies the page.
   *
   * Placing a signature means aiming at a spot on a page, and at column width
   * a line of body text is a couple of pixels tall -- you cannot see whether
   * the signature sits on the line or through it. The page renders at
   * `SHEET_PX` so there is real detail to magnify.
   *
   * The placement is stored in FRACTIONS of the page, so zooming moves
   * nothing: a signature at 0.4 x 0.8 is at 0.4 x 0.8 at any magnification.
   */
  let signZoom = $state(1);

  /**
   * Re-render the pages whenever the magnification changes.
   *
   * `loadSheets` keys on the zoom, so this is what asks it to run again. The
   * page stays on screen at the old resolution while the new one arrives --
   * the placement is in fractions of the page, so nothing moves when they swap.
   */
  $effect(() => {
    // Named so the dependency is visible: the effect exists BECAUSE the zoom
    // changed, and `loadSheets` reads it again through its own key.
    const zoom = signZoom;
    if (zoom > 0 && signing && activePaths.length > 0) void loadSheets();
  });

  /** Step to the next magnification, wrapping back to 1x. */


  let placing = $state(false);
  /** Where the ghost is, in viewport pixels. */
  let ghost = $state({ x: -9999, y: -9999 });

  /**
   * Where the signature has been put, or null.
   *
   * Coordinates are FRACTIONS of the page, not pixels: the same placement has
   * to survive the window being resized, the column being scrolled, and the
   * conversion to PDF points at save time — where all that matters is where it
   * sits relative to the page.
   */
  let placement = $state<{ page: number; x: number; y: number; width: number } | null>(null);
  let placedSignatures = $state<{ signature: Signature; placement: { page: number; x: number; y: number; width: number }; params: Record<string, string> }[]>([]);
  function keepPlacement() {
    if (placement && signature) placedSignatures = [...placedSignatures, { signature, placement: {...placement}, params: stampParams() }];
    placement = null;
  }
  function selectPlacement(index: number) {
    const item = placedSignatures[index];
    placedSignatures = placedSignatures.filter((_, i) => i !== index);
    keepPlacement();
    signature = item.signature;
    placement = item.placement;
    placing = false;
  }
  let movingPlacement = $state(false);
  let resizingPlacement = $state(false);
  let placementPointer = $state<number | null>(null);
  let placementStart = { x: 0, y: 0, ox: 0, oy: 0, w: 0, boxW: 1, signX: 1 };

  async function refreshSignatures() {
    try {
      signatures = await listSignatures();
      // Keep the selection if it survived; otherwise fall to the first, so the
      // dropdown never shows a name that is no longer in the library.
      signature = signatures.find((x) => x.name === signature?.name) ?? signatures[0] ?? null;
      signatureError = null;
    } catch (e) {
      signatureError = errorText(e);
    }
  }

  /** Step one of adding: choose the PNG. Naming comes after. */
  async function chooseSignatureFile() {
    try {
      const picked = await pickFiles("image");
      if (!picked || picked.length === 0) return;
      await loadSignatureFile(picked[0]);
    } catch (e) { signatureError = errorText(e); }
  }

  async function loadSignatureFile(path: string) {
    try {
      newSignatureSource = path;
      // Show what was chosen before asking for a name: "name this" is a much
      // easier question when the thing is on screen.
      const rendered = await previewFile(path, 1, 320, []);
      newSignaturePreview = rendered.dataUri;
      newSignatureName = baseName(path).replace(/\.[^.]+$/, "");
      signatureError = null;
    } catch (e) {
      signatureError = errorText(e);
    }
  }

  /** Step two: store it under the name. */
  async function saveSignature() {
    if (!newSignatureSource || newSignatureName.trim() === "") return;
    try {
      const made = await addSignature(newSignatureName, newSignatureSource);
      await refreshSignatures();
      signature = signatures.find((x) => x.name === made.name) ?? made;
      closeSignatureDialog();
    } catch (e) {
      signatureError = errorText(e);
    }
  }

  function closeSignatureDialog() {
    addingSignature = false;
    newSignatureSource = null;
    newSignaturePreview = null;
    newSignatureName = "";
  }

  async function removeSignature(s: Signature) {
    try {
      await deleteSignature(s.name);
      placedSignatures = placedSignatures.filter(item => item.signature.name !== s.name);
      if (signature?.name === s.name) { placement = null; placing = false; }
      await refreshSignatures();
    } catch (e) {
      signatureError = errorText(e);
    }
  }

  /** Arm placement: the signature follows the cursor until a page is clicked. */
  function startPlacing() {
    if (!signature) return;
    if (!placing) keepPlacement();
    placing = !placing;
  }

  function trackGhost(event: PointerEvent) {
    if (!placing) return;
    ghost = { x: event.clientX, y: event.clientY };
  }

  /**
   * Drop the signature on a page.
   *
   * `page` is one-based. The fractions are measured against that page's own
   * rendered box, so they mean the same thing whatever size it is drawn at.
   * The anchor is the CENTRE of the signature, which is what "I put it here"
   * means when you are looking at a cursor.
   */
  function placeOnPage(event: MouseEvent, page: number) {
    if (!placing || !signature) return;
    rememberEdit();
    const box = (event.currentTarget as HTMLElement).getBoundingClientRect();
    placement = {
      page,
      x: Math.min(Math.max((event.clientX - box.left) / box.width, 0), 1),
      y: Math.min(Math.max((event.clientY - box.top) / box.height, 0), 1),
      // A quarter of the page wide: about the size of a signature on a letter,
      // and immediately adjustable by the handle.
      width: 0.25,
    };
    placing = false;
    values.page = String(page);
  }

  /** Which corner a resize was started from. `null` for a move. */
  type Corner = "nw" | "ne" | "sw" | "se";

  function grabPlacement(event: PointerEvent, corner: Corner | null) {
    if (!placement) return;
    rememberEdit();
    event.stopPropagation();
    const sheet = (event.currentTarget as HTMLElement).closest<HTMLElement>("[data-sheet]");
    const box = sheet?.getBoundingClientRect();
    placementPointer = event.pointerId;
    placementStart = {
      x: event.clientX,
      y: event.clientY,
      ox: placement.x,
      oy: placement.y,
      w: placement.width,
      boxW: box?.width ?? 1,
      // The anchor is the CENTRE, so a left-hand corner grows when it is
      // dragged left and a right-hand one when it is dragged right. Without
      // the sign, the two corners on the left would shrink the signature when
      // pulled outward, which is the opposite of what the gesture means
      // everywhere else.
      signX: corner === "nw" || corner === "sw" ? -1 : 1,
    };
    if (corner !== null) resizingPlacement = true;
    else movingPlacement = true;
    try {
      (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    } catch {
      // Same reasoning as the board: capture is an optimisation, never the
      // thing the gesture depends on.
    }
  }

  function movePlacement(event: PointerEvent) {
    if (!placement || event.pointerId !== placementPointer) return;
    const sheet = (event.currentTarget as HTMLElement).closest<HTMLElement>("[data-sheet]");
    const box = sheet?.getBoundingClientRect();
    if (!box) return;
    if (movingPlacement) {
      placement = {
        ...placement,
        x: Math.min(
          Math.max(placementStart.ox + (event.clientX - placementStart.x) / box.width, 0),
          1,
        ),
        y: Math.min(
          Math.max(placementStart.oy + (event.clientY - placementStart.y) / box.height, 0),
          1,
        ),
      };
    } else if (resizingPlacement) {
      // Drag right to grow. Floored at 2% so it cannot be shrunk into
      // something too small to grab again.
      const delta = ((event.clientX - placementStart.x) / placementStart.boxW) * placementStart.signX;
      placement = {
        ...placement,
        width: Math.min(Math.max(placementStart.w + delta * 2, 0.02), 1),
      };
    }
  }

  function releasePlacement(event: PointerEvent) {
    if (event.pointerId !== placementPointer) return;
    movingPlacement = false;
    resizingPlacement = false;
    placementPointer = null;
  }

  /**
   * True when a PDF tool is selected but there is nothing for it to do yet.
   *
   * Saving a "merge" of one document, or a "reorder" that reorders nothing,
   * writes a copy of the input under a new name — which looks like the tool
   * failing quietly. Naming the condition on the button is better than
   * producing a file that is identical to the one it came from.
   */
  const pageEdits = $derived(turnsParam !== "" || cropsParam !== "");

  /**
   * Whether a COMPOSING PDF tool has been given nothing to compose.
   *
   * **Only the composing tools.** This asked the question of every PDF tool,
   * and merge/reorder/sign are the only three it can answer: for Compress,
   * Split and Password all three sub-conditions are false by construction, so
   * `pdfNothingToDo` was true the moment any of them was selected — and the
   * footer told the user to "rearrange the pages" under a compression tool.
   *
   * What blocks those three is a missing parameter, which is
   * `savingBlocked`'s question, not this one.
   */
  const pdfNothingToDo = $derived.by(() => {
    if (kind !== "pdf" || activePaths.length === 0) return false;
    if (!COMPOSES.has(current.id)) return false;
    const merging = applied.has("pdf-merge") && activePaths.length > 1;
    // TURNING OR CROPPING A PAGE IS SOMETHING TO DO. Without this, editing a
    // page without also moving one left Save disabled under "nothing would
    // change" -- which was true of the ORDER and false of the document.
    const rearranging = applied.has("pdf-reorder") && (reordered || pageEdits);
    const signing_ = applied.has("pdf-stamp") && (placedSignatures.length > 0 || (placement !== null && signature !== null));
    return !merging && !rearranging && !signing_;
  });

  /** Signing shows one scrolling column of pages, not the side-by-side board. */
  const signing = $derived(kind === "pdf" && applied.has("pdf-stamp"));


  // ==========================================================================
  // PDF: merge, in two stages
  // ==========================================================================
  //
  // Upload, choose the order of the DOCUMENTS, press Next, and then every page
  // of all of them is open as though it were one document — where reordering,
  // removing and signing all happen in the same view.
  //
  // Two stages because the two questions are different sizes. "Which document
  // comes first" is a decision about three or four things; "which page goes
  // where" is a decision about ninety. Putting both on one board means
  // answering the first one by dragging through the second.

  /**
   * How many page renders may be in flight at once.
   *
   * Each one is a worker process rendering a page, so this is a real cost and
   * not a scheduling nicety. Four is enough to hide the per-call latency on
   * the boards people actually look at, and small enough that a hundred-page
   * document does not try to start a hundred workers.
   */
  const RENDER_LANES = 4;

  /**
   * Render `items` a few at a time, reporting each as it lands.
   *
   * `stale()` is checked before every start and before every report, so a
   * document swapped out mid-pass abandons the rest instead of writing pages
   * of the previous one into the new one's board.
   */
  async function renderLanes<T>(
    items: T[],
    render: (item: T) => Promise<void>,
    stale: () => boolean,
  ) {
    const queue = [...items];
    const lanes = Array.from({ length: Math.min(RENDER_LANES, queue.length) }, async () => {
      for (;;) {
        const item = queue.shift();
        if (item === undefined || stale()) return;
        try {
          await render(item);
        } catch {
          // One page that will not render keeps its placeholder; the rest of
          // the document still arrives.
        }
      }
    });
    await Promise.all(lanes);
  }

  /**
   * The first page of every open document, for the merge board's cards.
   *
   * **In parallel, because in series it read as broken.** Each render is a
   * round trip to a worker; done one after another, a board of four documents
   * showed empty placeholders and then filled in one card at a time over
   * several seconds, which was reported as the previews "not loading
   * immediately". The work is the same; only the waiting was serial.
   */
  async function loadMergeThumbs() {
    const token = ++mergeThumbToken;
    const pathsNow = [...activePaths];
    mergeThumbs = {};
    const stale = () => token !== mergeThumbToken;
    await renderLanes(
      pathsNow,
      async (path) => {
        const rendered = (await previewFile(path, 1, 220, [])).dataUri;
        if (stale() || !activePaths.includes(path)) return;
        mergeThumbs = { ...mergeThumbs, [path]: rendered };
      },
      stale,
    );
  }

  /**
   * The long edge of a page in the full-document preview.
   *
   * 420 was right for the contact sheet this used to be, where a page was
   * about 180 px wide. It is now a scrolling document at up to 760 px, and a
   * 420 px render stretched to that is the blur that made the preview hard to
   * read. 1000 covers the widest case at 1x and stays crisp on a 2x display.
   */


  /**
   * THE TOOLS THAT NEED TO READ THE DOCUMENT, not look at one page of it.
   *
   * Both ask a question about a page you have to find first. "Split after
   * page 5" means nothing until you have seen what is on page 5, and a
   * password prompt over a single rendered page tells you nothing about which
   * document you are unlocking. The stage showed page one and a pager, so
   * finding page 5 was five presses of a button beside a picture.
   *
   * Merge already had the answer: `previewDocument` renders every page into
   * one scrolling column. It is the same component, opened automatically
   * rather than behind an eye icon, because for these two there is only ever
   * one document and nothing to choose between.
   */
  /// Kept for the record: these two are the tools the reading column was
  /// built for, and the reason it is now the default rather than a list.
  const READS_THE_DOCUMENT = new Set(["pdf-split", "pdf-protect"]);

  /**
   * The documents a split would write, from the boundaries typed so far.
   *
   * **A SPLIT IS THE ONE OPERATION WHOSE RESULT IS SEVERAL FILES**, and the
   * screen showed one: the source document, scrolling, exactly as it looked
   * before anything was typed. So "2,5,10" -- four files out of one -- was a
   * string in a box with nothing to check it against, and the only way to find
   * out what it meant was to run it and look in the folder.
   *
   * Merge already had the answer to this shape of question: a card per
   * document, with a preview. This is the same board for the other direction.
   *
   * Empty until the boundaries parse, because a half-typed "2," describes
   * nothing and a board that flickers per keystroke is worse than one that
   * waits.
   */
  let splitMode = $state("boundaries"), splitEvery = $state(1);
  let splitNames = $state<Record<number,string>>({});
  const splitBoundaries = $derived(splitMode === "boundaries" ? (values.boundaries ?? "") : Array.from({length: Math.max(0,pdfPages-1)},(_,i)=>i+1).filter(n=>n % (splitMode === "pages" ? 1 : Math.max(1,Math.floor(splitEvery))) === 0).join(","));
  const splitParts = $derived.by(() => {
    if (current.id !== "pdf-split") return [];
    const raw = splitBoundaries.trim();
    if (raw === "") return [];
    const total = pdfPages;
    const cuts: number[] = [];
    for (const piece of raw.split(",")) {
      const n = Number(piece.trim());
      // ANY nonsense means no board, rather than a board built from the parts
      // that happened to parse. A guess at what someone half-typed is a guess
      // presented as a plan.
      if (!Number.isInteger(n) || n < 1 || n >= total) return [];
      cuts.push(n);
    }
    const sorted = [...new Set(cuts)].sort((a, b) => a - b);
    if (sorted.length === 0) return [];
    const parts: { from: number; to: number }[] = [];
    let start = 1;
    for (const cut of sorted) {
      parts.push({ from: start, to: cut });
      start = cut + 1;
    }
    parts.push({ from: start, to: total });
    return parts;
  });
  void READS_THE_DOCUMENT;

  /**
   * How wide the pages are drawn, as a fraction of the column.
   *
   * SEPARATE FROM `zoom`. That one is a transform on the image stage and pans
   * with a pointer; this is a width, and the column scrolls on its own. Using
   * one variable for both would mean panning a document that has no pan, or a
   * scroll position that jumps when the picture stage is zoomed.
   */
  /**
   * Whether the reading column is the STAGE or something the user opened.
   *
   * The same component serves both: it is the whole stage for Compress, Split
   * and Password, and it is what the eye icon opens over the merge board. The
   * two differ in when they should close -- the stage's copy has to go the
   * moment another tool wants the room, and the eye's copy must not be shut
   * under someone who deliberately opened it.
   *
   * Without the distinction, switching from Compress to Merge left the
   * previous tool's reading column on screen and the merge board rendered
   * underneath it, invisible: the `{:else if documentPreview}` branch wins.
   */
  let readerIsStage = $state(false);

  /**
   * True while the stage is the reading column rather than a single page.
   *
   * **EVERY PDF TOOL THAT IS NOT SHOWING A BOARD READS THE DOCUMENT.** It
   * started as a list of two tools, which was too narrow: the default stage
   * was a rendered page with Previous and Next under it, so Compress -- and
   * Merge, before it had its board back -- put the user in a page-at-a-time
   * viewer to look at a document. Nobody reads a PDF that way, and no viewer
   * anyone uses works that way.
   *
   * The three exceptions are the views that are not reading: arranging
   * documents, arranging pages, and placing a signature all need to see many
   * pages at once, and each has a layout built for it.
   */
  const reading = $derived(
    kind === "pdf" &&
      current.id !== "pdf-compress" && splitParts.length === 0 &&
      activePaths.length > 0 &&
      !arrangingDocuments &&
      !allPages &&
      !signing &&
      !merged,
  );

  $effect(() => {
    // Opened on selection, and closed again on the way out: the column is
    // this tool's stage, not a mode the user turned on and has to turn off.
    const wants = reading;
    const path = activePaths[0];
    if (wants && (path && documentPreview?.path !== path || !readerIsStage)) {
      readerIsStage = true;
      void previewDocument(0);
    } else if (!wants && documentPreview && readerIsStage) {
      // Only the stage's own copy. The eye icon's is the user's, and shutting
      // it because some other state changed would be closing a window they
      // opened.
      readerIsStage = false;
      closeDocumentPreview();
    }
  });

  async function previewDocument(slot: number, range?: { from: number; to: number }) {
    const path=activePaths[slot]; if(!path)return;
    documentPreview={path,from:range?.from ?? 1,to:range?.to};
  }

  function closeDocumentPreview() {
    readerIsStage = false;
    documentPreviewToken += 1;
    documentPreview = null;
    busy = false;
  }

  function removeDocument(slot: number) {
    const removed = activePaths[slot];
    mergeThumbToken += 1;
    if (documentPreview?.path === removed) closeDocumentPreview();
    const next = [...activePaths];
    next.splice(slot, 1);
    chosenPaths = next;
    orderBuiltFor = -1;
    mergeThumbs = {};
    void loadPreview();
  }

  /**
   * Open every page of every document as one sequence.
   *
   * The page counts have to be asked for one document at a time: the merge has
   * not happened yet — nothing is written until Save — so there is no combined
   * document to count. Rendering page 1 of each is how its page count arrives.
   */
  async function openMerged() {
    if (activePaths.length === 0) return;
    closeDocumentPreview();
    busy = true;
    error = null;
    try {
      const counts: number[] = [];
      mergedDocumentOrder = [...order];
      for (const slot of mergedDocumentOrder) {
        const path = activePaths[slot];
        if (path === undefined) continue;
        const first = await previewFile(path, 1, 220, []);
        counts.push(first.pageCount);
      }
      mergedPages = counts;
      merged = true;

      // Then the thumbnails, in the merged sequence.
      mergedThumbs = {};
      let n = 1;
      for (const [i, slot] of mergedDocumentOrder.entries()) {
        const path = activePaths[slot];
        if (path === undefined) continue;
        for (let page = 1; page <= (counts[i] ?? 0); page += 1) {
          try {
            const rendered = await previewFile(path, page, 220, []);
            mergedThumbs = { ...mergedThumbs, [n]: rendered.dataUri };
          } catch {
            // A page that will not render keeps its placeholder; the sequence
            // must not shift because one thumbnail failed.
          }
          n += 1;
        }
      }
      order = Array.from({ length: n - 1 }, (_, i) => i);
      orderBuiltFor = n - 1;
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }

  /** Back to arranging documents. The document order is kept. */
  function unmerge() {
    merged = false;
    order = [...mergedDocumentOrder];
    orderBuiltFor = activePaths.length;
    mergedThumbs = {};
    mergedPages = [];
  }

  /** Which document each merged page came from, for its label. */
  const mergedLabels = $derived.by(() => {
    const out: { label: string; doc: string }[] = [];
    for (const [i, slot] of mergedDocumentOrder.entries()) {
      const doc = baseName(activePaths[slot] ?? "");
      for (let page = 1; page <= (mergedPages[i] ?? 0); page += 1) {
        out.push({ label: `${page}`, doc });
      }
    }
    return out;
  });
</script>

<!--
  THE SETTINGS FOR THE SELECTED TOOL, under the tool that owns them.

  They used to sit in a band above the stage, and the note there gave the
  reason plainly: the rail was a 216 px column, and a page-order field, a
  signature library and a row of choice controls do not fit inside the width of
  a button strip. But that reason was about HOW WIDE THE COLUMN WAS, and a
  column's width is a decision, not a constraint. The rail is wider now and
  every control runs its full width, which is the arrangement the band was
  standing in for.

  Two things it buys. The control sits under the button that turns it on, so
  there is one place to look rather than two on opposite sides of the window.
  And the stage gets back the height the band was holding -- for reorder, on a
  900 px window, that was about an eighth of the document.

  The signing row and the magnifier come back here from the FOOTER, where they
  were parked in the batch selector's slot for exactly the same reason: the
  rail had no room, and now it does.

  NO HEADING. "Compress settings", under a lit button reading Compress, in a
  workspace whose header reads Compress, is the third time.

  The condition is on `applied`, not on `current`, and that is the rule the
  band already followed: deselecting a tool used to leave its settings on
  screen under a rail where nothing was lit.
-->
{#snippet toolSettings()}
  {#if tiers.length > 1 || params.length > 0 || ["audio-transcribe", "pdf-reorder", "pdf-stamp", "pdf-split", "image-pick"].includes(current.id)}
  <fieldset class="settings-fieldset" disabled={busy} aria-label={`${current.title} settings`}>
  {#if current.id === "audio-transcribe"}
    <div class="tool-settings"><div class="seg" role="group" aria-label="Transcript output">
      <button type="button" class:on={(values.format ?? "txt") === "txt"} aria-pressed={(values.format ?? "txt") === "txt"} onclick={()=>values.format="txt"}>Text</button>
      <button type="button" class:on={values.format === "vtt"} aria-pressed={values.format === "vtt"} onclick={()=>values.format="vtt"}>Timed subtitles</button>
    </div></div>
  {/if}

  <!--
    WHICH MODEL, when the tool has more than one.

    Above the tool's own parameters, because it is a bigger decision than any
    of them: it changes how long the run takes and how good it is, and it may
    involve a download. One tier is not a choice, so nothing renders.
  -->
  {#if tiers.length > 1}
    <div class="tool-settings">
      <div class="seg tier-seg" role="group" aria-label="Model">
        {#each tiers as f (f.id)}
          <button
            type="button"
            aria-pressed={f.active}
            disabled={busy || tierBusy !== null}
            title={f.downloaded ? f.does : `${f.does} (${tierSize(f.sizeBytes)} to download)`}
            onclick={() => void chooseTier(f)}
          >
            {tierLabel(f.tier)}

          </button>
        {/each}
      </div>
    </div>
  {/if}
  <!-- NO TICK BOXES. They listed four operations that are already four
       buttons in the rail immediately above them, so choosing "invert" meant
       finding it in whichever of the two lists you happened to look at. The
       mode makes the rail itself multi-select, and the rail is the list. -->
  {#if current.id === "pdf-reorder"}
    <div class="tool-settings reorder-settings">
      <div class="reorder-row">
        <!-- No "How" label. Drag / Numbering / Shortcuts describe
             themselves; the group keeps its accessible name. -->
        <div class="seg reorder-mode" role="group" aria-label="How to reorder">
          {#each [["drag", "Drag"], ["numbering", "Numbering"], ["shortcuts", "Shortcuts"]] as [id, label] (id)}
            <button
              type="button"
              aria-pressed={reorderMode === id}
              onclick={() => {
                reorderMode = id as ReorderMode;
                orderError = null;
              }}
            >
              {label}
            </button>
          {/each}
        </div>

        {#if reorderMode === "numbering"}
          <!-- The example IS the instruction. Three lines of prose about
               commas and ranges said less than the placeholder does. -->
          <input
            class="field reorder-field"
            type="text"
            aria-label="New page order"
            placeholder="1-5, 10-8 — ranges count down to reverse"
            bind:value={numbering}
            onkeydown={(e) => {
              if (e.key === "Enter") applyNumbering();
            }}
          />
          <button type="button" class="quiet" onclick={applyNumbering}>Apply</button>
        {:else if reorderMode === "shortcuts"}
          <!-- Side by side, on the segment's baseline. Each acts on the
               CURRENT order, so they compose: drop the blank versos, then
               reverse what is left. -->
          {#each SHORTCUTS as sc (sc.id)}
            <button type="button" class="quiet" onclick={() => runShortcut(sc.id)}>
              {sc.label}
            </button>
          {/each}
        {/if}
      </div>

      {#if orderError}<p class="param-error" role="alert">{orderError}</p>{/if}
    </div>
  {:else if current.id === "pdf-stamp"}
    <!-- IN THE SHARED WRAPPER, like every other tool's settings.

         This panel and the colour picker's were the two that rendered their
         controls straight into the well, so they alone missed the inset every
         other tool gets -- which is the "sign does not have the same padding
         from every border as modify pages" report. The wrapper is the only
         place that inset is declared, so being outside it is the whole of the
         difference. -->
    <div class="tool-settings">
      <div class="sign-row">
        <Dropdown
          label="Signature"
          placeholder="Add signature"
          value={signature?.name ?? null}
          options={[
            ...signatures.map((sg) => ({
              value: sg.name,
              label: sg.name,
              preview: sg.dataUri,
              removable: true,
            })),
            { value: "__add", label: "Add signature…" },
          ]}
          onremove={(v) => {
            const sg = signatures.find((s) => s.name === v);
            if (sg) void removeSignature(sg);
          }}
          onchange={(v) => {
            if (v === "__add") {
              addingSignature = true;
              return;
            }
            keepPlacement();
            signature = signatures.find((sg) => sg.name === v) ?? null;
          }}
        />

        <div class="placement-actions">
        <button
          type="button"
          class="quiet place"
          class:armed={placing}
          disabled={!signature}
          onclick={startPlacing}
        >
          {placing ? "Click a page" : "Place signature"}
        </button>

        {#if placement}
          <button type="button" class="quiet" aria-label="Remove placed signature" onclick={() => { rememberEdit(); placement = null; placing = false; }}><Icon name="trash-2" size={15} /></button>
        {/if}
        </div>

      </div>
    </div>
  {:else if current.id === "image-pick"}
    <div class="tool-settings">
      <div class="seg mode" role="group" aria-label="Magnifier">
        <button type="button" aria-pressed={!magnifier} onclick={() => (magnifier = false)}>
          No magnifier
        </button>
        <button type="button" aria-pressed={magnifier} onclick={() => (magnifier = true)}>
          Magnifier
        </button>
      </div>
      <ColourPalette colour={swatch} />
    </div>
  {:else if current.id === "pdf-split"}
    <div class="tool-settings"><div class="seg" role="group" aria-label="Split method">{#each [["boundaries","At pages"],["every","Every N"],["pages","Each page"]] as [id,label]}<button type="button" aria-pressed={splitMode===id} onclick={()=>splitMode=id}>{label}</button>{/each}</div>
    {#if splitMode==="boundaries"}<input class="field" aria-label="Split after pages" placeholder="2,5,10" bind:value={values.boundaries} />{:else if splitMode==="every"}<input class="field" type="number" min="1" max={pdfPages} aria-label="Pages per document" bind:value={splitEvery} />{/if}
    </div>
  {:else if params.length > 0}
    <div class="tool-settings">
      {#each params as p (p.id)}
        <div class="param">
          <!-- A TEXT FIELD CARRIES ITS OWN LABEL.

               Every parameter had a caption above it, which is right for a
               segment or a slider -- neither can hold text -- and a duplicate
               for a field that can. On the password tool it read "Password"
               over a box, under a segment reading Password, under a lit rail
               button reading Password: the same word four times in a 288 px
               column.

               The accessible name does not move: `aria-label` stays on the
               input, so a screen reader announces exactly what it did before.
               Only the visible caption goes, and only where the placeholder
               takes its place. -->
          <!-- NO CAPTION OVER A LONE CONTROL.

               A text field carries its own label as a placeholder, and a
               segment under a lit button reading "Compress" does not need the
               word "Compression" above it -- that is the tool's name a second
               time, in the same 288 px column, four pixels apart.

               More than one parameter is the case where captions earn their
               place: Crop declares five, and four of them are edges that are
               only distinguishable by name. So the rule is the count, not the
               kind. -->
          {#if params.length > 1 && p.kind !== "text" && p.kind !== "number" && p.id !== "direction"}
            <span class="param-title">{p.title}</span>
          {/if}

          {#if p.kind === "choice"}
            <div class="seg" role="group" aria-label={p.title}>
              {#each p.options ?? [] as opt (opt.value)}
                <button
                  type="button"
                  aria-pressed={values[p.id] === opt.value}
                  onclick={() => (values[p.id] = opt.value)}
                >
                  {opt.label}
                </button>
              {/each}
            </div>
          {:else if p.kind === "range"}
            <div class="range">
              <input
                type="range"
                aria-label={p.title}
                min={p.min ?? 0}
                max={p.max ?? 100}
                step={p.step ?? 1}
                bind:value={values[p.id]}
              />
              <output>{values[p.id]}</output>
            </div>
          {:else}
            <input
              class="field"
              type={p.id.includes("password") ? "password" : p.kind === "number" ? "number" : "text"}
              aria-label={p.title}
              placeholder={p.title}
              bind:value={values[p.id]}
              required={p.required}
            />
          {/if}
        </div>
      {/each}

    </div>
  {/if}
  </fieldset>
  {/if}
{/snippet}

<section class="workspace" aria-labelledby="tool-heading">
  <header>
    <div class="title">
      <h2 id="tool-heading">{tool.category} · {current.title}</h2>
      <!-- ONE FILE IS A NAME; SEVERAL ARE A COUNT. The header printed
           `activePaths[0]` whatever was open, so a list of three recordings was
           titled after the first one — the same "first file stands for all of
           them" assumption the stage used to make. -->
      <span class="file">
        {accumulates && activePaths.length > 1
          ? `${activePaths.length} files`
          : fileName}
      </span>
    </div>
    <button type="button" class="close" onclick={onclose} aria-label="Close tool">
      Close <span class="key" aria-hidden="true">Esc</span>
    </button>
  </header>

  <div class="body">
    <!-- The tool rail, on the left. A Photoshop-shaped arrangement: what you
         can do sits beside the thing you are doing it to, and stays put while
         the stage changes. -->
    <aside class="rail" aria-label={`${tool.category} tools`}>
      {#if combineMode && kind === "image"}
      <h3>Combine edits</h3>
      <ImageWorkflowControls ops={[...applied].filter(id=>COMBINABLE.has(id))} bind:format={composeFormat} selected={current.id} disabled={busy} onchange={ops=>{applied=new Set(ops);void loadPreview();}} onselect={id=>{picked=siblings.find(t=>t.id===id)??null;}}>
        {@render toolSettings()}
      </ImageWorkflowControls>
      {:else}
      <h3>Tools</h3>
      <ul>
        {#each siblings as t (t.id)}
          <li class:open={current.id === t.id && applied.has(t.id)}>
            <button
              type="button"
              class="cmd"
              class:on={applied.has(t.id)}
              class:current={current.id === t.id}
              disabled={!t.available || busy}
              aria-pressed={applied.has(t.id)}
              title={t.unavailableReason ?? t.title}
              onclick={() => preview(t)}
            >
              <!-- No fallback glyph. A tool with no icon shows no icon,
                   which is a gap; a tool with a middle dot looks like a
                   deliberate mark meaning nothing, which is worse and is
                   what `image-ocr` looked like. -->
              <span class="cmd-icon" aria-hidden="true">
                {#if ICONS[t.id]}<Icon name={ICONS[t.id]} size={16} />{/if}
              </span>
              <span class="cmd-text">
                <span class="cmd-title">{t.title}</span>
                <!--
                  NO "APPLIED" LABEL.

                  Selection is shown by the tool looking selected — the lit
                  well, the accent edge, `aria-pressed` for anyone not looking
                  at it. A word underneath was a second, quieter statement of
                  the same fact, and it changed the height of the row when it
                  appeared, so turning a tool on nudged every tool below it.

                  "preview only" stays: that is not the selection state, it is
                  a limit of the tool, and a lit button whose Save will refuse
                  has to say so where the button is.
                -->
                {#if !t.available}
                  <span class="cmd-why">{t.unavailableReason}</span>
                {:else if applied.has(t.id) && t.previewOnly}
                  <span class="cmd-state">preview only</span>
                {/if}
              </span>
            </button>

            {#if current.id === t.id && applied.has(t.id)}
              {@render toolSettings()}
            {/if}
          </li>
        {/each}
      </ul>
      {/if}

    </aside>

    <!-- The document, and whatever the lit tool needs to be told.

         SETTINGS SIT ABOVE THE STAGE, NOT IN THE RAIL. They were in the rail's
         216 px column, under the tool list: a page-order field, a signature
         library and a set of choice controls all folded into the width of a
         button strip, which is why the reorder settings grew a horizontal
         scrollbar and the signature list had no room to show a signature. The
         rail is for CHOOSING an operation; configuring one is work on the
         document, so it belongs over the document, at the document's width. -->
    <div class="work">
      {#if reviewingInput}<OutputReview embedded path={reviewingInput} onclose={() => reviewingInput = null}/>{/if}
  {#if reviewing && savedOutput}
    {#if kind === "audio"}<AudioResults embedded results={runResults} paths={activePaths} durations={Object.fromEntries(activePaths.map(path => [path, factsFor(path)?.split(" · ")[0] ?? ""]))} onclose={() => reviewing = false}/>
    {:else}<OutputReview embedded path={savedOutput} original={sourcePath} onclose={() => reviewing = false}/>{/if}
  {/if}

      <!-- ---------------------------------------------------------------- -->
      <div class="stage" class:busy class:ring={busy && kind !== "audio"} class:running={busy && kind !== "audio"} class:sweeping={busy && kind !== "audio"} class:empty={noFile} inert={busy}>
      <!-- Batch keeps its own empty state: a list with an "add" button, not
           the single-file drop card, because the gesture there is "choose
           several" and the card says "open a file". -->
      {#if noFile && !(kind === "image" && batchMode)}
        <!-- The empty state IS the stage, not a banner above one. A stand-in
             preview underneath an "open a file" prompt reads as the user's
             file, which it is not. -->
        <button type="button" class="stage-drop" onclick={openFile}>
          <span class="drop-glyph" aria-hidden="true"><Icon name="upload" size={30}/></span>
          <span class="drop-title">Open {article} {tool.category.toLowerCase()} file</span>
          <span class="drop-hint">Click to choose, or drop one on the window</span>
        </button>
      {:else if (kind === "image" && batchMode) || current.id === "pdf-compress"}
        <!--
          BATCH IS A LIST. Same shape as the recordings list and the home
          screen: a filename, and a way to drop it. No preview, because there
          is no single picture to preview and rendering forty would cost a
          model run each to answer a question nobody asked.
        -->
        <div class="batch">
          <!-- THE SAME ROW THE HOME SCREEN USES.

               This list was built here, separately, and had drifted into a
               different thing: its own thumbnail at its own size, its own
               expand-on-click, its own name and bin. So the app showed a list
               of files two ways depending on which screen you were on, and the
               one here was the worse of the two -- its thumbnails are cropped
               square, so a landscape photograph arrived apparently zoomed in,
               and they were fetched by a second loader with no concurrency
               limit of its own.

               `FileRow` already solves all of that: it letterboxes the
               preview, it caps how many renders run at once across the whole
               screen, and it is the row people have already learnt on the home
               screen. `bare` drops the two parts that belong to a conversion
               and not to a tool -- the FROM -> TO control and the per-row menu.

               A row cannot render before its probe arrives, and the probes are
               already being fetched for the size caption. -->
          <div class="batch-list" role="list" aria-label="Files">
            {#each activePaths as path (path)}
              {@const probe = probedFor[path]}
              {#if probe}
                <div role="listitem">
                  <FileRow
                    {probe}
                    bare
                    resultLabel={aboutSize ? resultsFor[path] ?? estimates[path] ?? (estimateBusy ? "Checking compressed size…" : null) : null}
                    options={[]}
                    selected={-1}
                    onselect={() => {}}
                    checked={!excludedPaths.has(path)}
                    onchecked={(checked) => {
                      const next = new Set(excludedPaths);
                      if (checked) next.delete(path); else next.add(path);
                      excludedPaths = next;
                    }}
                    onpreview={() => reviewingInput = path}
                    onremove={() => removeAudio(path)}
                  />
                </div>
              {/if}
            {/each}
          </div>

          <!-- INLINE WITH THE BIN, not a full-width button underneath the
               floating one. The add button used to span the stage and the
               clear-all bin sat on top of its right-hand end. The right end is
               rounded to the bin's own radius so the pair reads as one
               control. -->
          <div class="batch-actions">
            <button type="button" class="batch-add" onclick={openFile}>
              <Icon name="upload" size={16} />
              {activePaths.length === 0 ? "Choose files" : "Add more files"}
            </button>
            {#if activePaths.length > 0}
              <button
                type="button"
                class="batch-clear"
                aria-label="Remove every file from this list"
                title="Remove every file"
                onclick={clearFile}
              >
                <Icon name="trash-2" size={15} />
              </button>
            {/if}
          </div>
        </div>
      {:else if kind === "image"}
        <!--
          THE CHECKERBOARD IS ALWAYS THERE, not only after a background is
          removed. Its job is to say "this is not part of the picture" — which
          is exactly as true of the space around a portrait on a wide stage as
          it is of a transparent cut-out. Painting it only for the cut-out case
          left an image floating on an opaque panel, where nothing told you
          which grey was the photo and which was the app.
        -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          bind:this={canvasEl}
          class="canvas checker"
          class:panning
          class:grabbable={!picking && zoom !== 1}
          onwheel={wheelZoom}
          onpointerdown={startPan}
          onpointermove={movePan}
          onpointerup={endPan}
          onpointercancel={endPan}
        >
          {#if shotError}
            <p class="stage-error" role="alert">{shotError}</p>
          {:else if shot}
            <!-- A PNG the backend rendered in a confined worker. The webview
                 decodes our output, never the user's original file. -->
            {#if picking}
              <!-- A real control, so the pointer and the keyboard both reach
                   it: press and drag aims and reads continuously, arrows aim
                   and Enter reads. -->
              <button
                type="button"
                class="pick-target"
                class:sampling
                onpointerdown={startSampling}
                onpointermove={moveSampling}
                onpointerup={endSampling}
                onpointercancel={endSampling}
                onkeydown={aim}
                aria-label="Pick a colour: hold and drag to sample, or arrow keys to aim and Enter to read"
              >
                <!--
                  THE ZOOM VARIABLES BELONG HERE TOO.

                  This branch rendered a bare `<img>` while the non-picking one
                  set `--zoom`, `--pan-x` and `--pan-y`. `.canvas img` transforms
                  by all three, so with none of them set every fallback resolved
                  to identity: the zoom buttons, the wheel, the pinch and Fit all
                  ran and moved nothing, for the one tool where aiming at a
                  specific pixel is the entire task.

                  The crosshair is a sibling of the image inside `.shot`, which
                  is sized to the IMAGE. It used to sit inside the button, which
                  is taller whenever the picture letterboxes — so a fraction of
                  the button was not the same point as a fraction of the image,
                  and the marker rode consistently high.
                -->
                <!--
                  THE ZOOM VARIABLES SIT ON THE WRAPPER, NOT ON THE IMAGE.

                  Custom properties inherit, so `.canvas img` still resolves
                  all three and transforms exactly as before -- but now the
                  CROSSHAIR can read them too, and it has to. The marker is a
                  sibling of the image, positioned as a fraction of `.shot`,
                  which is not transformed; the image is. At any zoom but 1 the
                  two disagreed, so the ring was drawn away from the pixel
                  being sampled.

                  `aimAt` was never wrong -- it measures the image's own
                  rectangle, which already includes the transform, so the
                  COLOUR was always the colour under the pointer. Only the
                  marker lied, which is worse: the swatch was right and looked
                  wrong, so the tool read as broken while it was working.
                -->
                <span
                  class="shot"
                  style:--ar={`${shot.width} / ${shot.height}`}
                  style:--zoom={zoom}
                  style:--pan-x={`${pan.x}px`}
                  style:--pan-y={`${pan.y}px`}
                >
                  <img
                    bind:this={stageImg}
                    src={shot.dataUri}
                    alt={`Preview of ${fileName}`}
                    draggable="false"
                  />
                  <span class="crosshair" style:--cx={cross.x} style:--cy={cross.y}></span>
                </span>
              </button>

              {#if magnifier && sampling && stageImg}
                <!--
                  THE LOUPE. The pixels around the crosshair, blown up with
                  hard edges, so "the right pixel" is a thing you can actually
                  see rather than aim at and hope.

                  Drawn as a background-image of the SAME preview PNG, scaled
                  and offset — no second decode, no canvas per frame, and it
                  tracks the pointer for free.
                -->
                <span
                  class="loupe"
                  aria-hidden="true"
                  style:--lx={`${loupeAt.x}px`}
                  style:--ly={`${loupeAt.y}px`}
                  style:--loupe-size={`${LOUPE_SIZE}px`}
                  style:background-image={`url(${shot.dataUri})`}
                  style:background-size={`${(stageImg.naturalWidth * LOUPE_SIZE) / LOUPE_PIXELS}px ${(stageImg.naturalHeight * LOUPE_SIZE) / LOUPE_PIXELS}px`}
                  style:background-position={`${LOUPE_SIZE / 2 - (cross.x * stageImg.naturalWidth + 0.5) * (LOUPE_SIZE / LOUPE_PIXELS)}px ${LOUPE_SIZE / 2 - (cross.y * stageImg.naturalHeight + 0.5) * (LOUPE_SIZE / LOUPE_PIXELS)}px`}
                >
                  <span class="loupe-cell" style:--cell={`${LOUPE_SIZE / LOUPE_PIXELS}px`}></span>
                </span>
              {/if}
            {:else}
              <!-- ORIGINAL ASPECT RATIO, ALWAYS. `object-fit: contain` with a
                   max-width and max-height, never `width: 100%`: the stage is
                   whatever shape the window is, and a photograph stretched to
                   it is a photograph the user cannot judge. The space left
                   over is the checkerboard's job, not the image's. -->
              <img
                bind:this={stageImg}
                src={visible?.dataUri}
                alt={`Preview of ${fileName}`}
                draggable="false"
                style:--zoom={zoom}
                style:--pan-x={`${pan.x}px`}
                style:--pan-y={`${pan.y}px`}
              />
            {/if}
          {:else if loadingShot}
            <p class="stage-empty">Rendering…</p>
          {/if}
        </div>
        <!--
          THE VIEWPORT PANEL, under the image.

          Zoom and pan are how anyone checks a result: whether the matte cut
          into someone's hair, whether upscaling invented detail or smeared it.
          A stage that only ever shows "the whole thing, scaled to fit" can be
          looked at and not examined.

          Compare is HELD, not toggled. Holding is the gesture every image
          editor uses for this, and it means the comparison cannot be left on
          by accident — let go and you are looking at the result again.
        -->
        <div class="viewport" role="group" aria-label="View">
          <button
            type="button"
            class="quiet vp"
            onclick={() => zoomStep(1 / 1.25)}
            disabled={zoom <= ZOOM_MIN}
            aria-label="Zoom out">−</button
          >
          <button type="button" class="quiet vp num" onclick={fitView} title="Fit to the stage">
            {zoomPercent}%
          </button>
          <button
            type="button"
            class="quiet vp"
            onclick={() => zoomStep(1.25)}
            disabled={zoom >= ZOOM_MAX}
            aria-label="Zoom in">+</button
          >
          <button type="button" class="quiet vp" onclick={() => { zoom = 1; pan = { x: 0, y: 0 }; }}>
            Fit
          </button>
          {#if canCompare}
            <button
              type="button"
              class="quiet vp compare"
              class:held={comparing}
              onpointerdown={() => (comparing = true)}
              onpointerup={() => (comparing = false)}
              onpointerleave={() => (comparing = false)}
              onpointercancel={() => (comparing = false)}
              onkeydown={(e) => {
                if (e.key === " " || e.key === "Enter") comparing = true;
              }}
              onkeyup={() => (comparing = false)}
              title="Hold to see the original"
            >
              {comparing ? "Original" : "Before / after"}
            </button>
          {/if}
        </div>

        <div class="stage-note">
          {#if picking}
            {#if swatch}
              <span class="swatch" style:--picked={swatch.hex}></span>
              <button type="button" class="colour-value mono" aria-label="Copy HEX" onclick={() => void copyColour(swatch!.hex)}>{copiedColour === swatch.hex ? "Copied ✓" : swatch.hex}</button>
              <button type="button" class="colour-value" aria-label="Copy RGB" onclick={() => void copyColour(swatch!.rgb)}>{copiedColour === swatch.rgb ? "Copied ✓" : swatch.rgb}</button>
            {:else}
              Click anywhere in the image to read a colour.
            {/if}
          {:else if shot}
            {#if aboutSize && sizeLine}
              {sizeLine}
            {:else if bgRemoved}
              Background removed, now transparent
            {:else}
              {shot.width} × {shot.height}
            {/if}
          {:else}
            &nbsp;
          {/if}
        </div>
      {:else if kind === "pdf"}
        {#if signing}
          <!--
            ONE SCROLLING COLUMN, chained like a PDF viewer — not the
            side-by-side board.

            The layout follows the mode because the task does. Reordering is
            about the sequence, so pages go side by side where the sequence is
            visible; signing is about a spot on a particular page, so pages go
            one under another at a size you can aim at.
          -->
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div class="sign-stage">
          <div class="column" class:placing style:--sign-zoom={signZoom} onpointermove={trackGhost}>
            {#each order as slot (board[slot]?.key ?? slot)}
              {@const page = slot + 1}
              <!-- svelte-ignore a11y_click_events_have_key_events -->
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <div
                class="sheet-wrap"
                data-sheet
                onclick={(e) => placeOnPage(e, page)}
                onpointermove={movePlacement}
                onpointerup={releasePlacement}
                onpointercancel={releasePlacement}
              >
                {#if sheets[page] ?? thumbs[page]}
                  <!-- The full render when it has arrived, the card thumbnail
                       until then: a soft page that sharpens beats an empty
                       rectangle, and the placement is in fractions of the page
                       so it survives the swap. -->
                  <img
                    class="sheet"
                    src={sheets[page] ?? thumbs[page]}
                    alt={`Page ${page}`}
                    draggable="false"
                  />
                {:else}
                  <div class="sheet placeholder"></div>
                {/if}
                <span class="sheet-no">{page}</span>

                {#each placedSignatures as stamp, index}
                  {#if stamp.placement.page === page}
                    <button type="button" class="placed" style:--px={stamp.placement.x} style:--py={stamp.placement.y} style:--pw={stamp.placement.width} aria-label={`Edit placed ${stamp.signature.name}`} onclick={(e) => { e.stopPropagation(); selectPlacement(index); }}>
                      <img src={stamp.signature.dataUri} alt="" draggable="false" />
                    </button>
                  {/if}
                {/each}
                {#if placement && placement.page === page && signature}
                  <!-- The placed signature. Positioned by fraction, so it
                       stays where it was put when the column is resized. -->
                  <!-- svelte-ignore a11y_no_static_element_interactions -->
                  <div
                    class="placed"
                    class:moving={movingPlacement}
                    style:--px={placement.x}
                    style:--py={placement.y}
                    style:--pw={placement.width}
                    onpointerdown={(e) => grabPlacement(e, null)}
                  >
                    <img src={signature.dataUri} alt={`${signature.name}, placed`} draggable="false" />
                    <!-- FOUR CORNERS, NOT ONE.

                         There was a single dot at the bottom right, and it was
                         invisible: it painted `var(--accent)`, a token that
                         was never defined, so the declaration was dropped and
                         the dot had no fill at all. "Resizing does not seem to
                         work" was mostly "there is nothing here that looks
                         like a thing you can pull".

                         Four marks read as a selected object at a glance,
                         which one does not, and all four resize -- the anchor
                         is the centre, so the only difference between them is
                         which way is outward. -->
                    {#each ["nw", "ne", "sw", "se"] as const as corner (corner)}
                      <!-- svelte-ignore a11y_no_static_element_interactions -->
                      <span
                        class="handle {corner}"
                        aria-hidden="true"
                        onpointerdown={(e) => grabPlacement(e, corner)}
                      ></span>
                    {/each}
                  </div>
                {/if}
              </div>
            {/each}
          </div>

          <!-- ZOOM BELONGS ON THE THING BEING ZOOMED. It was a four-button
               segment in a settings band above the stage; overlaid on the
               column it is where the eye already is, takes no layout, and does
               not push the page down to make room for itself. -->

          </div>

          {#if placing && signature}
            <!-- The ghost: the signature under the cursor, until a page takes
                 it. `pointer-events: none` so it never intercepts the click
                 that is meant for the page underneath. -->
            <img
              class="ghost"
              src={signature.dataUri}
              alt=""
              aria-hidden="true"
              style:--gx={`${ghost.x}px`}
              style:--gy={`${ghost.y}px`}
            />
          {/if}
        {:else if splitParts.length > 1 && !documentPreview}
          <!-- WHAT THE SPLIT WOULD WRITE, one card per file.

               The same board merge uses, for the opposite operation: merge
               asks which documents become one, this shows which documents one
               becomes. Each card carries its first page, because that is what
               tells two parts of the same report apart. -->
          <ol class="board" aria-label="The documents this split would write">
            {#each splitParts as part, i (part.from)}
              <li class="card split-card">
                <input class="part-name field" aria-label={`Output name for part ${i+1}`} placeholder={`${fileName.replace(/\.pdf$/i,"")}-pages-${part.from}-${part.to}.pdf`} value={splitNames[i]??""} oninput={e=>splitNames={...splitNames,[i]:e.currentTarget.value}} />
                <span class="card-grip still">
                  <span class="card-no">{i + 1}</span>
                  {#if thumbs[part.from]}
                    <img class="card-thumb" src={thumbs[part.from]} alt="" />
                  {:else}
                    <span class="card-thumb placeholder"></span>
                  {/if}
                  <span class="card-label">
                    {part.from === part.to
                      ? `page ${part.from}`
                      : `pages ${part.from}\u2013${part.to}`}
                  </span>
                </span>
                <div class="document-actions split-actions">
                  <button type="button" aria-label={`Preview part ${i + 1}`} onclick={() => {
                    readerIsStage = false;
                    void previewDocument(0, part);
                  }}><Icon name="eye" size={14} /></button>
                </div>
              </li>
            {/each}
          </ol>
        {:else if documentPreview}
          <div class="continuous-preview">{#if !readerIsStage}<div class="preview-heading"><strong>Preview</strong><button type="button" class="quiet" onclick={closeDocumentPreview}>Close</button></div>{/if}<PdfViewer path={documentPreview.path} from={documentPreview.from} to={documentPreview.to} operations={documentPreview.path === sourcePath ? pagePreviewOps : undefined}/></div>
        {:else if merged}
          <!-- STAGE TWO: every page of every document, as one sequence. The
               documents are gone as units; what is left is pages, which is
               what the output is.

               **REARRANGING IS A DIFFERENT TOOL, AND THIS IS NOT IT.** This
               board was always draggable, with a bin on every card, so
               choosing Merge and pressing Continue handed the user a
               page-rearranging interface they had not asked for -- and one
               whose edits Save did not even send, because `saveJob` only puts
               an order in the request when `pdf-reorder` is lit.
               So it offered an operation, performed it on screen, and dropped
               it silently.

               Lit alongside Merge, reorder makes the board editable and the
               order is sent. On its own, Merge shows what it is about to
               write. -->
          <ol class="board" aria-label="Every page, in output order">
            {#each order as slot, position (slot)}
              {@const item = mergedLabels[slot]}
              {#if item}<li
                class="card"
                class:dragging={allPages && dragFrom === position}
                class:over={allPages && dragOver === position && dragFrom !== position}
                style:--drag-x={`${dragOffset.x}px`}
                style:--drag-y={`${dragOffset.y}px`}
              >
                <button
                  type="button"
                  class="card-grip"
                  class:still={!allPages}
                  data-slot={position}
                  disabled={!allPages}
                  onpointerdown={(e) => allPages && grab(e, position)}
                  onpointermove={track}
                  onpointerup={release}
                  onpointercancel={cancelDrag}
                >
                  <span class="card-no">{position + 1}</span>
                  {#if mergedThumbs[slot + 1]}
                    <img class="card-thumb" src={mergedThumbs[slot + 1]} alt="" />
                  {:else}
                    <span class="card-thumb placeholder"></span>
                  {/if}
                  <span class="card-label">{item.doc} · p{item.label}</span>
                </button>
                {#if allPages && order.length > 1}<button type="button" class="card-bin" aria-label={`Remove page ${position + 1}`} onclick={() => removeAt(position)}>×</button>{/if}
              </li>{/if}
            {/each}
          </ol>
        <!-- THE BOARD, FOR PAGES OR FOR DOCUMENTS.

             The condition was `allPages` alone, which is
             `applied.has("pdf-reorder")` -- so the DOCUMENT board that `board`
             builds for a merge only ever appeared when reorder happened to be
             lit as well. Merge on its own fell through to the single-page
             pager below: a rendered page with Previous and Next under it, and
             nothing on screen about merging or any way to add a second
             document. That is the merge interface that was reported missing.

             `board` has always known how to build both. Only this line
             decided whether anyone saw it. -->
        {:else if allPages || arrangingDocuments}
          <!-- The board. Cards carry the order the output will have; dragging
               one moves it and closes the gap behind it. Keyboard users move a
               card with the arrow keys, because drag alone is not an interface
               everyone can use. -->
          <ol class="board" class:page-board={!arrangingDocuments} aria-label="Pages in output order">
            {#each order as slot, position (board[slot]?.key ?? slot)}
              {@const card = board[slot]}
              {#if card}
                <li
                  class="card"
                  class:editable-page={current.id === "pdf-reorder"}
                  class:dragging={dragFrom === position}
                  class:over={dragOver === position && dragFrom !== position}
                  style:--drag-x={`${dragOffset.x}px`}
                  style:--drag-y={`${dragOffset.y}px`}
                >
                  <button
                    type="button"
                    class="card-grip"
                    data-slot={position}
                    aria-label={`${card.label}, position ${position + 1} of ${order.length}. Left and right arrows move it.`}
                    onpointerdown={(e) => grab(e, position)}
                    onpointermove={track}
                    onpointerup={release}
                    onpointercancel={cancelDrag}
                    onkeydown={(e) => {
                      if (e.key === "ArrowLeft") {
                        e.preventDefault();
                        moveCard(position, position - 1);
                      } else if (e.key === "ArrowRight") {
                        e.preventDefault();
                        moveCard(position, position + 1);
                      }
                    }}
                  >
                    <span class="card-no">{position + 1}</span>
                    <!-- THIS PAGE, not "the page the stage happens to be
                         showing". Every card drew `shot.dataUri`, so a
                         twelve-page document showed twelve copies of page one
                         and the board could not be used for the one thing it
                         is for. -->
                    {#if arrangingDocuments && mergeThumbs[activePaths[card.index] ?? ""]}
                      <img class="card-thumb" src={mergeThumbs[activePaths[card.index] ?? ""]} alt="" />
                    {:else if arrangingDocuments}
                      <span class="card-thumb placeholder"></span>
                    {:else if thumbs[card.index + 1]}
                      <img class="card-thumb" src={thumbs[card.index + 1]} alt="" />
                    {:else}
                      <span class="card-thumb placeholder"></span>
                    {/if}
                    <span class="card-label">{card.label}</span>
                  </button>

                  {#if arrangingDocuments}
                    <div class="document-actions">
                      <!-- LUCIDE, NOT HAND-DRAWN. These were two paths written
                           here on a 16x16 grid at two different stroke widths,
                           beside a rail that has just been moved off exactly
                           that. `Icon.svelte`'s header records that this went
                           wrong twice; these were the last two. -->
                      <button type="button" aria-label={`Preview ${card.label}`} title="Preview all pages" onclick={() => {
                        // THE USER'S OWN COPY, not the stage's: it stays until
                        // they close it.
                        readerIsStage = false;
                        void previewDocument(card.index);
                      }}>
                        <Icon name="eye" size={14} />
                      </button>
                      <button type="button" aria-label={`Remove ${card.label}`} title="Remove document" onclick={() => removeDocument(card.index)}>
                        <Icon name="trash-2" size={14} />
                      </button>
                    </div>
                  {/if}

                  {#if current.id === "pdf-reorder"}
                    <!-- MODIFY, beside the bin, on the page it acts on.
                         Rotate and Crop were separate rail entries that asked
                         which pages to act on in a text field, while a board
                         showing those very pages sat beside them. -->
                    <button
                      type="button"
                      class="card-modify"
                      class:edited={turnFor(position) !== 0 ||
                        cropFor(position).some((v) => v !== 0)}
                      aria-label={`Modify ${card.label}`}
                      aria-expanded={modifying === position}
                      title="Turn or crop this page"
                      onclick={() => void openModify(position)}
                    >
                      <Icon name="scaling" size={13} />
                    </button>
                  {/if}
                  {#if current.id === "pdf-reorder" && order.length > 1}
                    <!-- Removal lives on the card in drag mode, because that is
                         where the page is. It drops the page from the OUTPUT;
                         the document on disk is not touched, and Reset order
                         brings it back. -->
                    <button
                      type="button"
                      class="card-bin"
                      aria-label={`Remove ${card.label} from the output`}
                      title="Remove this page"
                      onclick={() => removeAt(position)}
                    >
                      <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
                        <path
                          d="M6.5 2.5h3M2.5 4.5h11M4.5 4.5l.6 8a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8M6.8 7v4M9.2 7v4"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="1.2"
                          stroke-linecap="round"
                          stroke-linejoin="round"
                        />
                      </svg>
                    </button>
                  {/if}
                </li>
              {/if}
            {/each}

            {#if arrangingDocuments}
              <!-- Adding another document is part of the board, not a menu. -->
              <li class="card add">
                <button type="button" class="card-grip" onclick={openFile}>
                  <span class="add-plus" aria-hidden="true">+</span>
                  <span class="card-label">Add PDF</span>
                </button>
              </li>
            {/if}
          </ol>
        {:else if shotError}
          <div class="pages"><p class="stage-error" role="alert">{shotError}</p></div>
        {:else if shot}
          <div class="pages">
            <img class="sheet" src={shot.dataUri} alt={`Page ${shot.page} of ${pdfPages}`} />
          </div>
        {:else if loadingShot}
          <div class="pages"><p class="stage-empty">Rendering…</p></div>
        {/if}

        {#if modifying !== null && current.id === "pdf-reorder"}
          <!--
            UNDER THE BOARD, NOT OVER THE CARD.

            It was a popover hanging off the card it belonged to, which is the
            obvious place and the wrong one: a card is 78 px wide, the controls
            need about 190, and the board clips -- so opening the panel on a
            page in the last column cut it in half and truncated its own label.
            Anchoring it to the other edge only moves the problem to the first
            column.

            One panel in one place also means the controls do not jump when a
            different page is picked. The card says which page it is about: its
            icon stays lit while the panel is open, and lit in the accent once
            the page has actually been changed.
          -->
          {@const at = modifying}
          {@const crop = cropFor(at)}
          <!-- THE PAGE, THE SIZE OF THE STAGE, WITH ITS EDGES TO DRAG.

               This was four range sliders in a 190 px panel while the page
               they applied to sat elsewhere at 78 px. Cropping is a spatial
               decision -- this much off that side -- and it was being asked as
               four numbers about something too small to judge. Nobody crops a
               photograph that way.

               The state is the same four percentages; only the instrument
               changed. Handles are buttons so they can be tabbed to and moved
               with the arrow keys: a drag is not an interface everyone has. -->
          <div class="crop-editor" role="group" aria-label={`Modify page ${at + 1}`}>
            <div
              class="crop-page"
              style:--l={`${crop[0]}%`}
              style:--b={`${crop[1]}%`}
              style:--r={`${crop[2]}%`}
              style:--t={`${crop[3]}%`}
              style:--turn={`${turnFor(at)}deg`}
              style:--page-ratio={modifyShot ? modifyShot.width / modifyShot.height : 0.707}
              style:--turn-scale={turnFor(at) % 180 && modifyShot ? Math.min(modifyShot.width / modifyShot.height, modifyShot.height / modifyShot.width) : 1}
            >
              {#if modifyShot}
                <img src={modifyShot.dataUri} alt={`Page ${at + 1}`} />
              {:else}
                {#if modifyError}<p class="stage-error" role="alert">{modifyError}</p>{:else}<p class="stage-empty">Rendering page {at + 1}…</p>{/if}
              {/if}
              <!-- What survives the crop, drawn over what does not. -->
              <span class="crop-keep" aria-hidden="true"></span>
              {#each MODIFY_EDGES as edge, i (edge)}
                <button
                  type="button"
                  class={`crop-handle ${edge}`} disabled={!modifyShot}
                  aria-label={`Crop ${edge} of page ${at + 1}: ${crop[i]} per cent`}
                  onpointerdown={(e) => startCropDrag(e, i, at)}
                  onpointermove={(e) => moveCropDrag(e, at)}
                  onpointerup={endCropDrag}
                  onpointercancel={endCropDrag}
                  onkeydown={(e) => nudgeCrop(e, i, at)}
                ></button>
              {/each}
              {#each [[0, 3], [2, 3], [0, 1], [2, 1]] as [horizontal, vertical]}
                <button type="button" class="crop-corner" disabled={!modifyShot}
                  aria-label={`Crop ${MODIFY_EDGES[vertical]} ${MODIFY_EDGES[horizontal]} corner`}
                  style:left={horizontal === 0 ? `calc(${crop[0]}% - 7px)` : undefined}
                  style:right={horizontal === 2 ? `calc(${crop[2]}% - 7px)` : undefined}
                  style:top={vertical === 3 ? `calc(${crop[3]}% - 7px)` : undefined}
                  style:bottom={vertical === 1 ? `calc(${crop[1]}% - 7px)` : undefined}
                  onpointerdown={(e) => startCropDrag(e, horizontal, at, vertical)}
                  onpointermove={(e) => moveCropDrag(e, at)}
                  onpointerup={endCropDrag} onpointercancel={endCropDrag}
                  onkeydown={(e) => { nudgeCrop(e, horizontal, at); nudgeCrop(e, vertical, at); }}
                ></button>
              {/each}
            </div>

            <div class="crop-actions">
              <button type="button" class="quiet" disabled={!modifyShot} aria-label="Rotate page 90 degrees" onclick={() => turnPage(at)}><Icon name="refresh-cw" size={16} /></button>
              <button type="button" class="quiet" disabled={!modifyShot} onclick={() => void applyPageEditsToAll()}>Apply to all</button>
              <button type="button" class="quiet" onclick={() => void finishModify()}>Done</button>
              <button type="button" class="quiet" onclick={() => { rememberEdit(); const page = (order[at] ?? at) + 1; pageTurns = { ...pageTurns, [page]: 0 }; pageCrops = { ...pageCrops, [page]: [0,0,0,0] }; }}>Reset</button>
            </div>
          </div>
        {/if}

        {#if !documentPreview}<div class="stage-note pager">
          {#if signing}
            <span>
              {order.length} page{order.length === 1 ? "" : "s"}
              {placement ? `· signature on page ${placement.page}` : "· nothing placed yet"}
            </span>
          <div class="zoomer seg" role="group" aria-label="Zoom">
            <button type="button" aria-label="Zoom out" disabled={signZoom <= 1} onclick={() => signZoom = Math.max(1, signZoom - 0.5)}>−</button>
            <span>{Math.round(signZoom * 100)}%</span>
            <button type="button" aria-label="Zoom in" disabled={signZoom >= 3} onclick={() => signZoom = Math.min(3, signZoom + 0.5)}>+</button>
          </div>
          {:else if splitParts.length > 1}
            <span>{splitParts.length} documents</span>
          {:else if merged}
            <span>
              {mergedLabels.length} page{mergedLabels.length === 1 ? "" : "s"} from
              {mergedPages.length} document{mergedPages.length === 1 ? "" : "s"}
            </span>
            <button type="button" class="quiet" onclick={unmerge}>Back to documents</button>
          <!-- Same condition as the stage above, and for the same reason:
               this branch already knows how to say "3 documents" -- it checks
               `arrangingDocuments` in its own text -- and was never reached
               for a merge, because reaching it required reorder to be lit. So
               a board of documents sat under a caption reading "Page 1 of 6",
               with Previous and Next beside it. -->
          {:else if allPages || arrangingDocuments}
            <span>
              {order.length}
              {arrangingDocuments ? "document" : "page"}{order.length === 1 ? "" : "s"}
              {reordered ? "· reordered" : "· original order"}
            </span>
            <!-- The step forward used to live here, as a quiet button in the
                 board's own status bar, next to "3 documents · original
                 order" -- where it read as part of the caption rather than as
                 the way on, while the real primary action in the footer was a
                 Save that should not have been offered yet. It is the
                 footer's primary button now. -->
            {#if reordered || Object.keys(pageTurns).length > 0 || Object.keys(pageCrops).length > 0}
              <!-- PUT IT BACK. A board is a sequence of small irreversible
                   edits -- a drag, a bin, a shortcut that drops half the
                   pages -- and the only way back was to close the document and
                   open it again, which loses everything else with it. -->
              <button type="button" class="quiet reset" onclick={resetOrder}>
                <Icon name="refresh-cw" size={14} />
                Reset
              </button>
            {/if}
          {:else}
            <button
              type="button"
              class="quiet"
              onclick={() => turnTo(pdfPage - 1)}
              disabled={pdfPage <= 1 || loadingShot}>Previous</button
            >
            <span>Page {pdfPage} of {pdfPages}</span>
            {#if pageShape}<span class="shape">{pageShape}</span>{/if}
            <button
              type="button"
              class="quiet"
              onclick={() => turnTo(pdfPage + 1)}
              disabled={pdfPage >= pdfPages || loadingShot}>Next</button
            >
          {/if}
        </div>{/if}
      {:else}
        <!--
          AUDIO IS A LIST, NOT A STAGE.

          One recording used to fill the middle of the window as a single large
          centred waveform — the treatment a photo editor gives a photograph,
          applied to a thing that has no picture. Worse, it was a treatment for
          exactly ONE file: choosing five recordings drew the first one and said
          nothing about the other four, which were still converted on Save. A
          list shows what will be worked on, which is the only thing this screen
          has to get right.

          Same shape as the conversion screen's file list, deliberately: a
          filename, what is about to happen to it, and a way to drop it.
        -->
        <!-- THE RING GOES AROUND THE LIST, not across the top of it.

             A bar above the files says "something is happening somewhere"; a
             border that fills says "this, the thing you are looking at". It
             also costs no layout: it is drawn on the container's own edge, so
             nothing moves when it appears. -->
        <div class="audio-container">
        <div class="audio-list" role="list" aria-label="Recordings">
          {#if activePaths.length > 1}<span class="note">Files are processed one at a time.</span>{/if}
          {#each activePaths as path (path)}
            {@const active = busy && job.progress.some(p => p.index === activePaths.indexOf(path) && p.phase === "started")}
            <div class="audio-row" class:ring={active} class:running={active} class:sweeping={active} role="listitem" aria-label={active ? `${baseName(path)}: processing` : baseName(path)}>
              <div class="audio-head">
                <span class="audio-name" title={path}>{baseName(path)}</span>
                <!-- THE SELECTED TOOL IS NOT REPEATED PER ROW. Printing
                     "Transcribe to text" beside every filename restated what
                     the rail already shows lit, once per file, next to a
                     control that does something else entirely.

                     What survives is the EMPTY case: with nothing selected,
                     Save is disabled, and this is the only thing on screen
                     that says why. -->
                {#if !(current && applied.has(current.id))}
                  <span class="audio-op">No operation selected</span>
                {/if}
                {#if factsFor(path)}
                  <span class="audio-facts">{factsFor(path)}</span>
                {/if}
                <button
                  type="button"
                  class="audio-remove"
                  aria-label={`Remove ${baseName(path)}`}
                  title="Remove from this list"
                  disabled={busy}
                  onclick={() => removeAudio(path)}
                >
                  <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
                    <path
                      d="M6.5 2.5h3M2.5 4.5h11M4.5 4.5l.6 8a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8M6.8 7v4M9.2 7v4"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.2"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                    />
                  </svg>
                </button>
              </div>
              <WavePlayer {path} peaks={peaksFor[path] ?? []} />
              {#if runResults[activePaths.indexOf(path)]?.success}
                {@const result = runResults[activePaths.indexOf(path)]!}
                <div class="audio-result-actions">
                  <button type="button" class="quiet" aria-label={`Preview ${baseName(path)}`} onclick={() => reviewing = true}><Icon name="eye" size={15}/></button>
                  <button type="button" class="quiet" aria-label={`Show output for ${baseName(path)} in folder`} onclick={() => void revealOutput(result.outputPath)}><Icon name="folder-open" size={15}/></button>
                </div>
              {/if}
            </div>
          {/each}

          <!-- Same "+" tile every other upload uses. -->
          <button type="button" class="audio-add" onclick={openFile} disabled={busy}>
            <span class="audio-plus" aria-hidden="true">+</span>
            <span>Add audio</span>
          </button>
        </div>
        </div>
      {/if}

      <!-- NO "WORKING…" LABEL.

           It was a line of text that appeared above the list while a run was
           going, said one word, and pushed every row down by its own height as
           it arrived and again as it left -- so the thing you were looking at
           moved twice per run, to say something already on screen.

           Nothing is lost. The ring that runs around the list above says the
           same thing in the same moment, and says it about the thing being
           looked at rather than above it; it already carries
           `role="progressbar"` and `aria-label="Working"`, so the
           announcement this label made was the second of two. -->

      <!-- NOT IN BATCH EITHER, for the reason given for audio: the stage is a
           list, every row carries its own bin, and this one is labelled after
           the first file while clearing all of them. It also landed directly
           on top of the add button. -->
      {#if !noFile && kind !== "audio" && !(kind === "image" && batchMode) && current.id !== "pdf-compress"}
        <!-- Bottom-right, over the stage: removes the open file and returns to
             the chooser. It clears the workspace, not the file on disk.

             NOT ON AUDIO. The stage there is a list, every row carries its own
             trashcan, and a floating button labelled after the FIRST file that
             silently clears all of them is the worst of both — it names one
             file and acts on the batch. -->
        <button
          type="button"
          class="discard"
          onclick={clearFile}
          aria-label={`Close ${fileName} and choose another`}
          title="Close this file"
        >
          <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
            <path
              d="M6.5 2.5h3M2.5 4.5h11M4.5 4.5l.6 8a1 1 0 0 0 1 .9h3.8a1 1 0 0 0 1-.9l.6-8M6.8 7v4M9.2 7v4"
              fill="none"
              stroke="currentColor"
              stroke-width="1.2"
              stroke-linecap="round"
              stroke-linejoin="round"
            />
          </svg>
        </button>
      {/if}

      {#if fileDragOver}
        <!-- Over the stage, not over the whole window: the rail stays readable,
             and the overlay lands on the thing the files are about to join. -->
        <DropOverlay
          label={noFile
            ? `Release to open`
            : accumulates
              ? "Release to add to this list"
              : "Release to open this file"}
          sublabel={!noFile && !accumulates ? "This replaces what is open" : ""}
        />
      {/if}
      </div>
    </div>

    <!-- ---------------------------------------------------------------- -->
  </div>

  <!-- Transcript sits under the stage, scrolls on its own. ---------------- -->
  {#if addingSignature}
    <!--
      ADD A SIGNATURE: choose the file, then name it.

      Two steps in that order on purpose. "What should this be called?" is a
      much easier question once the thing is on screen — and seeing it first is
      also the only chance to notice that the PNG has a white box behind it
      instead of transparency, which is the failure this whole feature exists
      to avoid.
    -->
    <div
      class="sheet-modal"
      role="dialog"
      aria-modal="true"
      aria-label="Add a signature"
      tabindex="-1"
      onkeydown={(e) => {
        if (e.key === "Escape") closeSignatureDialog();
      }}
    >
      <div class="modal-card">
        <h3>Add a signature</h3>
        {#if !newSignaturePreview}<SignatureCapture oncapture={png=>{newSignatureSource=png;newSignaturePreview=png;newSignatureName="My signature";}} />{/if}

        {#if newSignaturePreview}
          <!-- On the checkerboard, so a PNG that is not actually transparent
               is obvious here rather than on a contract. -->
          <div class="sig-preview checker">
            <img src={newSignaturePreview} alt="What you chose" />
          </div>
          <label class="modal-field">
            <span>Name it</span>
            <input
              class="field"
              type="text"
              bind:value={newSignatureName}
              placeholder="My signature"
              onkeydown={(e) => {
                if (e.key === "Enter") saveSignature();
              }}
            />
          </label>
        {:else}
          <p class="modal-note">
            Choose a PNG with a transparent background. A scan of a signature
            on white paper will bring the white with it.
          </p>
          <button type="button" class="signature-drop" onclick={chooseSignatureFile}>
            <Icon name="upload" size={28}/>
            <span>Choose a PNG</span><span class="drop-hint">Click to choose, or drop one here</span>
          </button>
        {/if}

        {#if signatureError}<p class="param-error" role="alert">{signatureError}</p>{/if}

        <div class="modal-actions">
          <button type="button" class="quiet" onclick={closeSignatureDialog}>Cancel</button>
          <button
            type="button"
            class="quiet primary"
            disabled={!newSignatureSource || newSignatureName.trim() === ""}
            onclick={saveSignature}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  {/if}

  {#if transcriptOpen}
    <!--
      THE PANEL IS THE IMAGE HALF ONLY.

      OCR works on the one image the stage is showing, so its text belongs
      under that image. Transcription now works on a LIST, and its text sits on
      each row with the recording it came from — a single shared panel could
      only ever show one of them, with nothing saying which file it was.
    -->
    <div class="transcript" aria-label={TEXT_TOOLS[textTool ?? ""]?.heading ?? "Text"}>
      <div class="transcript-head">
        <h3>{TEXT_TOOLS[textTool ?? ""]?.heading ?? "Text"}</h3>
        <span class="note">
          {transcriptFor(sourcePath).length > 0
            ? `${transcriptFor(sourcePath).split(/\s+/).filter(Boolean).length} words · on-device`
            : `Press Save to ${TEXT_TOOLS[textTool ?? ""]?.verb ?? "read this file"}`}
        </span>
        <!-- An icon beside the text, not a worded button in a row of them. -->
        <button
          type="button"
          class="copy-icon"
          aria-label="Copy this text"
          title={transcriptCopied === sourcePath ? "Copied" : "Copy"}
          disabled={transcriptFor(sourcePath).length === 0}
          onclick={() => copyTranscript(sourcePath)}
        >
          {#if transcriptCopied === sourcePath}
            <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
              <path
                d="M3.5 8.5l3 3 6-6.5"
                fill="none"
                stroke="currentColor"
                stroke-width="1.6"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
            </svg>
          {:else}
            <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
              <path
                d="M5.5 5.5h7v9h-7zM3.5 10.5v-9h7"
                fill="none"
                stroke="currentColor"
                stroke-width="1.2"
                stroke-linecap="round"
                stroke-linejoin="round"
              />
            </svg>
          {/if}
        </button>
      </div>
      {#if transcriptFor(sourcePath).length === 0}
        <!-- THE EMPTY STATE SAYS WHY IT IS EMPTY. It used to be seeded with
             sample lines; a page of plausible text under this heading is
             indistinguishable from a real result. -->
        <p class="empty-note">
          Nothing read yet. Press Save and the text appears here, and in a
          <code>.txt</code> beside the file.
        </p>
        {#if textTool === "audio-transcribe"}
          <!--
            WHAT THIS MODEL CANNOT DO, BEFORE IT IS ASKED TO.

            Transcription is the one tool here whose output looks equally
            confident whether it is right or wrong: it returns fluent prose in
            every case, so the failure mode is a paragraph that reads perfectly
            and says something nobody said. Stating the limits after the run,
            or in a tooltip, is stating them where they no longer help.

            Every line is measured on this build, not adapted from a model card.
          -->
          <ul class="limits">
            <li>
              Roughly as long as the recording. Fourteen minutes of audio took
              about eight minutes in processor testing; speed depends on the model and hardware.
            </li>
            <li>
              Timed subtitles include model timestamps. Speaker names are not identified.
            </li>
            <li>
              Accuracy falls off with accents, crosstalk, background noise and
              technical vocabulary. This is a small model, chosen so it fits on
              an ordinary machine.
            </li>
            <li>
              Silence is skipped rather than transcribed, because a model given
              silence invents speech to fill it.
            </li>
            <li>
              The language is detected from the recording. If it cannot be
              placed confidently, nothing is written and the reason says so.
            </li>
          </ul>
        {/if}
      {:else}
        <!-- The text as the engine returned it. No invented timestamps: the
             adapter produces prose and lists timing among the things it does
             NOT carry across. -->
        <TextEditor path={savedOutput ?? sourcePath} text={transcriptFor(sourcePath)} audio={textTool === "audio-transcribe" ? sourcePath : undefined} />
      {/if}
    </div>
  {/if}

  {#if runResults.some(r => !r.success)}
    <BatchFileList results={runResults.filter(r => !r.success)} onretry={async () => {
      busy = true;
      try { const fresh = await job.retryFailed(); runResults = job.results; undo.offer(fresh.filter(r=>r.success).length); }
      finally { busy = false; }
    }} />
  {/if}

  <footer>
    <!-- SINGLE OR BATCH, at the far left of the footer, on the destination
         picker's own baseline -- the two questions are "how many files" and
         "where do they go", and they belong on the same line. -->
    {#if canBatch || canCombine}
      <div class="seg mode" role="group" aria-label="How this run works">
        <button
          type="button"
          aria-pressed={!batchMode}
          onclick={() => {
            batchMode = false;
            void loadPreview();
          }}
        >
          Single file
        </button>
        {#if canBatch}
          <button
            type="button"
            aria-pressed={batchMode}
            onclick={() => {
              batchMode = true;
              void loadPreview();
            }}
          >
            Batch
          </button>
        {/if}
        <!-- THE THIRD POSITION, and it belongs on this control rather than in
             the rail: it answers the same kind of question the other two do --
             not what to do, but how this run works. Turning it on lets the
             rail hold more than one lit tool, which is how every other tool is
             already chosen. -->
        {#if canCombine}
          <button
            type="button"
            aria-pressed={combineMode}
            onclick={() => {
              combineMode = !combineMode;
              applied = combineMode ? new Set([...applied].filter(id=>COMBINABLE.has(id))) : new Set(applied.has(current.id) ? [current.id] : []);
            }}
          >
            Combine edits
          </button>
        {/if}
      </div>
    {/if}
    {#if error}
      <p class="error" role="alert">{error}</p>

    {:else if savingBlocked}
      <span class="note">{savingBlocked}</span>
    {:else if pdfNothingToDo}
      <!-- A DISABLED BUTTON HAS TO SAY WHY IT IS DISABLED. Otherwise the tool
           is lit, the file is open, and Save is grey for a reason the user has
           to guess at — which is the same silence as a Save that does
           nothing. -->
      <span class="note">
        {#if applied.has("pdf-merge") && activePaths.length < 2}
          Add a second document to merge.
        {:else if applied.has("pdf-stamp") && !signature}
          Add a signature first.
        {:else if applied.has("pdf-stamp") && !placement}
          Place the signature on a page.
        {/if}
        <!-- NO GENERAL SENTENCE HERE.

             "Rearrange the pages, and Save writes the new order" was the
             `{:else}`, so it appeared under every PDF tool that reached this
             branch with nothing more specific to say -- explaining reordering
             beneath tools that do not reorder anything. The three messages
             above are about the tool that is actually lit; a fourth that is
             about a different tool is worse than silence. -->
      </span>
    {/if}
    <!--
      NO "NOTHING IS WRITTEN UNTIL YOU SAVE."

      It stood here permanently, on every tool, saying nothing about the state
      of anything — the same sentence before, during and after making a choice.
      A footer whose default is a constant is a footer people stop reading,
      which is a problem when the same line has to carry the failure message.
      The slot is now empty until there is something to report.
    -->

    <!--
      THE FOOTER'S RIGHT-HAND SIDE IS ONE SLOT WITH THREE STATES, and each one
      REPLACES the others rather than sitting beside them.

      1. Arranging documents. The only thing to do next is go to the page
         board, so that is the only control offered. A Save and a destination
         picker beside it invited saving a merge that had not been arranged
         yet, and the step button was stranded down in the board's own bar
         where it read as a caption rather than as the way forward.

      2. Just saved. The file is written; "save it again" is not the next
         thing anyone wants, and a destination picker cannot change where the
         file that already exists went. What IS wanted is to look at it, to
         see the record, or to take it back -- so those three take the slot.
         This used to be a panel floating in the middle of the stage, over the
         picture it was talking about.

      3. Everything else. Where it goes, and Save.
    -->
    {#if arrangingDocuments && applied.has("pdf-reorder")}
      <button
        type="button"
        class="primary"
        onclick={openMerged}
        disabled={busy || activePaths.length < 2}
      >
        {applied.has("pdf-reorder") ? "Continue to reorder" : "Continue"}
      </button>
    {:else if saved}
      <!--
        THE BAR STAYS UNTIL THE USER IS DONE WITH IT.

        It used to revert on a five-second timer, which is long enough to read
        the message and not long enough to decide anything -- the buttons were
        gone before someone reaching for Undo could reach it. It now stands
        until Undo is pressed or the next run replaces it.

        No History button: it is not what anyone wants in the second after
        saving a file, and the History screen is a click away in Settings
        whenever it is. No countdown either -- Undo simply stops being offered
        when the window closes, which the button disappearing already says.
      -->
      <div class="after-save">
        {#if textTool === "audio-transcribe"}
          <button type="button" class="quiet" aria-label="Copy transcript" onclick={() => void navigator.clipboard.writeText(activePaths.map(transcriptFor).filter(Boolean).join("\n\n")).catch(e => error = errorText(e))}><Icon name="copy" size={15}/></button>
        {/if}
        {#if savedOutput}
          {#if kind !== "audio"}<button type="button" class="quiet" aria-label="Preview saved output" onclick={() => reviewing = true}><Icon name="eye" size={15}/></button>{/if}
          <button type="button" class="quiet" onclick={() => void revealOutput(savedOutput ?? "")}>
            <Icon name="folder-open" size={15} />
            Show in folder
          </button>
        {/if}
        {#if undo.offered}
          <button type="button" class="quiet" onclick={undoSave}>
            <Icon name="undo-2" size={15} />
            Undo
          </button>
        {/if}
      </div>
    {:else}
      <div class="seg" role="group" aria-label="Save to">
        {#each destinations as d (d.id)}
          <button type="button" disabled={busy} aria-pressed={dest === d.id} onclick={() => (chosenDest = d.id)}>
            {d.label}
          </button>
        {/each}
      </div>
    {/if}
    <!--
      ONE SAVE, AND IT SAYS WHAT IT WRITES.

      Transcription used to carry a second Save inside the text panel, beside a
      Copy button, beside a word count, above the real Save — four controls for
      two actions, and no way to tell which of the two Saves wrote the `.txt`.
      Copy is an icon on the text now, and this is the only Save: for a text
      tool it names the file it produces.
    -->
    {#if job.running}

      <button type="button" class="quiet" disabled={job.cancelling} onclick={() => void job.cancel()}>{job.cancelling ? "Finishing current file…" : "Cancel"}</button>
    {/if}
    {#if (!arrangingDocuments || !applied.has("pdf-reorder")) && !saved}
      <button
        type="button"
        class="primary"
        class:save-progress={busy && kind !== "audio"}
        style:--save-progress={`${progressFraction * 100}%`}
        onclick={save}
        disabled={runPaths.length === 0 || busy || applied.size === 0 || savingBlocked !== null || pdfNothingToDo || (current.id === "pdf-split" && splitParts.length < 2)}
        title={savingBlocked ?? undefined}
      >
        <!-- THE BUTTON NAMES WHAT PRESSING IT DOES.

             "Save as TXT" sat there before anything had been read, offering to
             save text that did not exist yet -- and pressing it is what
             produces the text in the first place. One press analyses, writes
             the `.txt` and puts the text on screen; the row that replaces this
             button afterwards is what says a file was written, with the folder
             and the Undo beside it.

             So the label is the verb for the press: Analyze. -->
        <!-- THE BUTTON NAMES WHAT PRESSING IT DOES.

             "Save as TXT" sat there before anything had been read, offering to
             save text that did not exist yet -- and pressing it is what
             produces the text in the first place. One press reads the file,
             writes the `.txt` and puts the words on screen; the row that
             replaces this button afterwards is what says a file was written.

             Two verbs rather than one, because the two tools are not doing the
             same thing to a person: reading words out of a picture is
             analysis, and turning a recording into text is processing. Both
             are the verb for the press. -->
        {busy ? "In progress…" : textTool !== null ? `Save as ${(values.format ?? "txt").toUpperCase()}` : "Save"}
      </button>
    {/if}
  </footer>
</section>

<style>
  /* Flex column, not a fixed row template: the transcript is conditional, and
     a template that assumes it either wastes a row or pushes the footer into
     an implicit one. Same lesson as the app shell. */
  .preview-heading { display:flex;align-items:center;justify-content:space-between;gap:12px;flex:none; }
  .continuous-preview { display:flex;flex-direction:column;gap:12px;flex: 1; min-height: 0; height: 100%; width: 100%; }
  .workspace {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    height: 100%;
    min-height: 0;
  }

  .workspace > header,
  .workspace > footer,
  .workspace > .transcript {
    flex: none;
  }

  .workspace > .body {
    flex: 1 1 auto;
  }

  header {
    display: flex;
    align-items: center;
    gap: var(--space-4);
  }

  .title {
    display: grid;
    gap: 1px;
    min-width: 0;
  }

  h2 {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
  }

  .file {
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
    overflow-wrap: anywhere;
  }

  .close {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .key {
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
  }

  /* Rail first, stage second — the tools sit beside what they act on. */
  .body {
    display: grid;
    /* WIDE ENOUGH TO CONFIGURE A TOOL IN, which 216 px was not.

       That number was chosen when the rail held nothing but a strip of
       buttons, and it is the reason the settings ended up in a band above the
       stage: a page-order field and a signature picker do not fit in the width
       of a button. The controls live here now, so the column is sized for the
       widest of them rather than for the narrowest.

       It comes out of the stage, and the stage can afford it: it gets back the
       whole height the band was using, which on a short window is the more
       valuable axis for a document. */
    grid-template-columns: 288px minmax(0, 1fr);
    gap: var(--space-4);
    min-height: 0;
  }

  /* --- stage --- */
  .stage {
    position: relative;
    display: grid;
    /* The first child takes the height and everything after it is intrinsic.
       A fixed three-row template broke the moment the image stage grew a
       viewport panel: the canvas landed in an `auto` row and collapsed to the
       height of its own contents. `grid-auto-rows` means adding a row below
       the stage is not a layout change. */
    grid-template-rows: minmax(0, 1fr);
    grid-auto-rows: auto;
    gap: var(--space-3);
    padding: var(--space-5);
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    min-height: 0;
    overflow: hidden;
  }

  .stage.busy {
    opacity: 0.72;
  }

  /* FIT HAS TO HAVE SOMETHING TO FIT INTO.

     `.canvas img` asks for `max-height: 100%`, and this element gave it
     nothing to be 100% OF. With no explicit tracks the implicit row is
     content-sized, a percentage height against a content-sized track resolves
     to `auto`, and the constraint silently did not apply — so a tall image
     rendered at its full height inside a short canvas and `overflow: hidden`
     cropped it. That is why Fit looked like `cover`: it was showing the middle
     of an image far too big for the box, at 100% scale, exactly as `cover`
     would.

     Measured before and after: a 400x2000 image in a 401 px canvas rendered
     2000 px tall; with definite tracks it renders 401 px tall and fits.

     `minmax(0, ...)` on both axes so the tracks are definite AND can shrink
     below the image's intrinsic size, which is the whole point. */
  .canvas {
    display: grid;
    grid-template-rows: minmax(0, 1fr);
    grid-template-columns: minmax(0, 1fr);
    place-items: center;
    min-height: 0;
    border-radius: var(--radius-md);
    overflow: hidden;
    /* The wheel is a zoom here, so the browser must not also scroll. */
    touch-action: none;
  }

  .canvas.grabbable {
    cursor: grab;
  }

  .canvas.panning {
    cursor: grabbing;
  }

  /* Checkerboard: "this is not part of the picture". Behind every image, not
     only transparent ones — see the note on the markup. */
  .checker {
    background-image:
      linear-gradient(45deg, var(--surface-sunken) 25%, transparent 25%),
      linear-gradient(-45deg, var(--surface-sunken) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--surface-sunken) 75%),
      linear-gradient(-45deg, transparent 75%, var(--surface-sunken) 75%);
    background-size: 16px 16px;
    background-position: 0 0, 0 8px, 8px -8px, -8px 0;
  }

  /* ==================================================================== */
  /* PDF: the reorder rail                                                 */
  /* ==================================================================== */

  .info-dot {
    display: inline-grid;
    place-items: center;
    width: 14px;
    height: 14px;
    margin-inline-start: var(--space-2);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: 9px;
    font-style: italic;
    color: var(--text-secondary);
    vertical-align: middle;
  }

  .info-dot:hover {
    color: var(--text-primary);
    border-color: var(--text-secondary);
  }



  .param-error {
    margin: var(--space-2) 0 0;
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--accent-critical);
  }




  .wide {
    width: 100%;
    min-height: 26px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-md);
    border: 0.5px solid var(--hairline);
    font-size: var(--text-caption-size);
    text-align: center;
  }

  .wide:hover:not(:disabled) {
    background: var(--fill-quaternary);
  }

  .wide:disabled {
    opacity: 0.45;
  }

  /* The card's own bin, in drag mode. Top-right of the card, and only visible
     on hover or focus so the board is not a wall of delete buttons. */

  /* The three things worth doing once a file exists. Same row the destination
     picker and Save occupied, because it replaces them. */
  .after-save {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  /* WHAT THE RUN DID, on the line with what to do next. Tabular figures so the
     two sizes line up against each other rather than jittering. */


  .after-save button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* BOTTOM RIGHT, ON THE PAGE NUMBER'S OWN LINE.

     It sat top-left, opposite the bin, on the reasoning that two controls
     belong in a fixed order. What that produced was a card with something in
     three of its four corners and the two controls as far apart as the card
     allows -- so the pair that acts on the page was split across it, and the
     one that opens the editor was the furthest thing from the page number
     naming what it would open.

     Down beside the label it shares a baseline with the number, which is what
     says the two are about the same page. The bin keeps the top right: taking
     a page out of the document is not an edit to it. */
  .card-modify {
    position: absolute;
    inset-block-end: 2px;
    inset-inline-end: 2px;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    opacity: 1;
    transition: opacity var(--dur-fast) var(--ease-out);
  }

  .card:hover .card-modify,
  .card-modify:focus-visible,
  .card-modify[aria-expanded="true"] {
    opacity: 1;
  }

  /* A PAGE THAT HAS BEEN CHANGED SAYS SO WITHOUT BEING HOVERED. A board of
     twenty cards where three are turned is unusable if the only way to see
     which three is to point at each one. */
  .card-modify.edited {
    opacity: 1;
    color: var(--emphasis);
  }

  .card-modify:hover {
    color: var(--text-primary);
  }

  /* A BAND UNDER THE BOARD. See the note on the markup for why it is not a
     popover on the card. One row on a wide window, wrapping on a narrow one:
     the turn, four edges and Done are six controls that read left to right. */
  .page-modify {
    flex: none;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-4);
    padding: var(--space-3) var(--space-5);
    border-radius: var(--radius-md);
    background: var(--surface-sunken);
    border: 0.5px solid var(--hairline);
  }

  .page-modify-title {
    font-size: var(--text-caption-size);
    font-weight: var(--weight-medium);
    color: var(--text-secondary);
  }




  .card-bin {
    position: absolute;
    inset-block-start: 2px;
    inset-inline-end: 2px;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border: 0;
    background: none;
    color: var(--text-tertiary);
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }

  .card:hover .card-bin,
  .card-bin:focus-visible {
    opacity: 1;
  }

  .card-bin:hover {
    color: var(--accent-critical);
  }

  .card-bin svg {
    width: 12px;
    height: 12px;
  }

  /* ==================================================================== */
  /* PDF: the signing column                                               */
  /* ==================================================================== */

  /* The column and whatever floats over it. */
  .sign-stage {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  .sign-stage .column {
    flex: 1 1 auto;
  }

  .sign-zoom {
    position: absolute;
    inset-block-end: var(--space-4);
    inset-inline-end: var(--space-4);
    min-height: 30px;
    padding: 0 var(--space-4);
    border: 0.5px solid var(--hairline-strong);
    border-radius: var(--radius-full);
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    color: var(--text-primary);
    font-size: var(--text-small-size);
    font-variant-numeric: tabular-nums;
    box-shadow: var(--shadow-card);
  }

  .sign-zoom:hover {
    background: var(--surface-raised);
  }

  .column {
    display: grid;
    justify-items: safe center;
    gap: var(--space-4);
    /* BOTH axes: above 1x the page is wider than the column, and a magnified
       page you cannot scroll sideways is a page with its margins cut off. */
    overflow: auto;
    min-height: 0;
    padding: var(--space-2);
  }

  .column.placing {
    /* Nothing else should look clickable while a signature is on the cursor. */
    cursor: crosshair;
  }

  /* THE PAGE TAKES THE COLUMN.

     `max-width` alone let the sheet size itself to the IMAGE, and the image
     was a 220 px thumbnail, so a signing page sat at roughly a third of the
     width available to it. Width, not max-width: the sheet fills its measure
     and the render behind it is now sized for that. */
  .sheet-wrap {
    position: relative;
    /* `min()` INSIDE the multiplication, so 2x is twice the page and not
       twice a page that has already been clamped to the column. */
    width: calc(min(100%, 720px) * var(--sign-zoom, 1));
    line-height: 0;
  }

  .sheet-wrap .sheet {
    width: 100%;
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-sm);
    box-shadow: var(--shadow-card);
  }

  .sheet.placeholder {
    /* A-series, so an unrendered page reserves roughly the right height and
       the column does not jump as thumbnails arrive. */
    aspect-ratio: 1 / 1.414;
    width: min(100%, 420px);
    background: var(--fill-quaternary);
  }

  .sheet-no {
    position: absolute;
    inset-block-end: var(--space-2);
    inset-inline-end: var(--space-2);
    padding: 0 var(--space-2);
    border-radius: var(--radius-full);
    background: var(--surface-sunken);
    font-size: var(--text-caption-size);
    line-height: 16px;
    color: var(--text-secondary);
  }

  /* The placed signature. Anchored by its CENTRE, because that is where the
     cursor was when it was dropped. */
  /* THE PLACEMENT IS OUTLINED.
     
     "Resizing does not seem to work" was, at least in part, that nothing showed
     where to grab: the handle is 12 px, sits outside the image's box, and on a
     pale signature over a white page there was no edge to tell you the object
     had a corner at all. A hairline outline gives it an extent and the handle
     something to sit on. */
  .placed {
    outline: 1.5px solid var(--accent-attention);
    outline-offset: 1px;
    position: absolute;
    inset-block-start: calc(var(--py) * 100%);
    inset-inline-start: calc(var(--px) * 100%);
    width: calc(var(--pw) * 100%);
    transform: translate(-50%, -50%);
    cursor: grab;
    touch-action: none;
  }

  .placed.moving {
    cursor: grabbing;
  }

  .placed img {
    width: 100%;
    height: auto;
    pointer-events: none;
    -webkit-user-drag: none;
  }

  .placed::after {
    /* A hairline while it is being worked on, so its extent is visible against
       a busy page. It is not drawn into the output. */
    content: "";
    position: absolute;
    inset: -3px;
    border: 0.5px dashed var(--accent);
    border-radius: var(--radius-sm);
    pointer-events: none;
  }

  /* Bigger than it looks: the visible dot is 11 px and the hit area is 21 px,
     because an 11 px pointer target is a miss most of the time.

     The white ring is what makes it a handle rather than a speck: a dark dot
     on a signature's own ink is invisible, and a signature is exactly the kind
     of image whose corners are dark. */
  .handle {
    box-shadow: 0 0 0 5px transparent;
    position: absolute;
    width: 11px;
    height: 11px;
    border: 1.5px solid var(--control-knob);
    border-radius: var(--radius-full);
    background: var(--accent);
    touch-action: none;
  }

  .handle.nw {
    inset-block-start: -6px;
    inset-inline-start: -6px;
    cursor: nwse-resize;
  }

  .handle.ne {
    inset-block-start: -6px;
    inset-inline-end: -6px;
    cursor: nesw-resize;
  }

  .handle.sw {
    inset-block-end: -6px;
    inset-inline-start: -6px;
    cursor: nesw-resize;
  }

  .handle.se {
    inset-block-end: -6px;
    inset-inline-end: -6px;
    cursor: nwse-resize;
  }

  /* The signature under the cursor. Never intercepts the click meant for the
     page underneath it. */
  .ghost {
    position: fixed;
    inset-block-start: var(--gy);
    inset-inline-start: var(--gx);
    width: 160px;
    transform: translate(-50%, -50%);
    opacity: 0.75;
    pointer-events: none;
    z-index: 40;
  }

  .place.armed {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }

  /* ==================================================================== */
  /* PDF: the signature library                                            */
  /* ==================================================================== */

  /* The `.sig-*` rules stood here: a stacked list of signature rows with
     thumbnails, a name and a delete button each. The dropdown replaced all of
     it. */

  .sheet-modal {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgba(0, 0, 0, 0.35);
    z-index: 50;
  }

  /* SAVE HAS TO BE ON SCREEN.

     The card had no height bound at all, so on a short window the signature
     preview pushed the actions row off the bottom: the dialog asked for a name
     and then hid the button that accepts it, with nothing to scroll because
     the overflow was the CARD's, not the page's. Bounded and scrollable, with
     the actions pinned to the bottom edge where they cannot be pushed. */
  .modal-card {
    width: min(420px, calc(100vw - 2 * var(--space-6)));
    max-height: min(88vh, 640px);
    overflow-y: auto;
    display: grid;
    align-content: start;
    gap: var(--space-4);
    padding: var(--space-6);
    border-radius: var(--radius-xl);
    background: var(--surface-raised);
    box-shadow: var(--shadow-float);
  }

  .modal-card h3 {
    margin: 0;
    font-size: var(--text-body-size);
    font-weight: var(--weight-semibold);
  }

  .modal-note {
    margin: 0;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  .sig-preview {
    display: grid;
    place-items: center;
    min-height: 120px;
    padding: var(--space-4);
    border-radius: var(--radius-md);
  }

  .sig-preview img {
    max-width: 100%;
    max-height: 160px;
    object-fit: contain;
  }

  .modal-field {
    display: grid;
    gap: var(--space-2);
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  .modal-actions {
    position: sticky;
    bottom: calc(-1 * var(--space-6));
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    /* Its own ground, so scrolled content passes behind it rather than
       through it. */
    padding: var(--space-3) 0 0;
    background: var(--surface-raised);
  }

  .modal-actions button {
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
  }

  .modal-actions .primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--on-accent, #fff);
  }

  .modal-actions button:disabled {
    opacity: 0.45;
  }

  .viewport {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    justify-content: center;
  }

  .vp {
    min-width: 28px;
    min-height: 24px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-full);
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  .vp:hover:not(:disabled) {
    background: var(--fill-quaternary);
    color: var(--text-primary);
  }

  .vp:disabled {
    opacity: 0.4;
  }

  .vp.num {
    min-width: 52px;
    font-variant-numeric: tabular-nums;
  }

  .compare {
    margin-inline-start: var(--space-3);
    border: 0.5px solid var(--hairline-strong);
  }

  .compare.held {
    background: var(--fill-tertiary);
    color: var(--text-primary);
  }

  .canvas img {
    /* FIT, MEASURED, BECAUSE THE PERCENTAGE CHAIN CANNOT REACH HERE.

       `max-height: 100%` needs an ancestor with a DEFINITE height. `.canvas`
       has one -- its grid tracks are `minmax(0, 1fr)`, which is what the note
       up there fixed -- and a bare `<img>` that is a direct grid item resolves
       against it correctly. The colour picker's image is not a direct item:
       it sits inside `.pick-target` and then `.shot`, both of which shrink to
       fit their contents, so the percentage resolved against an `auto` height
       and was dropped. A tall picture rendered full size and `overflow:
       hidden` cut the bottom off -- the same failure as before, one level
       deeper, reintroduced by the wrapper the crosshair needs.

       `.shot` cannot simply be given a height: it is sized to the IMAGE on
       purpose, because the crosshair is positioned as a fraction of it. The
       answer is `aspect-ratio` on `.shot` -- see its own rule below -- which
       gives that element a definite box without measuring anything, and lets
       this percentage resolve again. */
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius-md);
    /* Zoom and pan are a transform, so the layout never reflows and the image
       is never resampled by the layout engine. `translate` before `scale`
       reads as "move the scaled image", which is what dragging it means. */
    transform: translate(var(--pan-x, 0), var(--pan-y, 0)) scale(var(--zoom, 1));
    transform-origin: center;
    /* Above 1:1 the point is to see the pixels, not a smoothed guess. */
    image-rendering: var(--sampling, auto);
    user-select: none;
    -webkit-user-drag: none;
  }

  @media (prefers-reduced-motion: no-preference) {
    .canvas:not(.panning) img {
      transition: transform var(--dur-fast) var(--ease-out);
    }
  }

  /* Transparency is shown, not implied. */
  .stage-note {
    margin: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    flex-wrap: wrap;
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    text-align: center;
  }

  /* The picker turns the image into a target, so it says so. */
  /* The button is the hit area; `.shot` inside it is the image's own box, and
     the crosshair positions against that. */
  /* THE IMAGE'S OWN BOX, WITH A DEFINITE SIZE AND NO MEASUREMENT.

     `.shot` exists so the crosshair can be positioned as a fraction of the
     IMAGE rather than of the button, which is what stopped the marker riding
     high on a letterboxed picture. But inserting it broke `Fit`: the image's
     `max-height: 100%` needs an ancestor with a definite height, and both
     `.shot` and the button around it shrink to their contents, so the
     percentage resolved against `auto`, was dropped, and a tall photograph
     rendered full size with its bottom cut off by `overflow: hidden`.

     `aspect-ratio` resolves it without measuring anything. The preview's own
     dimensions come back from the renderer, so the ratio is known before
     paint; with a ratio and the two max constraints, `.shot` computes a
     definite box that fits inside the canvas's definite grid area, and the
     image fills it exactly. `.shot` is still the image's box, so the
     crosshair is still exact.

     A ResizeObserver would also have worked and was written first. This does
     not need one -- it holds during first paint, before any callback could
     run, and it cannot be defeated by an environment that throttles them. */
  .shot {
    position: relative;
    display: block;
    aspect-ratio: var(--ar);
    max-width: 100%;
    max-height: 100%;
    /* `min-height: auto` IS THE DEFAULT FOR A GRID ITEM, AND IT BEATS
       `max-height`.

       Measured: with the ratio and the max constraints in place and these two
       lines absent, a 400x2000 page still laid out 460x2300 in a 460x348
       canvas -- the max-height was computed as 348 and simply not applied,
       because the automatic minimum size of a grid item is its content's and
       that floor wins. With them, the same page lays out 70x348 and fits.

       This file already carries `min-height: 0` in four other places for the
       same reason. It is the single most reliable way to lose a constraint
       inside a grid. */
    min-width: 0;
    min-height: 0;
    line-height: 0;
  }

  .shot img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  /* Definite, so `.shot` above has something to be a percentage of. Filling
     the canvas rather than the picture also means the crosshair can be aimed
     from anywhere on the stage; `aimAt` clamps to the image's own box. */
  .pick-target {
    position: relative;
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
    cursor: crosshair;
    line-height: 0;
    /* The drag is the gesture; the browser must not start a text selection or
       a scroll underneath it. */
    touch-action: none;
    user-select: none;
  }


  /* The magnified neighbourhood, at the pointer.
     `fixed` because it follows the pointer in viewport coordinates, and
     `pointer-events: none` because it sits under the cursor and must never
     take the drag it exists to serve. */
  .loupe {
    position: fixed;
    inset-block-start: var(--ly);
    inset-inline-start: var(--lx);
    width: var(--loupe-size);
    height: var(--loupe-size);
    transform: translate(-50%, calc(-100% - 18px));
    border-radius: var(--radius-full);
    /* HARD PIXEL EDGES. Smoothing the magnified image would defeat the whole
       point: the user is choosing between adjacent pixels, so the boundary
       between them has to be visible. */
    image-rendering: pixelated;
    background-repeat: no-repeat;
    box-shadow:
      0 0 0 1px var(--hairline-strong),
      0 8px 24px rgba(0, 0, 0, 0.28);
    pointer-events: none;
    z-index: 35;
    overflow: hidden;
  }

  /* The one pixel being read, outlined at the centre of the loupe. */
  .loupe-cell {
    position: absolute;
    inset-block-start: 50%;
    inset-inline-start: 50%;
    width: var(--cell);
    height: var(--cell);
    transform: translate(-50%, -50%);
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.65),
      inset 0 0 0 1px rgba(255, 255, 255, 0.85);
  }

  .crosshair {
    position: absolute;
    /* THE SAME TRANSFORM THE IMAGE GETS, worked out rather than applied.

       The image is drawn `translate(pan) scale(zoom)` about its own centre, so
       a point at fraction f along it lands at `50% + (f - 0.5) * zoom * 100%`,
       plus the pan. Writing that out is what keeps the marker on the pixel.

       Not `transform: scale(...)` on this element: that would magnify the
       13 px ring along with the picture, and the ring is a screen-space marker
       -- it has to stay the same size at every zoom to be aimed with. */
    left: calc(50% + (var(--cx, 0.5) - 0.5) * var(--zoom, 1) * 100% + var(--pan-x, 0px));
    top: calc(50% + (var(--cy, 0.5) - 0.5) * var(--zoom, 1) * 100% + var(--pan-y, 0px));
    width: 13px;
    height: 13px;
    translate: -50% -50%;
    border-radius: var(--radius-full);
    border: 1.5px solid #ffffff;
    box-shadow: 0 0 0 1.5px rgba(0, 0, 0, 0.65);
    pointer-events: none;
  }

  .swatch {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-sm);
    /* `--picked` is set through the CSSOM by the style: directive. */
    background: var(--picked, transparent);
    box-shadow: inset 0 0 0 0.5px var(--hairline-strong);
    flex: none;
  }

  .shape {
    color: var(--text-primary);
  }

  .pager {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-4);
  }

  .working {
    position: absolute;
    inset-block-start: var(--space-5);
    inset-inline-end: var(--space-5);
    margin: 0;
    padding: 2px var(--space-4);
    border-radius: var(--radius-full);
    background: var(--surface-sunken);
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  /* --- pdf --- */
  .pages {
    display: grid;
    place-items: center;
    min-height: 0;
    overflow: auto;
  }

  /* The board: cards in output order, wrapping to fill the stage. */
  .board {
    list-style: none;
    margin: 0;
    padding: var(--space-2);
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(108px, 1fr));
    gap: var(--space-4);
    align-content: start;
    min-height: 0;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
  }

  .card {
    position: relative;
    min-width: 0;
  }

  /* NO DRAG CURSOR ON A BOARD THAT DOES NOT DRAG. A grab cursor over a
     merged preview is the interface claiming an affordance it does not have,
     which is the whole complaint this branch answers. */
  .card-grip.still {
    cursor: default;
  }

  .card-grip {
    display: grid;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    border: 0.5px solid var(--hairline);
    background: var(--surface-base);
    cursor: grab;
    /* The browser must not claim the gesture for panning or text selection:
       with the default `touch-action`, a drag on a touch screen scrolls the
       board instead of moving the card, and on a mouse a slow drag starts a
       selection that fights the pointer capture. */
    touch-action: none;
    user-select: none;
    transition:
      border-color var(--motion-micro),
      translate var(--motion-micro);
  }

  .card-grip:active {
    cursor: grabbing;
  }

  .card-grip:hover {
    border-color: var(--text-tertiary);
  }

  .card.dragging {
    z-index: 5;
    transform: translate(var(--drag-x, 0), var(--drag-y, 0)) scale(1.035);
    filter: drop-shadow(0 14px 18px rgba(0, 0, 0, 0.24));
    pointer-events: none;
  }

  .card.dragging .card-grip {
    border-color: var(--emphasis);
    cursor: grabbing;
  }

  /* Where it would land: a rule on the leading edge, not a moved card. */
  .card.over .card-grip {
    border-color: var(--emphasis);
    translate: 4px 0;
  }

  .card-no {
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
    text-align: left;
  }
  .page-board .card-no { position: absolute; bottom: 4px; left: 8px; }
  .page-board .card-label { visibility: hidden; }

  /* THE WHOLE PAGE, WHATEVER SHAPE IT IS.

     This was `aspect-ratio: 1 / 1.414` with `object-fit: cover` — A4 portrait,
     hard-coded, cropping everything that was not A4 portrait. A landscape page
     lost both its ends; a wide spread showed its middle third. The board's
     entire job is letting someone recognise a page well enough to put it in
     the right place, and it was hiding the part that makes pages tell each
     other apart.

     `contain` inside a fixed box: the box keeps the grid regular, and the page
     letterboxes within it at its own proportions. The sunken ground shows
     around it, which is also how you can see at a glance that a document has
     mixed page sizes. */
  .card-thumb {
    width: 100%;
    aspect-ratio: 1 / 1.414;
    object-fit: contain;
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    box-shadow: 0 0 0 0.5px var(--hairline);
  }

  .card-thumb.placeholder {
    display: block;
  }

  .card-label {
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }

  .card.add .card-grip {
    border-style: dashed;
    cursor: pointer;
    min-height: 100%;
    place-content: center;
    text-align: center;
  }

  .add-plus {
    font-size: 32px;
    line-height: 1;
    color: var(--text-tertiary);
  }

  .card.add .card-label {
    text-align: center;
  }

  .document-actions {
    position: absolute;
    inset-block-start: var(--space-2);
    inset-inline-end: var(--space-2);
    display: flex;
    gap: var(--space-1);
  }

  /* NO DISC BEHIND THE ICON. A filled circle with a shadow reads as a button
     floating ON the card rather than an action belonging to it, and two of
     them side by side turned the corner of every document into a control
     panel. The glyphs carry themselves; the hit area stays 24 px. */
  .document-actions button {
    width: 24px;
    height: 24px;
    display: grid;
    place-items: center;
    border: 0;
    background: none;
    color: var(--text-tertiary);
  }

  /* NO SIZE RULE HERE. The icons are `Icon.svelte` now and it takes a `size`
     prop, so a stylesheet reaching in to resize them is a second answer to a
     question the component already answers -- and the one that loses when
     they disagree. */

  .document-actions button:hover {
    color: var(--text-primary);
  }

  .document-actions button:last-child:hover {
    color: var(--accent-critical);
  }

  /* A continuous document. See the note on the markup. */
  .document-preview {
    min-height: 0;
    /* BOTH AXES. This scrolled vertically only, so a page drawn wider than the
       column had no way to reach its own edges -- which is the whole of the
       "zoom in and the file is not fully visible any more" report. */
    overflow: auto;
    display: flex;
    flex-direction: column;
    /* `safe` IS WHAT KEEPS THE EDGES REACHABLE. A plain `center` centres an
       item that is wider than its container too, which pushes the left edge
       out past the scroll origin -- the browser will not scroll to it, so that
       side is not merely off-screen but unreachable. `safe` centres while it
       fits and falls back to start alignment the moment it does not, which is
       the behaviour every document viewer has. */
    align-items: safe center;
    gap: var(--space-4);
    padding: var(--space-2) var(--shadow-gutter) var(--space-6);
    /* The scrollbar is the only thing saying how long the document is. */
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
    scrollbar-gutter: stable;
  }





  /* On the page, bottom right, quiet. A caption under each page turns into a
     rule across a scrolling document. */


  .document-more {
    margin: 0;
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
  }

  .sheet {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
    border-radius: var(--radius-sm);
    box-shadow: 0 0 0 0.5px var(--hairline-strong), 0 4px 14px rgba(0, 0, 0, 0.18);
  }



  .stage-empty,
  .stage-error {
    margin: 0;
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .stage-error {
    color: var(--accent-critical);
  }

  /* --- audio ---------------------------------------------------------------

     TOP-ALIGNED, AND A LIST.

     `align-content: center` was the whole problem in one declaration: it put a
     single waveform in the middle of a tall panel, which is a stage. A list of
     files starts at the top and grows downwards, and scrolls when there are
     more of them than fit. */
  /* A border that fills as the work is done.

     Drawn with two stacked backgrounds in the element's PADDING box, clipped
     so only the border area paints: a `conic-gradient` over a flat track. That
     keeps it on the container's own edge with no extra element and no layout
     shift when it appears -- the border is the same width whether or not
     anything is running.

     `--progress` is 0..1. When the total is one file there is no honest
     fraction to draw, so `.sweeping` rotates a short arc
     instead of filling one. */
  .ring {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    border: 2px solid transparent;
    border-radius: var(--radius-lg);
    background:
      linear-gradient(var(--surface-base), var(--surface-base)) padding-box,
      conic-gradient(
          from -90deg,
          var(--accent) calc(var(--progress, 0) * 360deg),
          transparent 0
        )
        border-box;
  }

  /* Not running: no ring at all, and the same 2px so nothing moves. */
  .ring:not(.running) {
    background: linear-gradient(var(--surface-base), var(--surface-base)) padding-box;
  }

  @media (prefers-reduced-motion: no-preference) {
    .ring.running {
      transition: background var(--dur-fast) linear;
    }

    /* One file: a moving arc, because there is no measured fraction. */
    .ring.sweeping {
      background:
        linear-gradient(var(--surface-base), var(--surface-base)) padding-box,
        conic-gradient(
            from var(--sweep, 0deg),
            transparent 0deg,
            var(--accent) 40deg,
            transparent 80deg
          )
          border-box;
      animation: ring-sweep 1.4s linear infinite;
    }
  }

  /* Reduced motion: a still, complete ring. It says "running" without
     implying a position it does not know. */
  @media (prefers-reduced-motion: reduce) {
    .ring.sweeping {
      border-color: var(--hairline-strong);
    }
  }

  @property --sweep {
    syntax: "<angle>";
    inherits: false;
    initial-value: 0deg;
  }

  @keyframes ring-sweep {
    to {
      --sweep: 360deg;
    }
  }

  .audio-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    align-self: stretch;
    justify-self: stretch;
    align-content: start;
    width: 100%;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3);
  }

  .audio-row {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-card);
  }

  .audio-head {
    display: flex;
    align-items: baseline;
    gap: var(--space-3);
    min-width: 0;
  }

  /* The filename takes the room and truncates in the middle of the flex row
     rather than pushing the operation and the trashcan off the edge. */
  .audio-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--weight-medium);
  }

  .audio-op,
  .audio-facts {
    flex: none;
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--text-secondary);
  }

  .audio-remove {
    flex: none;
    align-self: center;
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .audio-remove:hover,
  .audio-remove:focus-visible {
    background: var(--surface-sunken);
    color: var(--text-primary);
  }

  .audio-remove svg {
    width: 15px;
    height: 15px;
  }

  /* A ROW-SIZED WAVEFORM.

     `WaveForm` is `height: 100%` with a 120 px floor, which is right for a
     stage that is one recording and wrong for a list: three files filled the
     panel and the fourth was below the fold. 56 px still shows the shape of
     the recording — where the speech is and where the silences are, which is
     all this row is claiming to show. */
  .audio-row :global(.wave) {
    height: 56px;
    min-height: 56px;
  }

  .audio-empty {
    margin: 0;
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--text-secondary);
  }

  /* The transcript, with its recording. */
  .audio-text {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    max-height: 180px;
    padding: var(--space-2);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
  }


  .copy-icon {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
  }

  .copy-icon:hover:not(:disabled),
  .copy-icon:focus-visible {
    background: var(--surface-raised);
    color: var(--text-primary);
  }

  .copy-icon:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .copy-icon svg {
    width: 16px;
    height: 16px;
  }

  /* The "+" tile, matching every other upload target in the app. */
  .audio-add {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-4);
    border: 1px dashed var(--hairline-strong);
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-secondary);
    font: inherit;
    cursor: pointer;
  }

  .audio-add:hover:not(:disabled),
  .audio-add:focus-visible {
    border-color: var(--focus-ring);
    color: var(--text-primary);
  }

  .audio-add:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .audio-plus {
    font-size: 1.25rem;
    line-height: 1;
  }

  /* --- command rail --- */
  .rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-2);
    /* A PALETTE, NOT ANOTHER CARD.

       The rail used `--surface-raised` with `--shadow-card` and the same
       radius as the stage beside it, so the thing you work WITH looked exactly
       like the thing you work ON: two panels of the same material, and no
       visual answer to "which of these is the instrument". The palette sits on
       the sunken ground instead, with a hard edge on the side facing the
       stage — the Photoshop arrangement, where the tools are furniture and the
       document is the only lit surface in the window. */
    background: var(--surface-sunken);
    border: 0.5px solid var(--hairline);
    border-radius: var(--radius-md);
    box-shadow: inset -1px 0 0 var(--hairline);
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }

  .limits {
    margin: var(--space-3) 0 0;
    padding-inline-start: var(--space-5);
    display: grid;
    gap: var(--space-1);
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-tertiary);
  }

  .rail h3,
  .transcript-head h3 {
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    font-weight: var(--weight-medium);
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-tertiary);
    padding-inline: var(--space-3);
  }

  .rail ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }

  .cmd {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 36px;
    padding: var(--space-2);
    /* The lit well is drawn INSIDE this box, so a tool turning on cannot
       change how tall the row is. That was the other half of the "applied"
       label problem: state that reflows the rail makes the rail move under
       the pointer while you are using it. */
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    text-align: left;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    transition:
      background var(--motion-micro),
      border-color var(--motion-micro);
  }

  .cmd:hover:not(:disabled) {
    background: var(--surface-raised);
    color: var(--text-primary);
  }

  /* Pointed at, but not turned on: the rail's own cursor. */
  .cmd.current:not(.on) {
    border-color: var(--hairline-strong);
    color: var(--text-primary);
  }

  /* SELECTED. This is the whole of what the "applied" label used to say, and
     it says it the way a tool palette does: the button is pressed in. */
  .cmd.on {
    background: var(--surface-base);
    border-color: var(--hairline-strong);
    box-shadow: var(--shadow-card);
    color: var(--text-primary);
  }

  /* THE SETTINGS ARE INSIDE THE TOOL'S WELL, not below it.

     They were a sibling under the lit button, which reads as a second thing
     that happens to be nearby -- and on a rail where the next tool is 2 px
     further down, "nearby" is not a relationship anyone can see. The well
     around both says whose settings these are, which is the whole point of
     having moved them here.

     The button's own well is dropped when the item is open, or there would be
     a card inside a card with a hairline between them. */
  li.open {
    background: var(--surface-base);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-card);
    padding-block-end: 0;
  }

  li.open .cmd.on {
    background: none;
    border-color: transparent;
    box-shadow: none;
  }

  .cmd.on .cmd-title {
    font-weight: var(--weight-medium);
  }

  .cmd:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .cmd-state,
  .cmd-why {
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
  }

  .cmd-state {
    color: var(--accent-attention);
  }

  .cmd-why {
    color: var(--text-tertiary);
  }

  /* Square, even, and the same size whatever the glyph — the grid a palette
     is read as. It was a rounded chip on the sunken surface, which is now the
     rail's own background, so a lit tool's icon vanished into it. */
  .cmd-icon {
    flex: none;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    border: 0.5px solid var(--hairline);
    font-size: 13px;
    color: var(--text-secondary);
  }

  .cmd.on .cmd-icon {
    background: var(--surface-sunken);
    border-color: var(--hairline-strong);
    color: var(--text-primary);
  }

  .cmd.current .cmd-icon {
    color: var(--text-primary);
  }

  .cmd-text {
    display: grid;
    gap: 1px;
    min-width: 0;
    text-align: left;
  }

  .cmd-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* Per-tool settings, rendered from the descriptor's own schema, in the rail
     under the tool that owns them.

     ONE COLUMN, EVERY CONTROL FULL WIDTH. The band this replaced laid its
     controls out in `auto-fit` columns to use the width of the stage; there is
     no width to spread across here, and a control that is sometimes half a row
     wide and sometimes whole is a control that moves when a sibling appears.

     The old rail clipped what did not fit (`overflow: hidden`, which is where
     the reorder controls' horizontal scrollbar came from). Nothing is clipped
     now -- the column scrolls, and every control is sized to it. */
  /* ONE SHAPE FOR EVERY SETTING IN EVERY TOOL.

     This is the only wrapper any tool's settings render into -- images, PDF
     and audio alike -- so the inset and the stretch are declared once here
     rather than per control. They were not symmetric: 12 px above, 8 px at the
     sides, 8 px below, which read as a control sitting slightly high and
     slightly wide in its well, and differently so from one tool to the next
     depending on what the control was.

     `justify-items: stretch` is what makes "full width" true for every child
     rather than only for the ones that happened to declare `width: 100%`. The
     magnifier segment was one of the ones that did not. */
  .part-name { width: 100%; min-width: 0; font-size: var(--text-caption-size); }
  .edit-actions { display: flex; flex-wrap: wrap; gap: 4px; }
  .page-select { position: absolute; top: 8px; left: 8px; z-index: 2; width: 18px; height: 18px; }
  .compression-note { font-size: var(--text-caption-size); color: var(--text-secondary); margin: 0; overflow-wrap: anywhere; }
  .settings-fieldset { display: grid; gap: 8px; border: 0; margin: 0; padding: 12px; min-width: 0; width: 100%; box-sizing: border-box; }

  .tool-settings {
    display: grid;
    gap: var(--space-3);
    padding: 0;
    justify-items: stretch;
    min-width: 0;
  }

  .tool-settings > * {
    min-width: 0;
  }

  .quiet.reset {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* The mode segment, then whatever that mode needs, stacked. It was one
     baseline across the band's full width; a column has no baseline to share. */
  .reorder-row {
    display: grid;
    gap: var(--space-2);
    min-width: 0;
  }

  .reorder-field {
    min-width: 0;
  }

  /* Which signature, and put it down -- one per line, each the width of the
     column. They were side by side in the footer, where "Place signature"
     ended up narrower than the dropdown beside it for no reason a user could
     see. */
  /* THE BIN BELONGS BESIDE THE SIGNATURE IT DELETES.

     It was on a line of its own under the Place button -- a lone icon at the
     bottom left of the panel, nearer to a control it has nothing to do with
     than to the name it removes. On the signature's own row, at the right
     edge, there is no question what it deletes.

     Two columns for that row; the Place button spans both, so it stays the
     full width of the column. */
  .sign-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--space-2);
    min-width: 0;
  }

  .sign-row :global(.dropdown) {
    width: 100%;
  }

  .sign-row :global(.trigger) { width: 100%; min-width: 0; }

  .placement-actions { display: flex; gap: var(--space-2); min-width: 0; }
  .placement-actions .place { flex: 1; min-width: 0; }
  .quiet { display: inline-flex; align-items: center; justify-content: center; gap: var(--space-2); }




  .sign-del {
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 0;
    background: none;
    color: var(--text-tertiary);
  }

  .sign-del:hover {
    color: var(--accent-critical);
  }

  /* The right-hand column: settings, then the document. The stage keeps every
     spare pixel; the band is only as tall as what it holds. */
  .work {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
    min-height: 0;
  }

  .work .stage {
    flex: 1 1 auto;
    min-height: 0;
  }

  /* Somewhere to put a file, when none was carried in. */
  /* With no file the stage is one big target; the row template collapses to
     a single stretched cell. */
  .stage.empty {
    grid-template-rows: minmax(0, 1fr);
  }

  .discard {
    position: absolute;
    inset-block-end: var(--space-5);
    inset-inline-end: var(--space-5);
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-raised);
    color: var(--text-secondary);
    transition:
      color var(--motion-micro),
      border-color var(--motion-micro);
  }

  .discard svg {
    width: 15px;
    height: 15px;
  }

  .discard:hover {
    color: var(--accent-critical);
    border-color: var(--accent-critical);
  }

  .stage-drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-5);
    border: 1.5px dashed var(--dropzone-border);
    border-radius: var(--radius-lg);
    cursor: pointer;
    transition:
      border-color var(--motion-micro),
      background var(--motion-micro);
  }

  .stage-drop:hover {
    border-color: var(--text-secondary);
    background: var(--surface-sunken);
  }

  .drop-glyph {
    font-size: 20px;
    color: var(--text-secondary);
  }

  .drop-title {
    font-size: var(--text-body-size);
    font-weight: var(--weight-medium);
  }

  .drop-hint {
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  .param {
    display: grid;
    gap: var(--space-2);
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  .field {
    min-height: 28px;
    width: 100%;
    padding: 0 var(--space-3);
    border-radius: var(--radius-sm);
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-base);
    color: var(--text-primary);
    font-size: var(--text-small-size);
  }

  /* --- transcript --- */
  .transcript {
    display: grid;
    /* The second track must be bounded or `.lines` cannot scroll and its rows
       spill past the workspace instead. */
    grid-template-rows: auto minmax(0, 1fr);
    gap: var(--space-2);
    padding: var(--space-4) 0;
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    height: 168px;
    min-height: 0;
  }

  .transcript-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding-inline: var(--space-3) var(--space-5);
  }

  .transcript-head .note {
    margin-left: auto;
  }


  /* One block of prose, scrolling. It was a two-column list of timestamped
     segments; the engine returns a string with no timing in it. */
  .lines {
    margin: 0;
    padding: 0 var(--space-5) 0 var(--space-6);
    overflow-y: auto;
    min-height: 0;
    font-size: var(--text-small-size);
    line-height: var(--text-body-lh);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  /* FOUR LINES PER RECORDING, whatever the sentences do.

     `excerpt` already cuts to a sentence boundary, which is right for reading
     but says nothing about HEIGHT: two sentences of a rambling dictation is
     four lines on a wide window and nine on a narrow one, so a list of four
     recordings changed shape with the window and the rows stopped being
     comparable. The clamp is about the list; the excerpt is about the words,
     and both are wanted.

     The whole text is in the `.txt` that was just written, and Copy still
     copies all of it -- the button is not an excerpt either. */

  .empty-note {
    margin: 0;
    padding: 0 var(--space-6);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  /* --- generic settings controls --- */
  .param-title {
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  .range {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .range input {
    flex: 1 1 auto;
    min-width: 0;
    accent-color: var(--emphasis);
  }

  .range output {
    flex: none;
    min-width: 3ch;
    text-align: right;
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
    font-variant-numeric: tabular-nums;
  }

  /* THE CROP EDITOR. The page fills the stage; the part being kept is drawn
     bright over a dimmed remainder, which is the convention every image
     cropper uses and needs no label. */
  .crop-editor {
    container-type: size;
    position: absolute;
    inset: 0;
    z-index: 8;
    background: var(--surface-base);
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: var(--space-3);
    min-height: 0;
    padding: var(--space-3);
  }

  .crop-page {
    transform: rotate(var(--turn, 0deg)) scale(var(--turn-scale, 1));
    justify-self: center;
    align-self: center;
    width: min(100%, calc((100cqh - 72px) * var(--page-ratio, 0.707)));
    aspect-ratio: var(--page-ratio, 0.707);
    max-width: 100%;
    max-height: 100%;
    position: relative;
    display: grid;
    place-items: center;
    min-height: 0;
    background: var(--surface-sunken);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .crop-page img {
    display: block;
    width: 100%;
    height: 100%;
    max-inline-size: 100%;
    max-block-size: 100%;
    object-fit: contain;
    /* The turn is shown, not just counted: a page about to be rotated looks
       rotated. */
  }

  /* The kept region: a bright outline over the whole page, inset by each
     edge's own percentage. The dimming is its outsized shadow, so there is one
     element rather than four. */
  .crop-keep {
    position: absolute;
    inset: var(--t, 0) var(--r, 0) var(--b, 0) var(--l, 0);
    border: 1px solid var(--emphasis);
    box-shadow: 0 0 0 9999px color-mix(in srgb, var(--surface-sunken) 72%, transparent);
    pointer-events: none;
  }

  /* Handles sit ON the kept region's edges, and are the only thing here that
     takes a pointer. 16 px is the smallest that can be hit reliably; the
     visible line is thinner. */
  .crop-handle {
    position: absolute;
    border: 0;
    background: none;
    padding: 0;
    cursor: grab;
    touch-action: none;
  }
  .crop-corner {
    position: absolute; width: 18px; height: 18px; border: 3px solid var(--emphasis);
    border-radius: 2px; padding: 0; background: var(--surface-base); cursor: move; touch-action: none;
  }

  .crop-handle::after {
    content: "";
    position: absolute;
    inset: 0;
    margin: auto;
    background: var(--emphasis);
    border-radius: 2px;
  }

  .crop-handle:focus-visible {
    outline: 2px solid var(--emphasis);
    outline-offset: 2px;
  }

  .crop-handle.left,
  .crop-handle.right {
    inline-size: 16px;
    inset-block: var(--t, 0) var(--b, 0);
    cursor: ew-resize;
  }

  .crop-handle.left::after,
  .crop-handle.right::after {
    inline-size: 3px;
    block-size: 36px;
  }

  .crop-handle.left {
    inset-inline-start: calc(var(--l, 0%) - 8px);
  }

  .crop-handle.right {
    inset-inline-end: calc(var(--r, 0%) - 8px);
  }

  .crop-handle.top,
  .crop-handle.bottom {
    block-size: 16px;
    inset-inline: var(--l, 0) var(--r, 0);
    cursor: ns-resize;
  }

  .crop-handle.top::after,
  .crop-handle.bottom::after {
    block-size: 3px;
    inline-size: 36px;
  }

  .crop-handle.top {
    inset-block-start: calc(var(--t, 0%) - 8px);
  }

  .crop-handle.bottom {
    inset-block-end: calc(var(--b, 0%) - 8px);
  }

  .crop-actions {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  /* Minus, the current size, plus. The number is a label between two
     buttons, so it must not look like a third one. */
  .zoomer {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .zoom-now {
    min-inline-size: 4ch;
    text-align: center;
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  /* The tier buttons carry a second line, so they are taller than a plain
     segment and the label has to sit above the size rather than beside it. */
  .tier-seg button {
    display: flex;
    align-items: center;
    justify-content: center;
    padding-block: 0;
    line-height: 1.2;
  }



  /* AS MANY COLUMNS AS THERE ARE BUTTONS, not three.

     Hard-coding three was survivable in a wide band; in a 288 px column a
     two-option segment left a third of itself empty and a four-option one
     wrapped to two rows of unequal width. `auto-flow: column` sizes to the
     options the tool actually declares. */
  .tool-settings .seg {
    width: 100%;
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: minmax(0, 1fr);
  }

  .tool-settings .seg button {
    display: flex; align-items: center; justify-content: center; text-align: center; min-height: 32px; line-height: 1.2;
    min-width: 0;
    padding-inline: 3px;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* --- footer --- */
  footer {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex-wrap: wrap;
  }

  .note {
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  .error {
    margin: 0;
    font-size: var(--text-small-size);
    color: var(--accent-critical);
  }



  footer .seg {
    margin-left: auto;
  }

  /* The file-count segment is the exception: it is the LEFT end of the row,
     so it must not take the auto margin that pushes the destination picker
     and Save to the right. */
  footer .seg.mode {
    margin-left: 0;
    margin-right: auto;
  }

  /* --- batch --- */
  .batch {
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
    gap: var(--space-3);
    min-height: 0;
  }

  .batch-list {
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-inline: var(--shadow-gutter);
  }



  .batch-thumb.open {
    cursor: zoom-out;
    box-shadow: 0 0 0 1.5px var(--accent);
  }





  /* The add button and the clear-all bin are one control, side by side. */
  .batch-actions {
    display: flex;
    align-items: stretch;
    gap: var(--space-2);
    padding-inline: var(--shadow-gutter);
  }

  .batch-add {
    flex: 1 1 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: 40px;
    border: 0.5px dashed var(--hairline-strong);
    /* Fully round on the trailing end, to match the bin beside it. */
    border-radius: var(--radius-full);
    background: none;
    color: var(--text-secondary);
    font-size: var(--text-small-size);
  }

  .batch-add:hover {
    color: var(--text-primary);
    background: var(--surface-sunken);
  }

  .batch-clear {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 40px;
    border: 0.5px solid var(--hairline-strong);
    border-radius: var(--radius-full);
    background: none;
    color: var(--text-tertiary);
  }

  .batch-clear:hover {
    color: var(--accent-critical);
    background: var(--surface-sunken);
  }

  .quiet {
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .quiet:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .primary {
    min-height: 32px;
    padding: 0 var(--space-6);
    border-radius: var(--radius-full);
    background: var(--text-primary);
    color: var(--surface-base);
    font-size: var(--text-body-size);
    font-weight: var(--weight-medium);
  }

  .primary:disabled {
    background: var(--surface-sunken);
    color: var(--text-disabled);
    cursor: default;
  }

  @media (prefers-reduced-motion: reduce) {
    .cmd {
      transition: none;
    }
  }

  .colour-value { border: 1px solid var(--hairline); border-radius: var(--radius-sm); background: var(--surface-sunken); padding: 5px 9px; font-size: var(--text-caption-size); }
  .crop-actions { justify-content: center; }
  .editable-page { padding-top: 0; }
  .editable-page .card-bin { inset-block-start: auto; inset-block-end: 2px; inset-inline-end: 26px; opacity: 1; height: 20px; width: 20px; }
  .split-actions { inset-block-start: auto; inset-block-end: 4px; }
  .split-card .card-grip { padding-bottom: 32px; }
  .split-card .card-no { position: absolute; bottom: 8px; left: 12px; }
  .audio-result-actions { display: flex; gap: 4px; justify-content: flex-end; }
  .after-save { gap: 6px; margin-left: auto; }
  .save-progress { background: linear-gradient(to right, #b9e6c5 0 var(--save-progress),  #eef5ef var(--save-progress) 100%) !important; color: #173d24; opacity: 1 !important; }

  .signature-drop { width: 100%; min-height: 110px; display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; border: 1px dashed var(--hairline-strong); border-radius: var(--radius-lg); background: var(--surface-sunken); }
</style>
