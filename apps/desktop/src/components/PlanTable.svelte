<!--
  What would happen, step by step, before anything does.

  **The most distinctive screen in the product, and it exists because of the
  architecture rather than in addition to it**: this is `route()` rendered, with
  `execute()` never called. Every column is a field the plan already carries —
  class, isolation, limits — so there is nothing here that could describe a
  conversion the engine would not actually run.

  A blocked plan shows warnings *instead of* steps, because a blocked plan has
  no steps. Refusal is the absence of a plan, not a flag on one.
-->
<script lang="ts">
  import type { ClassName, PlanPreview } from "../lib/ipc";
  import ProfileBadge from "./ProfileBadge.svelte";

  let { plan, showFile = false }: { plan: PlanPreview; showFile?: boolean } = $props();

  const glyphs: Record<ClassName, string> = { A: "=", B: "≈", C: "⌇", D: "✦" };
  const labels: Record<ClassName, string> = {
    A: "lossless",
    B: "lossy",
    C: "rebuilt",
    D: "generative",
  };

  function chipClass(c: ClassName): string {
    return c === "D" ? "generous" : c === "C" ? "inferred" : c === "B" ? "lossy" : "quiet";
  }

  const blocking = $derived(plan.warnings.filter((w) => w.blocking));
  const notes = $derived(plan.warnings.filter((w) => !w.blocking));
</script>

{#if !plan.executable}
  <div class="warnings" role="alert">
    <p class="headline">Nothing would run.</p>
    {#each blocking as w, i (i)}
      <p class="warning blocking">
        {#if showFile}<span class="file">{w.fileName}</span>{/if}{w.message}
      </p>
    {/each}
    {#if blocking.length === 0}
      <p class="warning blocking">No route met the requirements for this drop.</p>
    {/if}
  </div>
{:else}
  <!-- Wide content scrolls inside its own box, never past it.
       
       Seven columns do not fit a 650 px card, and with `overflow: visible` the
       table pushed its container instead: the file list started scrolling
       sideways, and because the rows are only as wide as the list, their
       backgrounds and dividers stopped at the fold. Scrolled right, the content
       sat on bare background — which reads as "the content disappeared". A
       scroller of its own is the fix; nothing else on the page moves. -->
  <div class="scroller">
    <table>
      <caption class="sr-only">
      The steps this conversion would run, with the fidelity class, sandbox
      profile and resource limits of each.
    </caption>
      <thead>
        <tr>
          <th scope="col" class="num">#</th>
          {#if showFile}<th scope="col">File</th>{/if}
          <th scope="col">Step</th>
          <th scope="col">Engine</th>
          <th scope="col">Class</th>
          <th scope="col">Sandbox</th>
          <th scope="col">Limits</th>
        </tr>
      </thead>
      <tbody>
        {#each plan.steps as step, i (i)}
          <tr>
            <td class="num">{i + 1}</td>
            {#if showFile}<td class="file">{step.fileName}</td>{/if}
            <td class="mono step">{step.kind}</td>
            <td class="engine">{step.engineName}</td>
            <td>
              <span class="chip {chipClass(step.className)}">
                <span aria-hidden="true">{glyphs[step.className]}</span>
              {step.className}
                <span class="chip-label">{labels[step.className]}</span>
              </span>
          </td>
            <td><ProfileBadge display={step.isolation} /></td>
            <td class="limits">{step.limitsSummary}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>

  {#if notes.length > 0}
    <div class="warnings">
      {#each notes as w, i (i)}
        <p class="warning">
          {#if showFile}<span class="file">{w.fileName}</span>{/if}{w.message}
        </p>
      {/each}
    </div>
  {/if}
{/if}

<style>
  .scroller {
    overflow-x: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
    /* Contain the scroll: a flick past the end must not scroll the panel
       behind it. */
    overscroll-behavior-x: contain;
  }

  table {
    width: 100%;
    /* Columns keep their content rather than crushing to fit; the scroller
       handles the excess. */
    min-width: max-content;
    border-collapse: collapse;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
  }

  th {
    text-align: left;
    font-weight: var(--weight-regular);
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    padding: var(--space-2) var(--space-4);
    border-bottom: 0.5px solid var(--hairline);
    white-space: nowrap;
  }

  td {
    height: var(--row-h);
    padding: var(--space-2) var(--space-4);
    vertical-align: middle;
  }

  tbody tr + tr td {
    border-top: 0.5px solid var(--hairline);
  }

  .num {
    color: var(--text-tertiary);
    width: 2ch;
  }

  .step {
    overflow-wrap: anywhere;
  }

  .engine,
  .limits {
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .file {
    color: var(--text-secondary);
    padding-right: var(--space-3);
    overflow-wrap: anywhere;
  }

  .chip-label {
    color: var(--text-tertiary);
  }

  .warnings {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-4) 0 0;
  }

  .headline {
    font-size: var(--text-title-size);
    font-weight: var(--weight-medium);
    color: var(--accent-critical);
  }

  .warning {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    border-left: 2px solid var(--accent-attention);
    padding-left: var(--space-4);
    overflow-wrap: anywhere;
  }

  .warning.blocking {
    color: var(--accent-critical);
    border-left-color: var(--accent-critical);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
