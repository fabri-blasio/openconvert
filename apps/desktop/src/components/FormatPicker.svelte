<!--
  Pick the output format for one file.

  **Ordered most likely first, and the top one is already chosen.** The order is
  the backend's ranking, not an alphabetical list: the whole point of predicting
  is that the answer is usually at the top, and sorting it away would throw that
  work out.

  Every entry is a target `route()` accepts for this input — the backend filters
  by the machine's engines and the file's own properties before scoring, so a
  format offered here cannot be refused by the plan a moment later.
-->
<script lang="ts">
  import type { Suggestion } from "../lib/ipc";

  let {
    options,
    selected,
    onselect,
    disabled = false,
  }: {
    options: Suggestion[];
    /** Index into `options`. */
    selected: number;
    onselect: (index: number) => void;
    disabled?: boolean;
  } = $props();

  let open = $state(false);
  let root = $state<HTMLDivElement | null>(null);

  const current = $derived(options[selected] ?? null);

  function choose(i: number) {
    open = false;
    onselect(i);
  }

  /** Escape closes; a click elsewhere closes. Both without a global listener
   *  that outlives the component. */
  function onWindowClick(e: MouseEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }

  function onKey(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      open = false;
    }
  }
</script>

<svelte:window onclick={onWindowClick} onkeydown={onKey} />

<div class="picker" bind:this={root}>
  <button
    type="button"
    class="trigger"
    {disabled}
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span class="value">{current ? current.target.toUpperCase() : "—"}</span>
    <span class="caret" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <ul class="menu" role="listbox" aria-label="Output format" tabindex="-1">
      {#each options as opt, i (opt.target)}
        <li>
          <button
            type="button"
            role="option"
            aria-selected={i === selected}
            class="option"
            class:current={i === selected}
            onclick={() => choose(i)}
          >
            <span class="name">{opt.target.toUpperCase()}</span>
            <!-- NO FIDELITY WORD HERE. "B (lossy, standard)" is the receipt's
                 job and History's; a format menu is where someone picks a
                 format, and a classification they have not been taught yet
                 reads as a warning about the option they are hovering. -->
            {#if i === 0}<span class="best">suggested</span>{/if}
          </button>
        </li>
      {/each}
      {#if options.length === 0}
        <li class="empty">Nothing routes from this file on this machine.</li>
      {/if}
    </ul>
  {/if}
</div>

<style>
  .picker {
    position: relative;
    flex: none;
  }

  .trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 30px;
    min-width: 104px;
    padding: 0 var(--space-3) 0 var(--space-4);
    border-radius: var(--radius-sm);
    border: 0.5px solid var(--hairline-strong);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-size: var(--text-small-size);
    font-weight: var(--weight-medium);
  }

  .trigger:hover:not(:disabled) {
    border-color: var(--text-tertiary);
  }

  .trigger:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .value {
    margin-right: auto;
    letter-spacing: 0.02em;
  }

  .caret {
    color: var(--text-tertiary);
    font-size: 10px;
  }

  .menu {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    right: 0;
    min-width: 208px;
    /* Long lists scroll rather than run off the window. */
    max-height: 264px;
    overflow-y: auto;
    scrollbar-width: thin;
    scrollbar-color: var(--hairline-strong) transparent;
    list-style: none;
    margin: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-raised);
    box-shadow: var(--shadow-float);
  }

  .option {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 30px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-sm);
    text-align: left;
    font-size: var(--text-small-size);
    color: var(--text-primary);
  }

  .option:hover {
    background: var(--surface-sunken);
  }

  .option.current {
    background: var(--surface-sunken);
    font-weight: var(--weight-medium);
  }

  .name {
    letter-spacing: 0.02em;
  }

  .best {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: var(--text-caption-size);
  }

  .empty {
    padding: var(--space-3);
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }
</style>
