<!--
  The other things this file could become.

  Numbered, because the numbers are the shortcut: `1`–`9` pick by position, and
  a list whose order is not shown makes that binding unguessable. Every entry is
  a target `route()` would accept — the backend filtered the route table before
  scoring, so nothing offered here is refused by the plan preview a moment later.
-->
<script lang="ts">
  import type { Suggestion } from "../lib/ipc";

  let {
    suggestions,
    selected,
    onselect,
  }: {
    suggestions: Suggestion[];
    selected: number;
    onselect: (index: number) => void;
  } = $props();

  function percent(score: number): string {
    return `${Math.round(score * 100)}%`;
  }
</script>

{#if suggestions.length > 1}
  <ul class="alternates" aria-label="Other targets">
    {#each suggestions as alt, i (alt.target)}
      <li>
        <button
          type="button"
          class="row"
          class:current={i === selected}
          aria-current={i === selected ? "true" : undefined}
          onclick={() => onselect(i)}
        >
          <span class="n" aria-hidden="true">{i + 1}</span>
          <span class="label">{alt.target}</span>
          <span class="class">{alt.className}</span>
          <span class="score">{percent(alt.score)}</span>
        </button>
      </li>
    {/each}
  </ul>
{:else if suggestions.length === 0}
  <p class="none">No target routes from this format on this machine.</p>
{/if}

<style>
  .alternates {
    list-style: none;
    margin: 0;
    padding: var(--space-3) 0 0;
    display: grid;
    gap: var(--space-1);
  }

  .row {
    display: flex;
    align-items: baseline;
    gap: var(--space-4);
    width: 100%;
    min-height: var(--row-h);
    padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    text-align: left;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    transition: background var(--motion-micro);
  }

  .row:hover {
    background: var(--surface-sunken);
  }

  .current {
    background: var(--surface-sunken);
    font-weight: var(--weight-medium);
  }

  .n {
    width: 2ch;
    color: var(--text-tertiary);
  }

  .label {
    text-transform: uppercase;
    letter-spacing: 0.02em;
  }

  .class {
    color: var(--text-secondary);
  }

  .score {
    margin-left: auto;
    color: var(--text-tertiary);
  }

  .none {
    padding: var(--space-4);
    font-size: var(--text-small-size);
    color: var(--text-secondary);
  }

  @media (prefers-reduced-motion: reduce) {
    .row {
      transition: none;
    }
  }
</style>
