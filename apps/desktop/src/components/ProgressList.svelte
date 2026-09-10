<!--
  How far the batch has got.

  Every row here was placed by a `conversion-progress` event from the backend.
  Nothing is interpolated and nothing is timed: a bar driven by `setInterval`
  can show "3 of 40" while the third file has already failed, which is the one
  thing this screen must never do.

  The bar itself lives at the top of the content area (App.svelte), which is
  where the design system pins it (07 §10). What this component owns is the
  live count — what assistive technology announces — and the per-file states.
-->
<script lang="ts">
  import type { FileProgress } from "../lib/stores/job.svelte";

  let {
    progress,
    total,
    completed,
    cancelling = false,
    oncancel,
  }: {
    progress: FileProgress[];
    total: number;
    completed: number;
    cancelling?: boolean;
    oncancel?: () => void;
  } = $props();

  const marks: Record<FileProgress["phase"], string> = {
    started: "◌",
    done: "✓",
    failed: "✕",
    cancelled: "—",
  };
</script>

<!-- tabindex -1: focus lands on this region when the screen appears. -->
<section class="running" aria-label="Conversion progress" tabindex="-1">
  <header>
    <p class="count" role="status" aria-live="polite">
      {completed} of {total}
      {cancelling ? "· finishing the current file, then stopping" : ""}
    </p>
    <button type="button" class="cancel" onclick={() => oncancel?.()} disabled={cancelling}>
      {cancelling ? "Finishing current file…" : "Cancel"}
    </button>
  </header>

  <ul class="files">
    {#each progress as file (file.index)}
      <li class={file.phase}>
        <span class="mark" aria-hidden="true">{marks[file.phase]}</span>
        <span class="name">{file.fileName}</span>
        <span class="phase">{file.phase}</span>
      </li>
    {/each}
  </ul>
</section>

<style>
  .running {
    display: grid;
    gap: var(--space-4);
  }

  header {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
  }

  .count {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
  }

  .cancel {
    margin-left: auto;
    /* 28 px interactive floor (07 §11). */
    min-height: 28px;
    padding: 0 var(--space-5);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  .cancel:hover:not(:disabled) {
    color: var(--accent-critical);
    border-color: var(--accent-critical);
  }

  .cancel:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--space-1);
    max-height: 220px;
    overflow-y: auto;
  }

  .files li {
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

  .name {
    overflow-wrap: anywhere;
  }

  .phase {
    margin-left: auto;
    color: var(--text-tertiary);
  }

  .started .name {
    color: var(--text-secondary);
  }

  .failed .mark,
  .failed .phase {
    color: var(--accent-critical);
  }

  .cancelled .name,
  .cancelled .phase {
    color: var(--text-tertiary);
  }
</style>
