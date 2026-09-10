/**
 * Fixture data for the browser preview. **Not shipped.**
 *
 * Every value here is shaped to the real wire types in `src/lib/ipc.ts`, so the
 * preview exercises the real components with the real CSS. Nothing in `src/`
 * knows this file exists — the swap happens by Vite alias in
 * `vite.preview.config.ts`.
 */

import type {
  AiFeature,
  Config,
  ConversionResult,
  HistoryEntry,
  ModelInfo,
  PlanPreview,
  Prediction,
  ProbeResult,
  ReceiptEntry,
  Signature,
  ToolDescriptor,
} from "../src/lib/ipc";

const DIR = "C:\\Users\\you\\Pictures";

/**
 * A plausible per-file duration that is the same on every run.
 *
 * This was `340 + Math.random() * 900`, which made the results screen show
 * different numbers every reload. That is noise when reviewing the UI, and it
 * is worse than noise for `site/tools/shots`: the screenshots on the website
 * are regenerated from this harness and diffed, so a random millisecond count
 * would report "the UI changed" on every single capture and the diff would
 * stop meaning anything.
 *
 * Hashing the path keeps the rows looking different from each other without
 * being different from themselves.
 */
function pseudoDuration(path: string): number {
  let h = 2166136261;
  for (let i = 0; i < path.length; i++) {
    h ^= path.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return 340 + ((h >>> 0) % 900);
}

/** Documents, for the PDF workspace. */
export const PDF_PATHS = [`${DIR}\\report.pdf`, `${DIR}\\appendix.pdf`];

export const PATHS = [
  `${DIR}\\IMG_4821.heic`,
  `${DIR}\\IMG_4822.heic`,
  `${DIR}\\IMG_4823.heic`,
  `${DIR}\\screenshot.png`,
  `${DIR}\\voice-memo.flac`,
];

/**
 * A second and third recording, so the audio workspace can be previewed as
 * what it now is: a LIST.
 *
 * The workspace used to draw one file as a large centred waveform, so one
 * fixture path was enough to preview all of it. It is a batch list now, and a
 * harness that can only ever hand it one row would preview a case the product
 * no longer has.
 */
export const AUDIO_PATHS = [
  PATHS[4],
  `${DIR}\\interview-part-2.mp3`,
  `${DIR}\\standup-recording.mp4`,
];

const probe = (
  path: string,
  detected: string,
  extra: Partial<ProbeResult> = {},
): ProbeResult => ({
  path,
  fileName: path.slice(path.lastIndexOf("\\") + 1),
  detected,
  declared: detected,
  mismatched: false,
  polyglot: false,
  nameAltered: false,
  inputBytes: 2_847_100,
  properties: {
    kind: "image",
    width: 4032,
    height: 3024,
    hasAlpha: false,
    frames: 1,
    durationMs: null,
    channels: null,
    sampleRate: null,
    videoCodec: null,
    audioCodec: null,
    pages: null,
    encrypted: null,
    entries: null,
    depth: null,
    rows: null,
    columns: null,
  },
  ...extra,
});

export const PROBES: Record<string, ProbeResult> = {
  ...Object.fromEntries(PDF_PATHS.map(path => [path, probe(path, "pdf", {
    properties: { ...probe(path, "pdf").properties!, kind: "document", width: null, height: null, pages: 6 }
  })])),
  [PATHS[0]]: probe(PATHS[0], "heic"),
  [PATHS[1]]: probe(PATHS[1], "heic"),
  [PATHS[2]]: probe(PATHS[2], "heic"),
  // The extension says PNG and the bytes say JPEG: SR-4's surfaced mismatch.
  [PATHS[3]]: probe(PATHS[3], "jpeg", {
    declared: "png",
    mismatched: true,
    inputBytes: 412_880,
    properties: {
      kind: "image",
      width: 2560,
      height: 1440,
      hasAlpha: false,
      frames: 1,
      durationMs: null,
      channels: null,
      sampleRate: null,
      videoCodec: null,
      audioCodec: null,
      pages: null,
      encrypted: null,
      entries: null,
      depth: null,
      rows: null,
      columns: null,
    },
  }),
  [PATHS[4]]: probe(PATHS[4], "flac", {
    inputBytes: 18_220_400,
    properties: {
      kind: "audio",
      width: null,
      height: null,
      hasAlpha: null,
      frames: null,
      durationMs: 194_000,
      channels: 2,
      sampleRate: 48_000,
      videoCodec: null,
      audioCodec: null,
      pages: null,
      encrypted: null,
      entries: null,
      depth: null,
      rows: null,
      columns: null,
    },
  }),
  [AUDIO_PATHS[1]]: probe(AUDIO_PATHS[1], "mp3", {
    inputBytes: 7_940_112,
    properties: {
      kind: "audio",
      width: null,
      height: null,
      hasAlpha: null,
      frames: null,
      durationMs: 812_000,
      channels: 1,
      sampleRate: 44_100,
      videoCodec: null,
      audioCodec: null,
      pages: null,
      encrypted: null,
      entries: null,
      depth: null,
      rows: null,
      columns: null,
    },
  }),
  // A recording that arrives in a VIDEO container, because that is the case
  // the file dialog used to hide: the routes have always accepted it.
  [AUDIO_PATHS[2]]: probe(AUDIO_PATHS[2], "mp4", {
    inputBytes: 44_119_003,
    properties: {
      kind: "audio",
      width: null,
      height: null,
      hasAlpha: null,
      frames: null,
      durationMs: 1_505_000,
      channels: 2,
      sampleRate: 48_000,
      videoCodec: null,
      audioCodec: null,
      pages: null,
      encrypted: null,
      entries: null,
      depth: null,
      rows: null,
      columns: null,
    },
  }),
};

export const PREDICTIONS: Prediction[] = [
  {
    input: "heic",
    kind: "image",
    paths: PATHS.slice(0, 3),
    why: "you have converted heic to jpeg 14 times",
    suggestions: [
      { target: "jpeg", score: 0.91, className: "B (lossy, standard)", armed: true },
      { target: "png", score: 0.34, className: "A (lossless)", armed: false },
      { target: "webp", score: 0.21, className: "B (lossy, standard)", armed: false },
      { target: "pdf", score: 0.08, className: "C (rebuilt)", armed: false },
    ],
  },
  {
    input: "jpeg",
    kind: "image",
    paths: [PATHS[3]],
    why: "screenshots usually want compressing",
    suggestions: [
      { target: "png", score: 0.68, className: "A (lossless)", armed: true },
      { target: "webp", score: 0.38, className: "B (lossy, standard)", armed: false },
    ],
  },
  {
    input: "flac",
    kind: "audio",
    paths: [PATHS[4]],
    why: "this is what flac usually becomes",
    suggestions: [
      { target: "wav", score: 0.72, className: "A (lossless)", armed: true },
      { target: "ogg", score: 0.31, className: "B (lossy, standard)", armed: false },
    ],
  },
];

const LIMITS = "1024 MiB memory, 256 Mpx decode, 120s wall";

export function planFor(paths: string[], target: string): PlanPreview {
  const names = paths.map((p) => p.slice(p.lastIndexOf("\\") + 1));
  const audio = target === "wav" || target === "ogg";

  if (target === "pdf") {
    return {
      executable: false,
      steps: [],
      warnings: names.map((fileName) => ({
        fileName,
        blocking: true,
        message:
          "this route is class C and the policy arms up to B. Ask for the operation by name to run it.",
      })),
      estimatedDurationMs: 0,
      totalInputBytes: paths.length * 2_847_100,
    };
  }

  return {
    executable: true,
    steps: names.map((fileName) => ({
      fileName,
      kind: audio
        ? `Transcode { from: flac, to: ${target} }`
        : `Transcode { from: ${PROBES[paths[0]]?.detected ?? "heic"}, to: ${target} }`,
      className: target === "png" || target === "wav" ? "A" : "B",
      isolation: "sandboxed (elevated)",
      limitsSummary: LIMITS,
      engineName: audio ? "oc-audio" : "oc-images",
    })),
    warnings: names
      .filter((n) => n === "screenshot.png")
      .map((fileName) => ({
        fileName,
        blocking: false,
        message: "the name says png and the content says jpeg; routing by content",
      })),
    estimatedDurationMs: 0,
    totalInputBytes: paths.length * 2_847_100,
  };
}

/**
 * The one file in the sample drop that does not convert.
 *
 * A harness that only ever shows success cannot be used to check the states
 * that matter most: the failure row in `BatchFileList`, the `.failed` styling
 * in `ReceiptView`, and the summary's failed count. One file is enough to put
 * all three on screen.
 */
const FAILS = "IMG_4823.heic";

export function resultFor(path: string, target: string): ConversionResult {
  const fileName = path.slice(path.lastIndexOf("\\") + 1);
  if (fileName === FAILS) {
    return {
      fileName,
      outputPath: "",
      receiptPath: "",
      success: false,
      errorMessage:
        "libheif could not decode this file: unsupported colour profile",
      durationMs: 120,
      inputBytes: PROBES[path]?.inputBytes ?? 2_847_100,
      outputBytes: 0,
      classApplied: "",
      removedMetadata: [],
      contentId: "",
      receiptDetail: null,
    };
  }
  const stem = fileName.slice(0, fileName.lastIndexOf("."));
  const ext = target === "jpeg" ? "jpg" : target;
  const outputPath = `${DIR}\\${stem}.${ext}`;
  const inputBytes = PROBES[path]?.inputBytes ?? 2_847_100;
  const outputBytes = Math.round(inputBytes * 0.42);

  return {
    fileName,
    outputPath,
    receiptPath: `${outputPath}.receipt.json`,
    success: true,
    errorMessage: null,
    durationMs: pseudoDuration(path),
    inputBytes,
    outputBytes,
    classApplied: target === "png" || target === "wav" ? "A (lossless)" : "B (lossy, standard)",
    removedMetadata: ["GPS coordinate", "camera serial number", "capture timestamp"],
    contentId: "9f2c41ab77e0d3c518be40aa2b6f19d47c3e88a1f05b6d2e9c7a4318bb0e5f62",
    receiptDetail: {
      version: 1,
      tool: "openconvert 0.1.0",
      contentId: "9f2c41ab77e0d3c518be40aa2b6f19d47c3e88a1f05b6d2e9c7a4318bb0e5f62",
      detected: PROBES[path]?.detected ?? "heic",
      declaredMismatch: PROBES[path]?.mismatched ? "png" : null,
      class: target === "png" || target === "wav" ? "A" : "B",
      outputName: `${stem}.${ext}`,
      outputBytes,
      steps: [
        {
          kind: `Transcode { from: ${PROBES[path]?.detected ?? "heic"}, to: ${target} }`,
          class: target === "png" || target === "wav" ? "A" : "B",
          engine: target === "wav" || target === "ogg" ? "oc-audio" : "oc-images",
          isolation: "sandboxed (elevated)",
          limitsSummary: LIMITS,
        },
      ],
    },
  };
}

export const CONFIG: Config = {
  workerReuse: "balanced",
  forceSandbox: false,
  // false so the preview SHOWS the first-run chooser rather than hiding it.
  aiSetupDone: false,
  workerReuseLocked: false,
  useGpu: true,
  workerMemoryMb: 1024,
  workerMemoryMaxMb: 16384,
  defaultFormat: null,
  theme: "system",
  modelAutoUpdate: true,
  writeReceipts: true,
  outputDestination: "same_folder",
  namingTemplate: "{name}.{ext}",
};

const now = Math.floor(Date.now() / 1000);

export const HISTORY: HistoryEntry[] = [
  { sourceName: "IMG_4790.heic", outputName: "IMG_4790.jpg", contentId: "3a1f…", outcome: "completed", reason: null, whenSecs: now - 3600, outputBytes: 1_204_881, receiptPath: "/Users/you/Pictures/IMG_4790.jpg.receipt.json" },
  { sourceName: "IMG_4791.heic", outputName: "IMG_4791.jpg", contentId: "8b22…", outcome: "completed", reason: null, whenSecs: now - 3600, outputBytes: 1_180_004, receiptPath: "/Users/you/Pictures/IMG_4791.jpg.receipt.json" },
  { sourceName: "invoice.pdf", outputName: "invoice.png", contentId: "c04d…", outcome: "failed", reason: "the best route needs the oc-pdf engine, which is not installed.", whenSecs: now - 86_400, outputBytes: 0, receiptPath: null },
  { sourceName: "archive.zip", outputName: "archive.tar", contentId: "77ae…", outcome: "completed", reason: null, whenSecs: now - 172_800, outputBytes: 8_442_112, receiptPath: "/Users/you/Downloads/archive.tar.receipt.json" },
  { sourceName: "notes.csv", outputName: "notes.json", contentId: "1de9…", outcome: "skipped", reason: null, whenSecs: now - 259_200, outputBytes: 0, receiptPath: null },
];

export const RECEIPTS: ReceiptEntry[] = [
  { id: "r-0001", date: new Date((now - 3600) * 1000).toISOString(), sourceName: "IMG_4790.heic", outputName: "IMG_4790.jpg", outputBytes: 1_204_881 },
  { id: "r-0002", date: new Date((now - 3610) * 1000).toISOString(), sourceName: "IMG_4791.heic", outputName: "IMG_4791.jpg", outputBytes: 1_180_004 },
  { id: "r-0003", date: new Date((now - 172_800) * 1000).toISOString(), sourceName: "archive.zip", outputName: "archive.tar", outputBytes: 8_442_112 },
];

/**
 * The model rows, mirroring `models.toml`.
 *
 * These used to be four invented ids that exist nowhere in the registry -- the
 * harness was previewing a product that was not this one.
 *
 * `blocked` is null on every row because every shipped row is pinned: the two
 * that were not are gone, rather than left in Settings behind a button that
 * refused every press. The branch that renders a reason stays, because the
 * next unpinned row will need it, and the refusal itself is covered in
 * `models.rs` against a fixture row the test build appends.
 */
export const MODELS: ModelInfo[] = [
  { id: "whisper-encoder", title: "whisper-encoder", purpose: "Speech-to-text, first half: audio to hidden states", sizeBytes: 23_201_314, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "whisper-decoder", title: "whisper-decoder", purpose: "Speech-to-text, second half: hidden states to tokens", sizeBytes: 53_310_178, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "whisper-tokenizer", title: "whisper-tokenizer", purpose: "Token vocabulary the decoder's ids index into", sizeBytes: 2_480_466, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "u2netp", title: "u2netp", purpose: "Foreground segmentation for background removal", sizeBytes: 4_574_861, licence: "Apache-2.0", downloaded: true, enabled: true, blocked: null },
  { id: "deepfilternet", title: "deepfilternet", purpose: "Speech enhancement: removes background noise", sizeBytes: 8_608_859, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "deepfilternet-aux", title: "deepfilternet-aux", purpose: "ERB matrices and analysis window for speech enhancement", sizeBytes: 126_976, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "realesrgan-x4", title: "realesrgan-x4", purpose: "Image super-resolution, x4", sizeBytes: 67_051_787, licence: "BSD-3-Clause", downloaded: true, enabled: false, blocked: null },
  { id: "silero-vad", title: "silero-vad", purpose: "Voice activity detection; keeps transcription off silence", sizeBytes: 2_327_524, licence: "MIT", downloaded: false, enabled: false, blocked: null },
  { id: "modnet", title: "modnet", purpose: "Higher-quality foreground matting (remove-background quality tier)", sizeBytes: 6_632_188, licence: "Apache-2.0", downloaded: false, enabled: false, blocked: null },
];

/**
 * The AI capabilities, mirroring `models.toml`'s `[[feature]]` table.
 *
 * Sizes are the real summed artifact sizes, and `usable` follows the same rule
 * the backend applies: a capability with no tool is listed and never offered.
 * The harness has drifted from the registry three times in this project's life,
 * and each time the UI worked in the browser and was dead in the product -- so
 * this stays row-for-row with the source of truth.
 */
export const AI_FEATURES: AiFeature[] = [
  // Row for row with `models.toml`, INCLUDING the rows that are not offered at
  // first run. The screen filters; the fixture does not, because Settings
  // shows every tier and the harness has to be able to show Settings.
  //
  // It had drifted again by the time this was written: the `best` tier was
  // missing entirely, three rows carried `tool: null` for tools that exist,
  // and the upscale line still claimed the model doubles as well as
  // quadruples. Every one of those made the browser show something the
  // product does not.
  { id: "denoise", title: "Clean up noisy audio", does: "Removes hiss and background noise from speech recordings.", sizeBytes: 8_735_835, downloaded: false, usable: true, licences: ["MIT"], tier: "small", tool: "audio-denoise", ready: false, active: false, offeredAtInstall: true, models: ["deepfilternet", "deepfilternet-aux"] },

  // THREE TIERS OF ONE TOOL, and only the middle one is a first-run offer.
  // `u2netp` appears in all three on purpose: it is the small tier, and it is
  // the fallback inside both of the others.
  //
  // `small` is off the first-run screen because its one artifact is already
  // inside `better`; `best` is off it because 224 MB is not a tick box. Both
  // are chosen in the tool, and both still appear in Settings.
  { id: "remove-background-small", tier: "small", tool: "image-remove-background", ready: true, active: false, title: "Remove image backgrounds", does: "Cuts the subject out of a photo and leaves the rest transparent.", sizeBytes: 4_574_861, downloaded: true, usable: true, licences: ["Apache-2.0"], offeredAtInstall: false, models: ["u2netp"] },
  { id: "remove-background-better", tier: "better", tool: "image-remove-background", ready: false, active: false, title: "Remove image backgrounds", does: "Sharper edges on hair and fur. Falls back to the small model when it finds no subject.", sizeBytes: 11_207_049, downloaded: false, usable: true, licences: ["Apache-2.0"], offeredAtInstall: true, models: ["modnet", "u2netp"] },
  { id: "remove-background-best", tier: "best", tool: "image-remove-background", ready: false, active: false, title: "Remove image backgrounds", does: "The most accurate cut-out, and much the slowest -- about a minute per image on a CPU.", sizeBytes: 228_579_949, downloaded: false, usable: true, licences: ["Apache-2.0", "MIT"], offeredAtInstall: false, models: ["birefnet-lite", "u2netp"] },

  { id: "upscale", title: "Enlarge images", does: "Quadruples an image's size without the usual blur.", sizeBytes: 67_051_787, downloaded: false, usable: true, licences: ["BSD-3-Clause"], tier: "small", tool: "image-upscale", ready: false, active: false, offeredAtInstall: true, models: ["realesrgan-x4"] },
  { id: "transcribe", title: "Transcribe speech", does: "Turns speech in an audio file into text you can read and search.", sizeBytes: 81_319_482, downloaded: false, usable: true, licences: ["MIT"], tier: "small", tool: "audio-transcribe", ready: false, active: false, offeredAtInstall: true, models: ["whisper-encoder", "whisper-decoder", "whisper-tokenizer", "silero-vad"] },
  {"id": "transcribe-large", "tier": "best", "title": "Transcribe speech", "does": "Noticeably more accurate, especially on accents and background noise. Much slower on a processor.", "tool": "audio-transcribe", "models": ["whisper-large-turbo-encoder", "whisper-large-turbo-decoder", "whisper-tokenizer", "silero-vad"], "sizeBytes": 1089315036, "downloaded": false, "usable": true, "ready": false, "active": false, "licences": ["MIT"], "offeredAtInstall": false},
  { id: "ocr", title: "Read text in images", does: "Finds the words in a photo or scan and writes them to a text file.", sizeBytes: 138_662_763, downloaded: false, usable: true, licences: ["Apache-2.0"], tier: "small", tool: "image-ocr", ready: false, active: false, offeredAtInstall: true, models: ["paddleocr-det", "paddleocr-rec", "paddleocr-dict"] },
];

/**
 * The signature library starts EMPTY.
 *
 * A pre-loaded "Jane Smith" would be a fabricated document artefact sitting in
 * a list the user is about to place on a contract. The empty state is also the
 * one that needs previewing most — it is what every new user sees, and it is
 * where the "Add new signature" path has to be discoverable.
 */
export const SIGNATURES: Signature[] = [];

// IN THE ORDER THE APP SHOWS THEM, which is `BY_FREQUENCY` in `tools.rs`.
//
// The harness returns this array as-is rather than through `list_tools()`, so
// the sort that orders the real rail does not reach it -- the browser showed
// the rail in the order these rows happened to be written, and the app showed
// it by frequency. `the_harness_offers_what_the_app_offers` compares the two
// as SEQUENCES for that reason, so this cannot drift back.
export const TOOLS: ToolDescriptor[] = [
  // Mirrors `crates/openconvert-run/src/tools.rs` exactly — same ids, same
  // categories, same preview_only flags. When the two drift, the harness stops
  // being a preview of the product and becomes a second product.
  { id: "image-remove-background", category: "image", title: "Remove background", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "image-compress", category: "image", title: "Compress", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [{ id: "quality", title: "Compression", kind: "choice", required: false, options: [{ value: "85", label: "Low" }, { value: "65", label: "Medium" }, { value: "20", label: "High" }], default: "65" }] },
  { id: "image-upscale", category: "image", title: "Upscale", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "image-ocr", category: "image", title: "Read text in image", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "image-pick", category: "image", title: "Colour picker", available: true, multiInput: false, previewOnly: true, unavailableReason: null, params: [] },
  { id: "image-invert", category: "image", title: "Invert colours", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "image-greyscale", category: "image", title: "Black and white", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },


  { id: "pdf-compress", category: "pdf", title: "Compress", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [{ id: "quality", title: "Compression", kind: "choice", required: false, options: [{ value: "lossless", label: "Low" }, { value: "85", label: "Medium" }, { value: "45", label: "High" }], default: "85" }] },
  { id: "pdf-merge", category: "pdf", title: "Merge documents", available: true, multiInput: true, previewOnly: false, unavailableReason: null, params: [] },
  // SIX PDF TOOLS. The harness had four and the registry had eleven, which
  // is how the browser came to show a rail nobody's app had; the registry then
  // folded Extract, Remove, Rotate and Crop into "Modify pages" and Remove
  // password into Password. Both moves are mirrored here because
  // `the_harness_offers_what_the_app_offers` compares the two as sequences.
  { id: "pdf-split", category: "pdf", title: "Split", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [
    { id: "boundaries", title: "Split after pages, like 2,5,10", kind: "text", required: false },
  ] },
  { id: "pdf-reorder", category: "pdf", title: "Modify pages", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [
    { id: "order", title: "New order (e.g. 3,1,2)", kind: "text", required: false },
  ] },
  { id: "pdf-stamp", category: "pdf", title: "Sign", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [
    { id: "page", title: "Page", kind: "number", required: true },
    { id: "image", title: "Image file (transparent PNG)", kind: "text", required: true },
  ] },
  { id: "pdf-protect", category: "pdf", title: "Password", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [
    { id: "direction", title: "Password", kind: "choice", required: false, options: [{ value: "add", label: "Add" }, { value: "remove", label: "Remove" }], default: "add" },
    { id: "password", title: "Password", kind: "text", required: true },
    { id: "owner_password", title: "Owner password (optional)", kind: "text", required: false },
  ] },
  { id: "pdf-text", category: "pdf", title: "Extract text", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "audio-denoise", category: "audio", title: "Remove background noise", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },
  { id: "audio-transcribe", category: "audio", title: "Transcribe to text", available: true, multiInput: false, previewOnly: false, unavailableReason: null, params: [] },

];
