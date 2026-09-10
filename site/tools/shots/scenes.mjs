/**
 * The scenes. **Not shipped** — this drives the desktop UI preview harness and
 * produces the screenshots the site is built around (13-SITE-REBUILD §4.3).
 *
 * Every scene is a click path through `apps/desktop/preview`, which mounts the
 * *real* `src/App.svelte` with the real components and the real `tokens.css`
 * and only swaps the four `@tauri-apps/api` entry points for fixtures. So a
 * shot here is a photograph of the shipping UI, not a mock of it — the one
 * thing that keeps the site from drifting away from the product.
 *
 * `alt` is authored here, next to the click path that produces the picture.
 * A screenshot with machine-written alt text is an accessibility failure with
 * a checkbox next to it.
 */

/** The app opens on the first-run AI sheet. Most scenes are behind it. */
const skipSetup = async (page) => {
  const notNow = page.getByRole("button", { name: "Not now" });
  await notNow.waitFor({ state: "visible" });
  await notNow.click();
};

/** Load the five fixture files through the mocked picker. */
const dropFiles = async (page) => {
  await page.getByRole("button", { name: /Drop files to convert/ }).click();
  await page.getByRole("button", { name: /^Convert/ }).waitFor({ state: "visible" });
  // The per-row format pickers settle a frame after the rows do.
  await page.waitForTimeout(400);
};

/** Open a tool from the title-bar menu. */
const openTool = async (page, name) => {
  await page.getByRole("button", { name: /^Tools/ }).click();
  await page.getByRole("menuitem", { name }).or(page.getByRole("button", { name })).first().click();
  await page.locator(".workspace").waitFor({ state: "visible" });
};

/** Feed the tool workspace its fixture file. */
const openToolFile = async (page, settled) => {
  await page.locator(".stage-drop").click();
  // `.first()` deliberately: the status line and the rail label can both carry
  // the word, and "the workspace has settled" is true when either appears.
  await page.getByText(settled).first().waitFor({ state: "visible", timeout: 30_000 });
  await page.waitForTimeout(600);
};

