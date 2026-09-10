<!--
  What a drag looks like on a screen that already has something on it.

  The drop screen is its own target and says so at full size (`DropZone`). Every
  other screen has content the user is in the middle of, so the invitation goes
  OVER it rather than replacing it: the list or the picture stays visible and
  dimmed underneath, which is what makes "this will be added to what I have"
  believable before it happens.

  One component for all three surfaces — the conversion list, the tool stage,
  the merge board — because a drop overlay that looks different depending on
  which screen you are on is three things to recognise instead of one. What
  changes per surface is the sentence, which the caller supplies.

  `pointer-events: none`: this is a picture of a state, not a control. Tauri
  delivers the drop through the OS channel and nothing here can receive it, so
  the overlay must not sit between the pointer and whatever is beneath it.
-->
<script lang="ts">
  let {
    label = "Release to add",
    sublabel = "",
  }: {
    /** The main line. Names what will happen to what is already open. */
    label?: string;
    /** Optional second line, for a surface where "added" needs qualifying. */
    sublabel?: string;
  } = $props();
</script>

<div class="overlay" role="status" aria-live="polite">
  <div class="card">
    <!-- The same glyph the drop screen uses. A drag that means the same thing
         should not be drawn two different ways. -->
    <span class="glyph" aria-hidden="true">⇪</span>
    <span class="label">{label}</span>
    {#if sublabel}<span class="sub">{sublabel}</span>{/if}
  </div>
</div>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    z-index: 30;
    display: grid;
    place-items: center;
    padding: var(--space-5);
    border-radius: inherit;
    /* Translucent, not opaque: the point is that what is underneath survives
       the drop, and the only way to say that is to keep showing it. `thin`
       rather than `thick` for exactly that reason — at 0.88 alpha the list
       vanished, and an overlay that hides the thing it is promising to add to
       is making the promise harder to believe, not easier. */
    background: var(--material-thin);
    backdrop-filter: var(--blur-thin);
    pointer-events: none;
  }

  .card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-8);
    border: 1.5px dashed var(--dropzone-border);
    border-radius: var(--radius-xl);
    text-align: center;
  }

  .glyph {
    font-size: 26px;
    line-height: 1;
    color: var(--text-secondary);
  }

  .label {
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
    color: var(--text-primary);
  }

  .sub {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
  }

  @media (prefers-reduced-transparency: reduce) {
    .overlay {
      background: var(--surface-raised);
      backdrop-filter: none;
    }
  }
</style>
