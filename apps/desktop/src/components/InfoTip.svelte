<!--
  The "why is this here" affordance for a settings section.

  A disclosure, not a hover tooltip: hover text is unreachable by keyboard, gone
  on touch, and cannot be read at leisure. This is a button that toggles a
  paragraph, so the explanation is available to everyone and stays put while it
  is read.

  It carries no copy of its own — the text is passed in, which keeps the
  sentence next to the controls it describes rather than in a strings file that
  drifts away from them.
-->
<script lang="ts">
  let { label, open = $bindable(false) }: { label: string; open?: boolean } = $props();
</script>

<button
  type="button"
  class="info"
  aria-expanded={open}
  aria-label={open ? `Hide help for ${label}` : `What is ${label}?`}
  onclick={() => (open = !open)}
>
  <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false">
    <circle cx="8" cy="8" r="6.4" fill="none" stroke="currentColor" stroke-width="1.2" />
    <circle cx="8" cy="5.1" r="0.85" fill="currentColor" />
    <path
      d="M8 7.4v3.9"
      fill="none"
      stroke="currentColor"
      stroke-width="1.2"
      stroke-linecap="round"
    />
  </svg>
</button>

<style>
  .info {
    /* 28 px interactive floor (07 §11), met by the control rather than by
       padding on a smaller hit area. */
    width: 28px;
    height: 28px;
    display: inline-grid;
    place-items: center;
    border-radius: var(--radius-full);
    color: var(--text-tertiary);
    transition:
      color var(--motion-micro),
      background var(--motion-micro);
  }

  .info:hover,
  .info[aria-expanded="true"] {
    color: var(--text-primary);
    background: var(--surface-sunken);
  }

  svg {
    width: 15px;
    height: 15px;
  }

  @media (prefers-reduced-motion: reduce) {
    .info {
      transition: none;
    }
  }
</style>
