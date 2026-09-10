<!--
  One dropped file, one row.

  Name and size on the left, the conversion in the middle, and the two controls
  that act on this file alone on the right: a ⋯ menu for everything occasional,
  and a bin that removes it from the drop.

  **The plan lives behind the ⋯, not in a card of its own.** A row per file is
  what makes a drop of forty legible; the step-by-step preview is still the
  product's distinctive claim, so it is one click away rather than deleted.
-->
<script module lang="ts">
  /**
   * HOW MANY ROWS MAY RENDER AT ONCE, across every row on the screen.
   *
   * A preview is a worker run: a confined process, a decode, a rasterise. One
   * per row is fine for a row; a folder of two hundred photographs dropped at
   * once is two hundred of them started in the same tick, which is a machine
   * that stops responding while it renders thumbnails nobody has scrolled to
   * yet.
   *
   * Four, matching the workspace's own `RENDER_LANES` -- the same reasoning
   * about the same worker pool, and there is no sense in two answers.
   *
   * Combined with the observer below, the real bound is tighter still: only
   * rows that have been ON SCREEN ask at all.
   */
  const LANES = 4;
  let running = 0;
  const waiting: (() => void)[] = [];

  async function lane<T>(work: () => Promise<T>): Promise<T> {
    if (running >= LANES) await new Promise<void>((r) => waiting.push(r));
    running += 1;
    try {
      return await work();
    } finally {
      running -= 1;
      waiting.shift()?.();
    }
  }
</script>

