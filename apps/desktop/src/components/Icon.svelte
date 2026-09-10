<!--
  Icons, from a real set.

  **Hand-drawn SVG paths have now been wrong twice**, and before that they were
  text glyphs — `⚙`, `▱`, `⌄`, `⟳` — rendered in the UI font, which is why the
  gear looked like a blob and the "show in folder" button looked like a
  rectangle. It was a rectangle: U+25B1 WHITE PARALLELOGRAM.

  These are **Lucide** (ISC licence, credited in NOTICE), vendored as path data
  rather than pulled from a package. Vendoring is deliberate:

  - No runtime, no dependency, no network. The CSP forbids remote assets and
    the app is offline by design; an icon font or a CDN sprite is not available
    to us even if we wanted one.
  - Only the icons actually used are here. A whole set would be dead weight in
    a binary that ships its own font already.

  Every icon is drawn on a 24×24 grid with `stroke-width: 2`, which is Lucide's
  own geometry — mixing that with a hand-drawn 16×16 path is what made the
  earlier set look inconsistent at small sizes. `currentColor` throughout, so
  colour is the caller's business and both themes work without a second copy.

  Adding one: copy the `<path>` data out of the Lucide source for that icon,
  keep the 24×24 viewBox, and add a line to `PATHS`.
-->
<script module lang="ts">
  /** The icons this app uses, by Lucide's own name for them.

      In the MODULE script, not the instance one: a type exported from the
      instance script belongs to the component's own scope and cannot be
      imported. The rail names its icons with this union, so it has to be
      reachable from outside. */
  export type IconName =
    | "settings"
    | "folder-open"
    | "chevron-down"
    | "trash-2"
    | "upload"
    | "eye"
    | "copy"
    | "check"
    | "x"
    | "refresh-cw"
    | "audio-waveform"
    | "type"
    | "layers"
    | "arrow-up-down"
    | "pen-line"
    | "sun"
    | "moon"
    | "pipette"
    | "image-off"
    | "eraser"
    | "droplet-off"
    | "minimize-2"
    | "scan-text"
    | "file-text"
    | "file-stack"
    | "square-split-vertical"
    | "shield"
    | "scaling"
    | "contrast"
    | "history"
    | "undo-2";
</script>

