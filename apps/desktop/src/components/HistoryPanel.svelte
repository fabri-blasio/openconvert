<!--
  Past conversions, from the batch journals.

  This is where undo goes to live: the 60-second toast is the prominent
  window, and when it lapses the record stays here (02 §12.2). Everything
  shown was already on disk — this view adds nothing and invents nothing;
  even the timestamp is the batch file's own mtime, because the journal
  format carries no per-line clock.

  THREE COLUMNS, AND NO PROSE.

  It used to carry an explanatory paragraph and put the failure reason in the
  row, so a list of twelve conversions was a wall of sentences. A history is
  scanned, not read: date, what was converted, size. The reason for a failure
  is still available — it is the row's tooltip — but it no longer sets the
  width of the column or the height of the row.

  AND THE ROW IS THE RECEIPT. Every completed conversion wrote a
  `<output>.receipt.json` beside its output; the row now knows where, and
  activating it reveals that file in the file manager. That is the whole
  interaction: no detail pane, no second copy of the receipt rendered here.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import {
    listHistory,
    wipeHistory,
    revealReceipt,
    revealOutput,
    errorText,
    type HistoryEntry,
  } from "../lib/ipc";

  import OutputReview from "./OutputReview.svelte";
  let { onclose, onrepeat }: { onclose: () => void; onrepeat?: (request: NonNullable<HistoryEntry["request"]>) => void } = $props();
  let search = $state(""), toolFilter = $state("");
  let reviewing = $state<string | null>(null);


  let entries = $state<HistoryEntry[]>([]);
  const visible = $derived(entries.filter(e => `${e.sourceName} ${e.outputName}`.toLowerCase().includes(search.toLowerCase()) && (!toolFilter || e.tool === toolFilter)));
  let error = $state<string | null>(null);
  let wiping = $state(false);
  let heading = $state<HTMLHeadingElement | null>(null);

  async function refresh() {
    try {
      entries = await listHistory(200);
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  async function wipe() {
    wiping = true;
    try {
      await wipeHistory();
      await refresh();
    } catch (e) {
      error = errorText(e);
    } finally {
      wiping = false;
    }
  }

  function when(secs: number): string {
    if (secs <= 0) return "—";
    // The user's own locale and timezone. `toISOString` reported UTC, which
    // for anyone not on it labelled this morning's conversion with the wrong
    // hour and occasionally the wrong day.
    const d = new Date(secs * 1000);
    return d.toLocaleDateString(undefined, { day: "2-digit", month: "short" })
      + " " + d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  }

  /** Bytes, at the precision a person reads. */
  function size(bytes: number): string {
    if (bytes <= 0) return "—";
    if (bytes < 1024) return `${bytes} B`;
    const units = ["KB", "MB", "GB"];
    let n = bytes / 1024;
    let u = 0;
    while (n >= 1024 && u < units.length - 1) {
      n /= 1024;
      u += 1;
    }
    return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${units[u]}`;
  }

  async function reveal(entry: HistoryEntry) {
    if (entry.outputPath) { reviewing = entry.outputPath; return; }
    if (!entry.receiptPath) return;
    try {
      await revealReceipt(entry.receiptPath);
      error = null;
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(() => {
    void refresh();
    heading?.focus();
  });
</script>

<div class="panel">
  <header class="panel-head">
    <!-- tabindex -1: focus moves here when the panel opens. -->
    <h2 bind:this={heading} tabindex="-1">History</h2>
    <button type="button" class="ghost" onclick={onclose}>Close</button>
  </header>

  <div class="filters"><input aria-label="Search history" placeholder="Search files" bind:value={search} /><select aria-label="Filter history by tool" bind:value={toolFilter}><option value="">All tools</option>{#each [...new Set(entries.map(e=>e.tool).filter(Boolean))] as tool}<option value={tool}>{tool}</option>{/each}</select></div>
  {#if reviewing}<OutputReview path={reviewing} onclose={() => reviewing = null} />{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}

  {#if entries.length === 0 && !error}
    <p class="empty">No conversions recorded yet.</p>
  {:else}
    <ul>
      {#each visible as entry, i (i)}
        <li class:failed={entry.outcome === "failed"}>
          {#if entry.outputPath || entry.receiptPath}
            <!-- A row with a receipt is a button, so it reaches the keyboard
                 as one. A row without is not, because there is nothing to
                 open and a control that does nothing is worse than none. -->
            <button
              type="button"
              class="row"
              onclick={() => reveal(entry)}
              title="Review output"
            >
              <span class="when">{when(entry.whenSecs)}</span>
              <span class="names">{entry.sourceName} → {entry.outputName}</span>
              <span class="size">{size(entry.outputBytes)}</span>
            </button>
          {:else}
            <div class="row" title={entry.reason ?? entry.outcome}>
              <span class="when">{when(entry.whenSecs)}</span>
              <span class="names">{entry.sourceName || "—"} → {entry.outputName || "—"}</span>
              <span class="size">{entry.outcome === "failed" ? "failed" : size(entry.outputBytes)}</span>
            </div>
          {/if}
          <div class="entry-actions">
            {#if entry.outputPath}<button type="button" onclick={() => void revealOutput(entry.outputPath!).catch(e=>error=errorText(e))}>Show in folder</button>{/if}
            {#if entry.receiptPath}<button type="button" onclick={() => void revealReceipt(entry.receiptPath!).catch(e=>error=errorText(e))}>Receipt</button>{/if}
            {#if entry.request && onrepeat}<button type="button" onclick={() => onrepeat?.(entry.request!)}>Run again</button>{/if}
          </div>
        </li>
      {/each}
    </ul>
    <footer>
      <span>{entries.length} entr{entries.length === 1 ? "y" : "ies"}</span>
      <button type="button" class="ghost danger-text" disabled={wiping || entries.length === 0} onclick={wipe}>
        Wipe history
      </button>
    </footer>
  {/if}
</div>

<style>
  .filters,.entry-actions { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
  .filters input { flex: 1; min-width: 0; }
  .entry-actions { padding: 0 12px 8px; justify-content: flex-end; }
  .filters input,.filters select,.entry-actions button { color: var(--text-primary); background: var(--surface-sunken); border: 1px solid var(--hairline-strong); border-radius: var(--radius-sm); padding: 6px 10px; }
  .panel {
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    border-radius: var(--radius-xl);
    box-shadow: var(--shadow-float);
    padding: var(--space-6) var(--space-7);
    display: grid;
    gap: var(--space-5);
    overflow-y: auto;
  }

  .panel-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
  }

  h2 {
    font-size: var(--text-heading-size);
    line-height: var(--text-heading-lh);
    font-weight: var(--weight-semibold);
  }

  h2:focus-visible {
    outline-offset: 4px;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0;
    border-block: 0.5px solid var(--hairline);
  }

  li + li {
    border-top: 0.5px solid var(--hairline);
  }

  /* Three columns, and the middle one takes the slack: a date and a size are
     fixed-width facts, a pair of filenames is not. `minmax(0, 1fr)` rather
     than `1fr` so a long name truncates instead of pushing the size off. */
  .row {
    display: grid;
    grid-template-columns: 112px minmax(0, 1fr) 72px;
    align-items: center;
    gap: var(--space-4);
    width: 100%;
    min-height: 36px;
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-md);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    text-align: left;
    color: inherit;
  }

  button.row:hover {
    background: var(--fill-quaternary);
  }

  button.row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }

  .when {
    color: var(--text-secondary);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  .names {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .size {
    color: var(--text-secondary);
    text-align: right;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }

  li.failed .size {
    color: var(--accent-critical);
  }

  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-top: 0.5px solid var(--hairline);
    padding-top: var(--space-3);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .empty {
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .error {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--accent-critical);
    overflow-wrap: anywhere;
  }

  .ghost {
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-primary);
  }

  .danger-text {
    color: var(--accent-critical);
  }

  @media (prefers-reduced-transparency: reduce) {
    .panel {
      background: var(--surface-raised);
      backdrop-filter: none;
    }
  }
</style>