<script lang="ts">
  import { previewFile, type PlanPreview, type ProbeResult, type Suggestion } from "../lib/ipc";
  import FormatPicker from "./FormatPicker.svelte";
  import PlanTable from "./PlanTable.svelte";

  let {
    probe,
    options,
    selected,
    plan = null,
    planning = false,
    checked = true,
    lockedTo = null,
    bare = false,
    resultLabel = null,
    onselect,
    onchecked,
    onremove,
    onpreview,
  }: {
    probe: ProbeResult;
    options: Suggestion[];
    selected: number;
    plan?: PlanPreview | null;
    planning?: boolean;
    checked?: boolean;
    /**
     * The format the whole batch is being converted to, or null.
     *
     * Non-null means the shared control above the list is governing: this row's
     * own menu is disabled, because two controls that set the same value are
     * two places for it to disagree.
     */
    lockedTo?: string | null;
    /**
     * Render the file WITHOUT the conversion half.
     *
     * The tools workspace shows a batch too, and it had grown a list of its
     * own -- a different row, a different thumbnail, its own sizing and its
     * own loading behaviour, for the job this row already does. Two lists of
     * files in one app that do not look alike is one list too many.
     *
     * What `bare` removes is what does not apply there: the `FROM -> TO`
     * control, because a tool run has no target format to choose, and the
     * per-row menu, whose entries are all about a conversion plan. What stays
     * is what was wanted -- the thumbnail, the name, the size, the tick and
     * the bin.
     */
    bare?: boolean;
    resultLabel?: string | null;
    onselect: (index: number) => void;
    onchecked: (checked: boolean) => void;
    onremove: () => void;
    onpreview?: () => void;
  } = $props();

  let menuOpen = $state(false);
  let detailsOpen = $state(false);
  let root = $state<HTMLDivElement | null>(null);

  /**
   * THE ROW SHOWS THE FILE, not a glyph for its category.
   *
   * Every image row carried the same square, every PDF the same rectangle, so
   * a list of twelve photographs was twelve identical marks and the only thing
   * telling them apart was a filename out of a camera. The thumbnail is the
   * one piece of information a person actually recognises.
   *
   * `null` until it arrives, and it may never arrive: audio, archives and
   * anything the backend cannot rasterise keep the glyph, which is a real
   * answer for them rather than a failure.
   */
  let thumb = $state<string | null>(null);
  let expanded = $state<string | null>(null);
  let expanding = $state(false);

  /** Kinds the backend can draw. Asking for the others is a refusal per row. */
  const RASTERISABLE = new Set(["image", "document"]);

  /**
   * Ask only once the row has been on screen.
   *
   * A drop of two hundred files mounts two hundred of these, and rendering a
   * thumbnail for one at the bottom of a list nobody has scrolled to is work
   * spent on something nobody has looked at. The observer is disconnected
   * after the first sighting -- this asks once, and the answer does not change.
   */
  function whenSeen(node: HTMLElement) {
    if (!RASTERISABLE.has(facts?.kind ?? "unknown")) return;
    const io = new IntersectionObserver((entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      io.disconnect();
      void lane(async () => {
        try {
          // 96 px: the slot is 34, and twice that covers a 2x display with
          // room to spare. A 1024 px render per row would be the stampede
          // this is avoiding, in a different form.
          const rendered = await previewFile(probe.path, 1, 96, []);
          thumb = rendered.dataUri;
        } catch {
          // The glyph stands. A row is not the place to report that a
          // thumbnail could not be drawn -- the conversion itself will say so,
          // with a reason, if the file is genuinely unreadable.
        }
      });
    });
    io.observe(node);
    return { destroy: () => io.disconnect() };
  }

  /** Open the bigger render, fetched on demand rather than kept per row. */
  async function expand() {
    if (onpreview) { onpreview(); return; }
    if (thumb === null || expanding) return;
    expanding = true;
    try {
      const rendered = await lane(() => previewFile(probe.path, 1, 900, []));
      expanded = rendered.dataUri;
    } catch {
      // Falling back to the thumbnail rather than showing nothing: it is the
      // same picture, smaller, which is still an answer to "which file is
      // this".
      expanded = thumb;
    } finally {
      expanding = false;
    }
  }

  const facts = $derived(probe.properties);
  const blocked = $derived(plan !== null && !plan.executable);

  const glyphs: Record<string, string> = {
    image: "▨",
    audio: "♪",
    video: "▶",
    document: "▤",
    archive: "▣",
    tabular: "▦",
    unknown: "◇",
  };

  function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const units = ["KB", "MB", "GB"];
    let v = n / 1024;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i += 1;
    }
    return `${v.toFixed(v < 10 && i > 0 ? 1 : 0)} ${units[i]}`;
  }

  /** "92 KB · WEBP Image" — size, then what it actually is. */
  const subtitle = $derived.by(() => {
    const kind = facts?.kind ?? "unknown";
    const noun = kind === "unknown" ? "file" : kind === "image" ? "Image" : kind;
    const dims = facts?.width && facts?.height ? ` · ${facts.width}×${facts.height}` : "";
    return `${bytes(probe.inputBytes)} · ${probe.detected.toUpperCase()} ${noun}${dims}`;
  });

  function onWindowClick(e: MouseEvent) {
    if (menuOpen && root && !root.contains(e.target as Node)) menuOpen = false;
  }
</script>

<svelte:window onclick={onWindowClick} />

