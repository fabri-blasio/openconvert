<!--
  What a finished batch came to, in four lines.

  # Why this replaced a stack of receipts

  The done screen rendered one full `ReceiptView` per file. For one file that
  is exactly right — the receipt IS the product's claim about what happened.
  For fifty it is fifty cards deep, and the question a person actually has
  ("did they all work?") is answered by scrolling. So: totals here, per-file
  detail one row each in `BatchFileList`, and the full receipt still one click
  away for any file that wants explaining.

  # What is NOT here, and why

  - **Undo All.** Undo already exists as the toast at the bottom of the window,
    and it is *time-limited* — it expires, then offers History instead. A
    second undo button with a different lifetime sitting beside it would be two
    controls making different promises about the same files.
  - **Open Folder.** There is no filesystem or shell capability on this side of
    the boundary: `plugin-opener` is one of the plugins refused by the gate in
    `xtask/src/desktop.rs`. A button that cannot work is worse than no button.
-->
<script lang="ts">
  import { exportBatchLog, errorText, type ConversionResult } from "../lib/ipc";

  let {
    results,
    durationMs,
  }: {
    results: ConversionResult[];
    /** Wall time for the whole batch, measured by the caller. */
    durationMs: number;
  } = $props();

  const converted = $derived(results.filter((r) => r.success).length);
  const failed = $derived(results.filter((r) => !r.success).length);
  const bytesIn = $derived(
    results.filter((r) => r.success).reduce((n, r) => n + r.inputBytes, 0),
  );
  const bytesOut = $derived(
    results.filter((r) => r.success).reduce((n, r) => n + r.outputBytes, 0),
  );

  let exporting = $state(false);
  let exportedTo = $state<string | null>(null);
  let exportError = $state<string | null>(null);

  /** Bytes, in the units a person reads. */
  function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
    return `${(n / 1024 / 1024 / 1024).toFixed(1)} GB`;
  }

  function secs(ms: number): string {
    const s = ms / 1000;
    if (s < 10) return `${s.toFixed(1)}s`;
    if (s < 600) return `${Math.round(s)}s`;
    return `${Math.floor(s / 60)}m${String(Math.round(s % 60)).padStart(2, "0")}s`;
  }

  async function exportLog() {
    if (exporting) return;
    exporting = true;
    exportError = null;
    try {
      const written = await exportBatchLog(
        results.map((r) => ({
          fileName: r.fileName,
          ok: r.success,
          output: r.outputPath,
          error: r.errorMessage ?? "",
          durationMs: r.durationMs,
          inputBytes: r.inputBytes,
          outputBytes: r.outputBytes,
          classApplied: r.classApplied,
        })),
      );
      // `null` means the dialog was dismissed, which is not an error and must
      // not look like one.
      exportedTo = written;
    } catch (e) {
      exportError = errorText(e);
    } finally {
      exporting = false;
    }
  }
</script>

<section class="summary" aria-label="Batch summary">
  <p class="counts" role="status">
    <strong>{converted} converted</strong>
    {#if failed > 0}
      <span class="sep" aria-hidden="true">·</span>
      <span class="failed">{failed} failed</span>
    {/if}
    <span class="sep" aria-hidden="true">·</span>
    <span class="total">{results.length} total</span>
  </p>

  <p class="totals">
    {secs(durationMs)}
    <span class="sep" aria-hidden="true">·</span>
    {bytes(bytesIn)} → {bytes(bytesOut)}
  </p>

  <div class="actions">
    <button type="button" onclick={exportLog} disabled={exporting}>
      {exporting ? "Saving…" : "Export log"}
    </button>
    {#if exportedTo}
      <span class="note" role="status">Saved.</span>
    {/if}
    {#if exportError}
      <span class="note error" role="alert">{exportError}</span>
    {/if}
  </div>
</section>

<style>
  .summary {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-5);
    border: 0.5px solid var(--hairline);
    border-radius: var(--radius-lg);
  }

  .counts {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
  }

  .totals {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  .sep {
    color: var(--text-tertiary);
    padding: 0 0.25ch;
  }

  .failed {
    color: var(--accent-critical);
  }

  .total {
    color: var(--text-secondary);
    font-weight: var(--weight-regular);
  }

  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    margin-top: var(--space-2);
  }

  .actions button {
    /* 28 px interactive floor (07 §11). */
    min-height: 28px;
    padding: 0 var(--space-5);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .actions button:hover:not(:disabled) {
    color: var(--text-primary);
  }

  .actions button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .note {
    font-size: var(--text-small-size);
    color: var(--text-tertiary);
  }

  .note.error {
    color: var(--accent-critical);
  }
</style>
