<!--
  What actually happened to one file.

  **The product's second pillar.** Every output has a receipt, and this is
  the part of it a person reads: where the file went, what it cost in
  fidelity, which engines ran each step and under what confinement, what was
  taken out of it, and the identity of the bytes that went in.

  Everything shown arrives from the receipt struct itself (`ReceiptDetail` on
  the wire) — nothing is re-derived or prettified here, because a receipt the
  UI edits is a receipt nobody checks. When `write_receipts` is off nothing
  reaches this screen at all: no output row claims a provenance it does not
  have.

  The receipt path is shown, not linked. There is no `opener` plugin and there
  will not be one: a link that launches a file handler from the webview is the
  shell-injection surface the whole capability set exists to close.
-->
<script lang="ts">
  import { errorText, revealOutput, type ConversionResult } from "../lib/ipc";
  import ProfileBadge from "./ProfileBadge.svelte";

  let {
    result,
    expandedByDefault = false,
  }: { result: ConversionResult; expandedByDefault?: boolean } = $props();
  let open = $state(false);
  $effect(() => {
    if (expandedByDefault) open = true;
  });
  let revealError = $state<string | null>(null);

  async function showOutput() {
    try {
      await revealOutput(result.outputPath);
      revealError = null;
    } catch (e) {
      revealError = errorText(e);
    }
  }

  function bytes(n: number): string {
    if (n < 1024) return `${n} B`;
    const units = ["KB", "MB", "GB"];
    let value = n / 1024;
    let i = 0;
    while (value >= 1024 && i < units.length - 1) {
      value /= 1024;
      i += 1;
    }
    return `${value.toFixed(value < 10 ? 1 : 0)} ${units[i]}`;
  }

  const delta = $derived(
    result.inputBytes > 0
      ? Math.round(((result.outputBytes - result.inputBytes) / result.inputBytes) * 100)
      : 0,
  );
</script>

