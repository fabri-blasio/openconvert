<!--
  Every file in a finished batch, one row each.

  # Where this sits

  `ProgressList` owns the batch WHILE it runs — its rows are placed by
  `conversion-progress` events and carry no outcome yet, because there isn't
  one. This owns the batch AFTER it finishes, where every row has a result and
  the useful question is "which ones need me?".

  # Why rows, not receipts

  The done screen rendered a full `ReceiptView` per file. That is right for one
  file and unreadable for fifty. So each file gets one line here, and the whole
  receipt — the product's second pillar, and the thing that makes an output
  checkable — is one click away on the row that needs it. Nothing is removed;
  it is folded.

  Failures are expanded by default. A row a person has to open to discover why
  it failed is a row that hides the only information it has.
-->
<script lang="ts">
  import { errorText, revealOutput, type ConversionResult } from "../lib/ipc";
  import ReceiptView from "./ReceiptView.svelte";

  let { results }: { results: ConversionResult[] } = $props();

  /** Which rows are open, by index. Failures start open. */
  /**
   * Which rows have their receipt expanded.
   *
   * **A FAILURE IS NOT OPENED BY DEFAULT ANY MORE.** It used to be, and the
   * result was that a failed file rendered its compact row and then a whole
   * `ReceiptView` underneath — repeating the filename as a heading, restating
   * the same error, and adding "0 ms" and a second chevron. A batch where 43
   * of 44 files were cancelled became 43 stacked cards saying nothing the one
   * line above them had not said.
   *
   * A failure now reads exactly like a success: one line, `✕` and the reason
   * in red where the success has `✓` and its output in grey. The receipt is
   * still one click away, for the file where the reason is not enough.
   */
  let open = $state(new Set<number>());
  $effect(() => {
    void results;
    open = new Set<number>();
  });

  function toggle(i: number) {
    const next = new Set(open);
    if (next.has(i)) {
      next.delete(i);
    } else {
      next.add(i);
    }
    open = next;
  }

  async function showOutput(result: ConversionResult) {
    try {
      await revealOutput(result.outputPath);
    } catch (e) {
      window.alert(errorText(e));
    }
  }

  /**
   * The class glyph for a row.
   *
   * `classApplied` arrives as `"B (lossy, standard)"`; the letter is the part
   * that carries meaning. The word stays beside the glyph — `07` §"the glyphs
   * are a dependency" requires the label to survive a font that lacks `⌇` or
   * `✦`, which most do.
   */
  const GLYPHS: Record<string, { mark: string; word: string }> = {
    A: { mark: "=", word: "lossless" },
    B: { mark: "≈", word: "lossy" },
    C: { mark: "⌇", word: "AI-read" },
    D: { mark: "✦", word: "AI-generated" },
  };

  function classOf(applied: string): { mark: string; word: string } | null {
    return GLYPHS[applied.trim().charAt(0).toUpperCase()] ?? null;
  }

  /** Just the file name of an output path, for the row. */
  function baseName(path: string): string {
    const cut = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
    return cut >= 0 ? path.slice(cut + 1) : path;
  }

  function secs(ms: number): string {
    return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)}s`;
  }
</script>

<ul class="files" aria-label="Files in this batch">
  {#each results as result, i (i)}
    {@const klass = classOf(result.classApplied)}
    <li class:failed={!result.success}>
      <div class="row">
        <span class="mark" aria-hidden="true">{result.success ? "✓" : "✕"}</span>

        <span class="name">{result.fileName}</span>

        {#if result.success}
          <span class="arrow" aria-hidden="true">→</span>
          <span class="output">{baseName(result.outputPath)}</span>
          {#if klass}
            <span class="class">
              <span aria-hidden="true">{klass.mark}</span>
              {klass.word}
            </span>
          {/if}
          <span class="duration">{secs(result.durationMs)}</span>
        {:else}
          <!-- The same slot the output name occupies on a success, carrying the
               reason instead. One shape for both outcomes. -->
          <span class="why">{result.errorMessage ?? "did not convert"}</span>
        {/if}

        {#if result.success}
          <button
            type="button"
            class="folder"
            onclick={() => showOutput(result)}
            aria-label={`Show ${baseName(result.outputPath)} in folder`}
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
        {/if}
        <button
          type="button"
          class="disclose"
          aria-expanded={open.has(i)}
          aria-label={open.has(i) ? `Hide the receipt for ${result.fileName}` : `Show the receipt for ${result.fileName}`}
          onclick={() => toggle(i)}
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
      </div>

      {#if open.has(i)}
        <div class="detail">
          <ReceiptView {result} expandedByDefault={true} />
        </div>
      {/if}
    </li>
  {/each}
</ul>

<style>
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-1);
  }

  .row {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    min-height: var(--row-h);
    padding: 0 var(--space-4);
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    border-radius: var(--radius-md);
  }

  .mark {
    width: 1.5ch;
    color: var(--text-tertiary);
  }

  .name,
  .output {
    overflow-wrap: anywhere;
  }

  .output {
    color: var(--text-secondary);
  }

  .arrow,
  .class,
  .duration {
    color: var(--text-tertiary);
  }

  .class {
    white-space: nowrap;
  }

  .duration {
    margin-left: auto;
  }

  .why {
    color: var(--accent-critical);
    overflow-wrap: anywhere;
  }

  .failed .mark {
    color: var(--accent-critical);
  }

  .folder,
  .disclose {
    margin-left: var(--space-4);
    /* 28 px interactive floor (07 §11). */
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline);
    font-size: var(--text-small-size);
    color: var(--text-tertiary);
    white-space: nowrap;
  }

  .folder {
    width: 28px;
    padding: 0;
  }

  .folder,
  .disclose {
    display: inline-grid;
    place-items: center;
  }

  .folder svg,
  .disclose svg {
    width: 15px;
    height: 15px;
    display: block;
  }

  /* The reason takes the room the output name has on a success, and truncates
     rather than wrapping the row onto a second line. */
  .why {
    margin-left: auto;
    text-align: right;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .disclose {
    width: 28px;
    padding: 0;
  }

  .disclose svg {
    transition: transform 120ms ease;
  }

  .disclose[aria-expanded="true"] svg {
    transform: rotate(180deg);
  }

  .disclose:hover {
    color: var(--text-primary);
  }

  /* The failure row already spends its width on the reason. */
  .failed .duration {
    margin-left: 0;
  }

  .detail {
    padding: var(--space-2) 0 var(--space-4);
  }
</style>