<script lang="ts">
  /** Path data only — the wrapper below supplies the shared attributes. */
  const PATHS: Record<IconName, string[]> = {
    settings: [
      "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
    ],
    // Lucide `eraser`: background removal. The rail had U+25EB, a square with
    // a line down it, which is not a thing anyone associates with anything.
    eraser: [
      "M21 21H8a2 2 0 0 1-1.42-.587l-3.994-3.999a2 2 0 0 1 0-2.828l10-10a2 2 0 0 1 2.829 0l5.999 6a2 2 0 0 1 0 2.828L12.834 21",
      "m5.082 11.09 8.828 8.828",
    ],
    // Lucide `droplet-off`: greyscale, i.e. colour taken out.
    "droplet-off": [
      "M18.715 13.186C18.29 11.858 17.384 10.607 16 9.5c-2-1.6-3.5-4-4-6.5a10.7 10.7 0 0 1-.884 2.586",
      "m2 2 20 20",
      "M8.795 8.797A11 11 0 0 1 8 9.5C6 11.1 5 13 5 15a7 7 0 0 0 13.222 3.208",
    ],
    // Lucide `minimize-2`: compression, for both the image and the PDF tool.
    // The same operation on two kinds of file gets the same mark.
    "minimize-2": ["m14 10 7-7", "M20 10h-6V4", "m3 21 7-7", "M4 14h6v6"],
    // Lucide `scan-text`: reading text out of a picture. THIS IS THE ONE THAT
    // HAD NO ENTRY AT ALL, so it fell through to the middle-dot fallback.
    "scan-text": [
      "M3 7V5a2 2 0 0 1 2-2h2",
      "M17 3h2a2 2 0 0 1 2 2v2",
      "M21 17v2a2 2 0 0 1-2 2h-2",
      "M7 21H5a2 2 0 0 1-2-2v-2",
      "M7 8h8",
      "M7 12h10",
      "M7 16h6",
    ],
    // Lucide `file-text`: transcription, whose output is a text file.
    "file-text": [
      "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",
      "M14 2v5a1 1 0 0 0 1 1h5",
      "M10 9H8",
      "M16 13H8",
      "M16 17H8",
    ],
    // Lucide `square-split-vertical`: one document cut across into parts.
    "square-split-vertical": [
      "M2 12h20",
      "M21 16v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-3",
      "M3 8V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v3",
    ],
    // Lucide `shield`: the password tool, which adds one or takes it away.
    // Not `lock`, whose shackle is a `<rect>` -- this file carries path data
    // only, and converting a rounded rectangle by hand is exactly the
    // hand-drawing its header warns about.
    shield: [
      "M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z",
    ],
    // Lucide `file-stack`: several documents becoming one.
    "file-stack": [
      "M11 21a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1",
      "M16 16a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1V8a1 1 0 0 1 1-1",
      "M21 6a2 2 0 0 0-.586-1.414l-2-2A2 2 0 0 0 17 2h-3a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h6a1 1 0 0 0 1-1z",
    ],
    // Lucide `history`: a clock with an arrow curving back into it.
    history: [
      "M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8",
      "M3 3v5h5",
      "M12 7v5l4 2",
    ],
    // Lucide `undo-2`: an arrow turning back on itself.
    "undo-2": [
      "M9 14 4 9l5-5",
      "M4 9h10.5a5.5 5.5 0 0 1 5.5 5.5a5.5 5.5 0 0 1-5.5 5.5H11",
    ],
    "folder-open": [
      "m6 14 1.5-2.9A2 2 0 0 1 9.24 10H20a2 2 0 0 1 1.94 2.5l-1.54 6a2 2 0 0 1-1.95 1.5H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h3.9a2 2 0 0 1 1.69.9l.81 1.2a2 2 0 0 0 1.67.9H18a2 2 0 0 1 2 2v2",
    ],
    "chevron-down": ["m6 9 6 6 6-6"],
    "trash-2": [
      "M3 6h18",
      "M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6",
      "M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2",
      "M10 11v6",
      "M14 11v6",
    ],
    upload: ["M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4", "M17 8l-5-5-5 5", "M12 3v12"],
    eye: [
      "M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0",
      "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
    ],
    copy: [
      "M20 8h-8a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2v-8a2 2 0 0 0-2-2z",
      "M4 16c-1.1 0-2-.9-2-2V4c0-1.1.9-2 2-2h10c1.1 0 2 .9 2 2",
    ],
    check: ["M20 6 9 17l-5-5"],
    x: ["M18 6 6 18", "M6 6l12 12"],
    "refresh-cw": [
      "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8",
      "M21 3v5h-5",
      "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16",
      "M8 16H3v5",
    ],
    "audio-waveform": [
      "M2 13a2 2 0 0 0 2-2V7a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0V4a2 2 0 0 1 4 0v13a2 2 0 0 0 4 0v-4a2 2 0 0 1 2-2",
    ],
    type: ["M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2", "M9 20h6", "M12 4v16"],
    layers: [
      "m12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83z",
      "M2 12.4a1 1 0 0 0 .6.9l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 .58-.91",
      "M2 17.4a1 1 0 0 0 .6.9l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 .58-.91",
    ],
    "arrow-up-down": ["m21 16-4 4-4-4", "M17 20V4", "m3 8 4-4 4 4", "M7 4v16"],
    "pen-line": [
      "M12 20h9",
      "M16.376 3.622a1 1 0 0 1 3.002 3.002L7.368 18.635a2 2 0 0 1-.855.506l-2.872.838a.5.5 0 0 1-.62-.62l.838-2.872a2 2 0 0 1 .506-.854z",
    ],
    sun: [
      "M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10z",
      "M12 2v2",
      "M12 20v2",
      "m4.93 4.93 1.41 1.41",
      "m17.66 17.66 1.41 1.41",
      "M2 12h2",
      "M20 12h2",
      "m6.34 17.66-1.41 1.41",
      "m19.07 4.93-1.41 1.41",
    ],
    moon: ["M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"],
    pipette: [
      "m2 22 1-1h3l9-9",
      "M3 21v-3l9-9",
      "m15 6 3.4-3.4a2.1 2.1 0 1 1 3 3L18 9l.4.4a2.1 2.1 0 1 1-3 3l-3.8-3.8a2.1 2.1 0 1 1 3-3l.4.4z",
    ],
    "image-off": [
      "M10.41 10.41a2 2 0 1 1-2.83-2.83",
      "M13.5 13.5 6 21",
      "M18 12l-1.5-1.5",
      "M2 2l20 20",
      "M21 15V5a2 2 0 0 0-2-2H9",
      "M3.59 3.59A2 2 0 0 0 3 5v14a2 2 0 0 0 2 2h14a2 2 0 0 0 1.41-.59",
    ],
    scaling: [
      "M12 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7",
      "M16 3h5v5",
      "M14 10 21 3",
    ],
    contrast: ["M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z", "M12 18a6 6 0 0 0 0-12v12z"],
  };

  let {
    name,
    size = 16,
    label = null,
  }: {
    name: IconName;
    /** Rendered size in px. The grid is 24; this scales it. */
    size?: number;
    /**
     * An accessible name.
     *
     * Null means decorative, which is the common case: almost every icon here
     * sits inside a button that already carries an `aria-label`, and naming it
     * twice makes a screen reader say it twice.
     */
    label?: string | null;
  } = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden={label === null ? "true" : undefined}
  aria-label={label ?? undefined}
  role={label === null ? undefined : "img"}
  focusable="false"
>
  {#each PATHS[name] as d (d)}
    <path {d} />
  {/each}
</svg>

<style>
  svg {
    display: block;
    flex: none;
  }
</style>
