<!--
  The whole window is the drop target, and the whole content area is the button.

  Tauri v2 intercepts OS file drops itself (`dragDropEnabled`) and the webview's
  own `DragEvent.dataTransfer.files` never carries a real path — a browser
  `File` has no `path` property, and inventing one from `file.name` would send
  the backend a bare filename it would then resolve against the process's
  working directory. So the paths come from Tauri's `onDragDropEvent`, which is
  the only place they exist.

  **The zone is a `<button>`, not a region with a button inside it.** Clicking
  anywhere opens the host file dialog, which is what a large empty target
  invites. Making the element itself the control means Enter and Space work for
  free, it is one tab stop rather than two, and no separate "Browse" affordance
  has to compete with the drop instruction for attention.

  **The listener is not here any more.** This component owned the only
  `onDragDropEvent` subscription in the app, and it is only mounted on the drop
  screen — so once a file was loaded, dropping another one on the window did
  precisely nothing: no files added, no message, no sign the drop had been
  seen. The shell owns the subscription now and passes `dragging` down, so a
  drop is accepted on every screen and this component draws the state.
-->
<script lang="ts">
  let {
    onBrowse,
    disabled = false,
    dragging = false,
  }: {
    /** Opens the host-side file dialog; keyboard-only users convert through
     *  this (07 §11: every action reachable). Drag-drop stays. */
    onBrowse?: () => void;
    disabled?: boolean;
    /** True while a drag is over the window. Owned by the shell. */
    dragging?: boolean;
  } = $props();

  let zone = $state<HTMLButtonElement | null>(null);

  /** The drop screen's primary element: focus lands here on transitions. */
  export function focusPrimary(): void {
    zone?.focus();
  }
</script>

<button
  type="button"
  class="zone"
  class:dragging={dragging && !disabled}
  bind:this={zone}
  onclick={() => onBrowse?.()}
  {disabled}
  aria-label="Drop files to convert, or click to choose files"
>
  <span class="glyph" aria-hidden="true">⇪</span>
  <span class="title">{dragging ? "Release to add files" : "Drop files to convert"}</span>
  <span class="choose">
    or click to choose <span class="key" aria-hidden="true">Ctrl+O</span>
  </span>
</button>

<style>
  .zone {
    /* Fills the whole content area: a target the size of the window is easier
       to hit, and easier to aim a dragged file at, than a box inside one. */
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    width: 100%;
    height: 100%;
    min-height: 0;
    padding: var(--space-8) var(--space-6);
    border: 1.5px dashed var(--dropzone-border);
    border-radius: var(--radius-xl);
    text-align: center;
    cursor: pointer;
    transition:
      border-color var(--motion-standard),
      background var(--motion-standard);
  }

  .zone:hover:not(:disabled) {
    border-color: var(--text-secondary);
    background: var(--surface-raised);
  }

  .zone:disabled {
    cursor: default;
    opacity: 0.6;
  }

  .dragging {
    border-color: var(--text-secondary);
    background: var(--material-thin);
    backdrop-filter: var(--blur-thin);
  }

  .glyph {
    font-size: 40px;
    line-height: 1;
    color: var(--text-secondary);
    transition: color var(--motion-micro);
  }

  .zone:hover:not(:disabled) .glyph,
  .dragging .glyph {
    color: var(--text-primary);
  }

  .title {
    font-size: var(--text-display-size);
    line-height: var(--text-display-lh);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.02em;
  }

  .choose {
    display: inline-flex;
    align-items: center;
    gap: var(--space-3);
    font-size: var(--text-body-size);
    line-height: var(--text-body-lh);
    color: var(--text-secondary);
  }

  .key {
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
  }

  @media (prefers-reduced-motion: reduce) {
    .zone,
    .glyph {
      transition: none;
    }
  }

  @media (prefers-reduced-transparency: reduce) {
    .dragging {
      background: var(--surface-sunken);
      backdrop-filter: none;
    }
  }
</style>