export const scenes = [
  {
    id: "ai-features",
    alt:
      "The OpenConvert first-run panel headed “Add AI features?”, listing four optional on-device models " +
      "with their size on disk and licence, none of them installed, and a “Not now” button.",
    async run(page) {
      await page.getByRole("heading", { name: /Add AI features/ }).waitFor({ state: "visible" });
      await page.waitForTimeout(300);
    },
  },

  {
    id: "drop-idle",
    alt:
      "The empty OpenConvert window: one large dashed drop area reading “Drop files to convert”, " +
      "with “or click to choose · Ctrl+O” beneath it.",
    async run(page) {
      await skipSetup(page);
      await page.waitForTimeout(300);
    },
  },

  {
    id: "batch-plan",
    alt:
      "Five dropped files listed in OpenConvert, each showing its detected format and the target it will " +
      "convert to — three HEIC photos to JPEG, a file named screenshot.png flagged “The name says PNG " +
      "and the content says JPEG; routing by content”, and a FLAC recording to WAV.",
    async run(page) {
      await skipSetup(page);
      await dropFiles(page);
    },
  },

  {
    id: "progress",
    alt:
      "A conversion running in OpenConvert: a progress bar, the heading “4 of 5”, four rows marked done, " +
      "one marked started, and a Cancel button.",
    async run(page) {
      await skipSetup(page);
      await dropFiles(page);
      await page.getByRole("button", { name: /^Convert/ }).click();
      await page.getByRole("button", { name: "Cancel" }).waitFor({ state: "visible" });
      await page.getByText("4 of 5").waitFor({ state: "visible", timeout: 30_000 });
    },
  },

  {
    id: "batch-done",
    alt:
      "The OpenConvert results screen: “4 converted · 1 failed · 5 total”, elapsed time and bytes in and " +
      "out, each converted file listed with its operation class and duration, and one honest failure — " +
      "“libheif could not decode this file: unsupported colour profile”.",
    async run(page) {
      await skipSetup(page);
      await dropFiles(page);
      await page.getByRole("button", { name: /^Convert/ }).click();
      await page.getByText(/converted/).waitFor({ state: "visible", timeout: 60_000 });
      await page.waitForTimeout(600);
    },
  },

  {
    id: "receipt-open",
    alt:
      "A conversion receipt expanded inside OpenConvert, listing the engine and its version, the exact " +
      "parameters, the operation class, the limits and sandbox profile the job ran under, the hash of " +
      "the bytes read, and the number of network calls made.",
    async run(page) {
      await skipSetup(page);
      await dropFiles(page);
      await page.getByRole("button", { name: /^Convert/ }).click();
      await page.getByText(/converted/).waitFor({ state: "visible", timeout: 60_000 });
      await page.locator(".results button").filter({ hasText: "" }).first().waitFor();
      // The disclosure at the end of the first result row.
      await page.locator(".results [aria-expanded]").first().click();
      await page.waitForTimeout(500);
    },
  },

  {
    id: "tools-menu",
    alt:
      "The OpenConvert Tools menu open, grouped Image, Audio and PDF: remove background, upscale ×4, " +
      "invert colours, black and white, colour picker, remove background noise, transcribe to text, " +
      "merge documents, reorder pages, and sign.",
    async run(page) {
      await skipSetup(page);
      await page.getByRole("button", { name: /^Tools/ }).click();
      await page.waitForTimeout(400);
    },
  },

  {
    id: "remove-bg",
    alt:
      "The OpenConvert background-removal workspace: the image on a transparency checkerboard with the " +
      "subject cut out, a before/after toggle, zoom controls, and the status line “Background removed, " +
      "now transparent”.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Remove background");
      await openToolFile(page, /Background removed/);
    },
  },

  {
    id: "upscale",
    alt:
      "The OpenConvert upscaling workspace showing an image enlarged four times on device, with zoom " +
      "controls and a before/after toggle.",
    async run(page) {
      await skipSetup(page);
      // "Upscale x4" until the factor became a parameter and the menu entry
      // became plain "Upscale". A scene naming the old label clicks nothing and
      // times out, which is what it did.
      await openTool(page, "Upscale");
      // The upscale workspace's status line is the image's DIMENSIONS -- it
      // has no "Upscaled" message -- so that readout is what says it settled.
      // Waiting on words the UI never prints is why this scene timed out.
      await openToolFile(page, /\d+\s*×\s*\d+/);
    },
  },

  {
    id: "transcribe",
    alt:
      "The OpenConvert transcription workspace: a speech recording with its waveform and the transcript " +
      "produced from it on this machine.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Transcribe to text");
      await openToolFile(page, /Transcri/);
    },
  },

  {
    id: "pdf-merge",
    alt:
      "The OpenConvert document workspace with two PDFs loaded, their pages laid out as thumbnails ready " +
      "to be merged and reordered.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Merge documents");
      await openToolFile(page, /page|Page|merge|Merge/);
    },
  },

  {
    id: "colour-picker",
    alt:
      "The OpenConvert colour picker: an image open in the workspace with a sampled colour and its value " +
      "shown beside it.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Colour picker");
      await openToolFile(page, /colour|Colour|#/);
    },
  },

  {
    id: "pdf-reorder",
    alt:
      "The OpenConvert page-reordering workspace: a document's pages listed so they can be moved or " +
      "dropped before the file is written.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Reorder pages");
      await openToolFile(page, /page|Page/);
    },
  },

  {
    id: "denoise",
    alt:
      "The OpenConvert noise-removal workspace: a speech recording with its waveform, cleaned up on this " +
      "machine.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Remove background noise");
      await openToolFile(page, /noise|Noise|denois|Denois|clean/);
    },
  },

  {
    id: "invert",
    alt: "The OpenConvert image workspace with an image's colours inverted, before and after available.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Invert colours");
      await openToolFile(page, /Invert|invert/);
    },
  },

  {
    id: "black-and-white",
    alt: "The OpenConvert image workspace converting an image to black and white on this machine.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Black and white");
      await openToolFile(page, /[Bb]lack and white|greyscale|grayscale/);
    },
  },

  {
    id: "sign",
    alt: "The OpenConvert signing workspace: a signature kept on this machine, placed on a document page.",
    async run(page) {
      await skipSetup(page);
      await openTool(page, "Sign");
      await openToolFile(page, /[Ss]ign/);
    },
  },

  {
    id: "settings-models",
    alt:
      "The OpenConvert settings screen, showing each on-device model with its size, licence, an enable " +
      "switch, and a control that deletes it and reclaims the disk space.",
    async run(page) {
      await skipSetup(page);
      await page.getByRole("button", { name: "Settings" }).click();
      await page.waitForTimeout(600);
      const models = page.getByRole("heading", { name: /Models/i }).first();
      if (await models.count()) await models.scrollIntoViewIfNeeded();
      await page.waitForTimeout(400);
    },
  },
];
