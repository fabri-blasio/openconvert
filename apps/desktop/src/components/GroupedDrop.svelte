<!--
  A mixed drop is several decisions, not one.

  Forty photos and three songs dropped together do not share a target, so they
  do not share a card. The grouping is the backend's — which files belong
  together is a fact about their content, decided by `detect()`, not by their
  extensions or by anything this component could work out.

  Each group carries its own prediction, its own alternates and its own plan
  preview. Pressing Enter runs all of them, in the order shown.
-->
<script lang="ts">
  import type { Group } from "../lib/stores/prediction.svelte";
  import PlanTable from "./PlanTable.svelte";
  import PredictionCard from "./PredictionCard.svelte";

  let {
    groups,
    focused,
    expanded,
    onfocus,
    ontoggle,
    onselect,
  }: {
    groups: Group[];
    focused: number;
    expanded: boolean;
    onfocus: (index: number) => void;
    ontoggle: () => void;
    onselect: (groupIndex: number, suggestionIndex: number) => void;
  } = $props();
</script>

<div class="groups">
  {#each groups as group, i (group.prediction.input)}
    <section
      class="file-group"
      class:focused={i === focused}
      aria-label={`${group.prediction.paths.length} ${group.prediction.input} file(s)`}
    >
      <!--
        Focus is tracked, not created. The card inside is already a button, so
        `focusin` bubbling from it says which group the number keys act on —
        without adding a tab stop to a `<div>`, which is both an axe violation
        and a second thing to Tab past for no gain.
      -->
      <div class="focus-target" onfocusin={() => onfocus(i)}>
        <PredictionCard
          prediction={group.prediction}
          probes={group.probes}
          selected={group.selected}
          expanded={expanded && i === focused}
          ontoggle={() => {
            onfocus(i);
            ontoggle();
          }}
          onselect={(s) => onselect(i, s)}
        />
      </div>

      {#if group.planning}
        <p class="planning" role="status">Working out what would happen…</p>
      {:else if group.plan}
        <div class="plan">
          <PlanTable plan={group.plan} showFile={group.prediction.paths.length > 1} />
        </div>
      {/if}
    </section>
  {/each}
</div>

<style>
  .groups {
    display: grid;
    gap: var(--space-6);
  }

  /* `.file-group` and not `.group`: base.css owns `.group` as a settings
     card (surface, radius, shadow, overflow: hidden). A section that only
     means "these files belong together" was inheriting that chrome and
     drawing an identical card behind PredictionCard's own — measured: same
     background, same shadow, same 14 px radius, nested. */
  .file-group {
    display: grid;
    gap: var(--space-4);
  }

  .focus-target {
    border-radius: var(--radius-lg);
  }

  .plan {
    padding: 0 var(--space-2);
  }

  .planning {
    padding: var(--space-4) var(--space-5);
    font-size: var(--text-small-size);
    color: var(--text-tertiary);
  }

  /*
    A multi-group drop needs to say which group the number keys will act on.
    A hairline, not a highlight: prominence scales with consequence, and
    "this one is selected" is not a consequence.
  */
  .file-group:not(.focused) .focus-target {
    opacity: 0.72;
  }
</style>