<div class="row" class:blocked bind:this={root}>
  <input
    class="bulk-check"
    type="checkbox"
    {checked}
    aria-label={`Select ${probe.fileName} ${bare ? "for processing" : "for a bulk format change"}`}
    disabled={lockedTo !== null}
    onchange={(event) => onchecked(event.currentTarget.checked)}
  />
  <!-- SAME SIZE, SAME PLACE. The thumbnail replaces the glyph inside the
       existing 34 px square rather than beside it, so a list where some rows
       have a preview and some do not still lines up. -->
  {#if thumb}
    <button
      type="button"
      class="icon thumb"
      onclick={expand}
      aria-label={`Show a larger preview of ${probe.fileName}`}
      aria-busy={expanding}
    >
      <img src={thumb} alt="" />
    </button>
  {:else}
    <span class="icon" use:whenSeen aria-hidden="true">
      {glyphs[facts?.kind ?? "unknown"]}
    </span>
  {/if}

  <span class="ident">
    <span class="name" title={probe.fileName}>{probe.fileName}</span>
    <span class="meta">{resultLabel ?? subtitle}</span>
  </span>

  <!-- A DIV, NOT A SPAN. This was a `span` containing `<div class="more">`,
       which is invalid nesting: the parser closes the span and reparents the
       div, so `.controls` lost half its children and its flex rules stopped
       applying to them. That is the row misalignment, not a spacing value.

       The "⟳ Convert" label that stood here is gone: the row already reads
       `PDF → PNG`, and the verb is the button at the bottom of the screen. -->
  <!-- The conversion half, which a tool run has no use for: there is no
       target format to pick, and every entry behind the menu is about a
       conversion plan. See `bare`. -->
  <div class="controls">
    {#if !bare}
      <span class="from">{probe.detected.toUpperCase()}</span>
      <span class="arrow" aria-hidden="true">→</span>

      <FormatPicker
        {options}
        {selected}
        {onselect}
        disabled={options.length === 0 || lockedTo !== null}
      />
    {/if}

  {#if !bare}
  <div class="more">
    <button
      type="button"
      class="icon-btn"
      aria-haspopup="menu"
      aria-expanded={menuOpen}
      aria-label={`More options for ${probe.fileName}`}
      onclick={() => (menuOpen = !menuOpen)}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
        <circle cx="3.2" cy="8" r="1.4" fill="currentColor" />
        <circle cx="8" cy="8" r="1.4" fill="currentColor" />
        <circle cx="12.8" cy="8" r="1.4" fill="currentColor" />
      </svg>
    </button>

    {#if menuOpen}
      <ul class="menu" role="menu">
        <li>
          <button
            type="button"
            role="menuitem"
            onclick={() => {
              detailsOpen = !detailsOpen;
              menuOpen = false;
            }}
          >
            {detailsOpen ? "Hide what will happen" : "Show what will happen"}
          </button>
        </li>
        <li>
          <button type="button" role="menuitem" disabled>Convert this file only</button>
        </li>
        <li>
          <button
            type="button"
            role="menuitem"
            class="danger"
            onclick={() => {
              menuOpen = false;
              onremove();
            }}
          >
            Remove from list
          </button>
        </li>
      </ul>
    {/if}
  </div>
  {/if}

  <!-- THE BIN IS NOT PART OF THE MENU. It is a sibling of it, and it stays in
       `bare` mode: removing a file from the list is exactly as meaningful for
       a batch of tool runs as it is for a batch of conversions. -->
  <button
    type="button"
    class="icon-btn bin"
    aria-label={`Remove ${probe.fileName}`}
    onclick={onremove}
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
</div>

{#if probe.mismatched || probe.polyglot || blocked}
  <p class="flag" class:critical={probe.polyglot || blocked}>
    {#if probe.polyglot}
      More than one format signature matched. This file is quarantined.
    {:else if blocked}
      {plan?.warnings.find((w) => w.blocking)?.message ?? "Nothing would run for this file."}
    {:else}
      The name says {probe.declared?.toUpperCase()} and the content says
      {probe.detected.toUpperCase()}; routing by content.
    {/if}
  </p>
{/if}

{#if expanded}
  <!-- A DIALOG OVER THE WINDOW, not a popover pinned to a 34 px square. The
       row is 48 px tall inside a scrolling list; anything anchored to it is
       either clipped by the list or big enough to cover the rows the user
       opened it to compare against. -->
  <div
    class="expanded"
    role="dialog"
    aria-label={`Preview of ${probe.fileName}`}
    tabindex="-1"
    onclick={() => (expanded = null)}
    onkeydown={(e) => {
      if (e.key === "Escape" || e.key === "Enter" || e.key === " ") expanded = null;
    }}
  >
    <img src={expanded} alt={`Preview of ${probe.fileName}`} />
    <span class="expanded-name">{probe.fileName}</span>
  </div>
{/if}

{#if detailsOpen}
  <div class="details">
    {#if planning}
      <p class="planning" role="status">Working out what would happen…</p>
    {:else if plan}
      <PlanTable {plan} />
    {/if}
  </div>
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    min-height: 56px;
    /* Below the width the controls need, they take a line of their own rather
       than overflowing. A row that scrolls sideways takes the whole list with
       it, and the list's backgrounds stop at the fold. */
    flex-wrap: wrap;
  }

  /* The controls travel together — wrapping them one at a time would scatter
     the conversion across two lines in the wrong order. */
  .controls {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex: 0 1 auto;
    margin-left: auto;
  }

  .icon {
    flex: none;
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    font-size: 15px;
    color: var(--text-secondary);
    /* The checkerboard shows through a transparent PNG, which is the one
       case where a thumbnail on a flat ground is unreadable. */
    overflow: hidden;
    padding: 0;
  }

  button.icon {
    border: 0.5px solid var(--hairline);
    cursor: zoom-in;
  }

  button.icon:hover {
    border-color: var(--hairline-strong);
  }

  .thumb img {
    inline-size: 100%;
    block-size: 100%;
    object-fit: cover;
    display: block;
  }

  .expanded {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: grid;
    place-items: center;
    gap: var(--space-4);
    align-content: center;
    padding: var(--space-8);
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    cursor: zoom-out;
  }

  .expanded img {
    max-inline-size: min(92vw, 900px);
    max-block-size: 78vh;
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-float);
    background: var(--surface-raised);
  }

  .expanded-name {
    font-size: var(--text-small-size);
    color: var(--text-secondary);
    max-inline-size: 60ch;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .bulk-check {
    flex: none;
    width: 16px;
    height: 16px;
    accent-color: var(--emphasis);
  }

  .ident {
    display: grid;
    gap: 1px;
    /* `0` lets the name ellipsise instead of forcing the row wider; the basis
       is the point below which the controls wrap instead. */
    min-width: 0;
    flex: 1 1 160px;
  }

  .name {
    font-size: var(--text-body-size);
    line-height: var(--text-body-lh);
    font-weight: var(--weight-medium);
    /* One line, ellipsised: a 90-character name must not push the controls
       off the row or reflow it to three lines. */
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .from {
    flex: none;
    padding: 0 var(--space-3);
    min-height: 24px;
    display: inline-flex;
    align-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    font-size: var(--text-caption-size);
    font-weight: var(--weight-medium);
    letter-spacing: 0.02em;
    color: var(--text-secondary);
  }

  .arrow {
    flex: none;
    color: var(--text-tertiary);
  }

  .icon-btn {
    flex: none;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    transition:
      color var(--motion-micro),
      background var(--motion-micro);
  }

  .icon-btn svg {
    width: 15px;
    height: 15px;
  }

  .icon-btn:hover {
    background: var(--surface-sunken);
    color: var(--text-primary);
  }

  .bin:hover {
    color: var(--accent-critical);
  }

  .more {
    position: relative;
    flex: none;
  }

  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    right: 0;
    min-width: 196px;
    list-style: none;
    margin: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-float);
  }

  .menu button {
    width: 100%;
    min-height: 30px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-sm);
    text-align: left;
    font-size: var(--text-small-size);
    color: var(--text-primary);
    white-space: nowrap;
  }

  .menu button:hover:not(:disabled) {
    background: var(--surface-sunken);
  }

  .menu button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .menu .danger {
    color: var(--accent-critical);
  }

  .flag {
    margin: 0;
    padding: 0 var(--space-4) var(--space-3) 60px;
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
    color: var(--accent-attention);
  }

  .flag.critical {
    color: var(--accent-critical);
  }

  .details {
    padding: 0 var(--space-4) var(--space-4) 60px;
  }

  .planning {
    margin: 0;
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  @media (prefers-reduced-motion: reduce) {
    .icon-btn {
      transition: none;
    }
  }
</style>
