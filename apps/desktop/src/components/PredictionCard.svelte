<!--
  What we think you want, and why.

  The "why" line is not decoration. A suggestion the user cannot account for is
  one they have to check every time, which costs more than choosing manually
  would have. The sentence comes from the backend, where the journal that
  produced the number lives.
-->
<script lang="ts">
  import type { Prediction, ProbeResult } from "../lib/ipc";
  import AlternateList from "./AlternateList.svelte";

  let {
    prediction,
    probes,
    selected,
    expanded = false,
    ontoggle,
    onselect,
  }: {
    prediction: Prediction;
    probes: ProbeResult[];
    selected: number;
    expanded?: boolean;
    ontoggle?: () => void;
    onselect: (index: number) => void;
  } = $props();

  const top = $derived(prediction.suggestions[selected] ?? null);
  const first = $derived(probes[0] ?? null);
  const count = $derived(prediction.paths.length);

  const glyphs: Record<string, string> = {
    image: "▨",
    audio: "♪",
    video: "▶",
    document: "▤",
    archive: "▣",
    tabular: "▦",
    unknown: "◇",
  };

  function dimensions(p: ProbeResult | null): string | null {
    const props = p?.properties;
    if (!props) return null;
    if (props.width && props.height) return `${props.width}×${props.height}`;
    if (props.durationMs) return `${Math.round(props.durationMs / 1000)}s`;
    if (props.pages) return `${props.pages} page${props.pages === 1 ? "" : "s"}`;
    if (props.entries) return `${props.entries} entries`;
    if (props.rows) return `${props.rows} rows`;
    return null;
  }

  const detail = $derived(dimensions(first));
  const anyMismatch = $derived(probes.some((p) => p.mismatched));
  const anyPolyglot = $derived(probes.some((p) => p.polyglot));
  const anyAltered = $derived(probes.some((p) => p.nameAltered));
</script>

<div class="card">
  <button
    type="button"
    class="suggestion"
    aria-expanded={expanded}
    onclick={() => ontoggle?.()}
  >
    <span class="icon" aria-hidden="true">{glyphs[prediction.kind] ?? glyphs.unknown}</span>
    <span class="body">
      <span class="target">
        {prediction.input} → {top ? top.target : "—"}
      </span>
      <span class="meta">
        {count === 1 ? (first?.fileName ?? "") : `${count} files`}
        {#if detail}<span class="sep" aria-hidden="true">·</span>{detail}{/if}
        {#if top}<span class="sep" aria-hidden="true">·</span>class {top.className}{/if}
      </span>
      <span class="why">{prediction.why}</span>
    </span>
    <span class="enter" aria-hidden="true">{top?.armed ? "⏎" : "…"}</span>
  </button>

  {#if anyPolyglot || anyMismatch || anyAltered}
    <ul class="flags">
      {#if anyPolyglot}
        <li class="critical">
          More than one format signature matched. This file is quarantined.
        </li>
      {/if}
      {#if anyMismatch}
        <li class="attention">
          The extension disagrees with the content. Routing by content.
        </li>
      {/if}
      {#if anyAltered}
        <li class="attention">
          A filename contains characters that were escaped for display.
        </li>
      {/if}
    </ul>
  {/if}

  {#if expanded}
    <AlternateList suggestions={prediction.suggestions} {selected} {onselect} />
  {/if}
</div>

<style>
  .card {
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    padding: var(--space-2);
  }

  .suggestion {
    display: flex;
    align-items: center;
    gap: var(--space-5);
    width: 100%;
    padding: var(--space-4) var(--space-5);
    border-radius: var(--radius-md);
    text-align: left;
    transition: background var(--motion-micro);
  }

  .suggestion:hover {
    background: var(--surface-sunken);
  }

  .icon {
    flex: none;
    width: 40px;
    height: 40px;
    display: grid;
    place-items: center;
    font-size: 18px;
    background: var(--surface-sunken);
    border-radius: var(--radius-md);
  }

  .body {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .target {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
    letter-spacing: -0.01em;
    text-transform: uppercase;
  }

  .meta,
  .why {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    overflow-wrap: anywhere;
  }

  .meta {
    color: var(--text-secondary);
  }

  .why {
    /* 07 §10: the "why" line is the product's honesty mechanism — it renders
       at text-secondary, never text-tertiary, which fails 4.5:1 on every
       surface we ship and is reserved for non-essential text. */
    color: var(--text-secondary);
  }

  .sep {
    padding: 0 var(--space-2);
  }

  .enter {
    margin-left: auto;
    font-size: var(--text-title-size);
    color: var(--text-disabled);
    transition: color var(--motion-micro);
  }

  .suggestion:hover .enter,
  .suggestion:focus-visible .enter {
    color: var(--text-secondary);
  }

  .flags {
    list-style: none;
    margin: 0;
    padding: 0 var(--space-5) var(--space-3);
    display: grid;
    gap: var(--space-2);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
  }

  .flags .attention {
    color: var(--accent-attention);
  }

  .flags .critical {
    color: var(--accent-critical);
  }

  @media (prefers-reduced-motion: reduce) {
    .suggestion,
    .enter {
      transition: none;
    }
  }
</style>