<article class="receipt" class:failed={!result.success}>
  <header>
    <span class="mark" aria-hidden="true">{result.success ? "✓" : "✕"}</span>
    <!-- h2: the app's h1 is the product name; per-file headings sit under it. -->
    <h2>{result.fileName}</h2>
    <span class="duration">{result.durationMs} ms</span>
    {#if result.success}
      <!-- A DRAWN FOLDER, NOT `▱`. This button's content was the literal
           character U+25B1 WHITE PARALLELOGRAM, rendered in the UI font — which
           is why it looked like a rectangle: it IS a rectangle. Same for the
           chevron below, which was `⌄`. -->
      <button
        class="folder"
        type="button"
        onclick={showOutput}
        aria-label="Show output in folder"
        title="Show in folder"
      >
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path
            d="M1.75 12.5v-9h4.1l1.4 1.75h7v7.25a.75.75 0 0 1-.75.75H2.5a.75.75 0 0 1-.75-.75Z"
            fill="none"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linejoin="round"
          />
          <path d="M1.75 6.75h12.5" fill="none" stroke="currentColor" stroke-width="1.2" />
        </svg>
      </button>
      <button
        class="disclose"
        type="button"
        onclick={() => (open = !open)}
        aria-expanded={open}
        aria-label={open ? "Hide details" : "Show details"}
      >
        <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
          <path
            d="M4 6.25 8 10.25l4-4"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
            stroke-linejoin="round"
          />
        </svg>
      </button>
    {/if}
  </header>

  {#if result.success}
    <p class="output-line mono">{result.outputPath}</p>
    {#if revealError}<p class="error" role="alert">{revealError}</p>{/if}
    {#if open && result.receiptDetail}
      <p class="provenance">
        <span class="mono">{result.receiptDetail.detected}</span>
        {#if result.receiptDetail.declaredMismatch}
          <span class="mismatch" title={result.receiptDetail.declaredMismatch}>
            filename claimed differently — routed by content
          </span>
        {/if}
        <span class="tool">{result.receiptDetail.tool}</span>
      </p>
    {/if}

    {#if open}<dl>
      <dt>Output</dt>
      <dd class="mono path">{result.outputPath}</dd>

      <dt>Size</dt>
      <dd>
        {bytes(result.inputBytes)} → {bytes(result.outputBytes)}
        <span class="delta">({delta > 0 ? "+" : ""}{delta}%)</span>
      </dd>

      <dt>Class</dt>
      <dd>{result.classApplied}</dd>

      {#if result.contentId}
        <dt>Input ID</dt>
        <dd class="mono path" title="Blake3 of the bytes actually converted">
          {result.contentId.slice(0, 16)}…{result.contentId.slice(-8)}
        </dd>
      {/if}

      <dt>Removed</dt>
      <dd>
        {#if result.removedMetadata.length > 0}
          <ul class="removed">
            {#each result.removedMetadata as item, i (i)}
              <li>{item}</li>
            {/each}
          </ul>
        {:else}
          Nothing
        {/if}
      </dd>

      {#if result.receiptDetail && result.receiptDetail.steps.length > 0}
        <dt>Steps</dt>
        <dd>
          <ul class="steps">
            {#each result.receiptDetail.steps as step, i (i)}
              <li>
                <span class="step-kind mono">{step.kind}</span>
                <span class="step-engine">{step.engine} · class {step.class} · {step.limitsSummary}</span>
                <ProfileBadge display={step.isolation} />
              </li>
            {/each}
          </ul>
        </dd>
      {/if}

      {#if result.receiptPath}
        <dt>Receipt</dt>
        <dd class="mono path">{result.receiptPath}</dd>
      {/if}
    </dl>{/if}
  {:else}
    <p class="error" role="alert">{result.errorMessage ?? "The conversion did not complete."}</p>
    <p class="reassure">Nothing was written.</p>
  {/if}
</article>

<style>
  /* The path belongs to the filename above it, so the gap between them is a
     tight one. A uniform `--space-4` between every child put a full blank line
     between the name and the file it produced — two halves of one statement,
     spaced as though they were unrelated. The blocks that ARE separate ideas
     (the provenance line, the detail list) take their own margin below. */
  .receipt {
    background: var(--surface-raised);
    border-radius: var(--radius-lg);
    box-shadow: var(--shadow-card);
    padding: var(--space-5) var(--space-6);
    display: grid;
    gap: var(--space-2);
  }

  header {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
  }

  h2 {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
    overflow-wrap: anywhere;
  }

  .mark {
    color: var(--text-tertiary);
  }

  .failed .mark {
    color: var(--accent-critical);
  }

  .duration {
    margin-left: auto;
    font-size: var(--text-small-size);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .folder,
  .disclose {
    display: inline-grid;
    place-items: center;
    width: 28px;
    min-height: 28px;
    border: 0;
    border-radius: var(--radius-full);
    color: var(--text-secondary);
  }

  .folder svg,
  .disclose svg {
    width: 16px;
    height: 16px;
    display: block;
  }

  .folder:hover,
  .disclose:hover {
    background: var(--fill-quaternary);
    color: var(--text-primary);
  }

  .disclose {
    transition: transform 120ms ease;
  }

  .disclose[aria-expanded="true"] {
    transform: rotate(180deg);
  }

  .output-line {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
    overflow-wrap: anywhere;
  }

  .provenance {
    margin-top: var(--space-3);
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    flex-wrap: wrap;
    font-size: var(--text-caption-size);
    line-height: var(--text-caption-lh);
    color: var(--text-secondary);
  }

  .mismatch {
    color: var(--accent-attention);
  }

  .tool {
    margin-left: auto;
  }

  dl {
    display: grid;
    grid-template-columns: 84px 1fr;
    gap: var(--space-3) var(--space-5);
    /* Its own separation from the summary above, now that the container gap
       is tight enough to bind the filename to its output path. */
    margin: var(--space-3) 0 0;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
  }

  dt {
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
    line-height: var(--text-small-lh);
  }

  dd {
    margin: 0;
    overflow-wrap: anywhere;
  }

  .path {
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
  }

  .delta {
    color: var(--text-secondary);
  }

  .removed {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-1);
  }

  .removed li::before {
    content: "− ";
    color: var(--text-tertiary);
  }

  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-2);
  }

  .steps li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .step-kind {
    font-size: var(--text-caption-size);
  }

  .step-engine {
    color: var(--text-secondary);
    font-size: var(--text-caption-size);
  }

  .error {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--accent-critical);
    overflow-wrap: anywhere;
  }

  .reassure {
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }
</style>
