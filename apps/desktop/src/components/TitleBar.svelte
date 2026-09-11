<!--
  The window's own title bar, drawn by the app.

  The native frame is off (`decorations: false`), so this bar is what the user
  drags, and the three controls on the right are what the OS used to provide.
  That is the whole point: a white system strip above a dark application reads
  as two programs stacked, and the seam is the first thing anyone notices.

  **The controls are Windows-shaped on purpose** — minimise, maximise, close, in
  that order, on the right, with the close button turning red on hover. A custom
  frame that puts them somewhere else makes the app harder to use, not more
  distinctive.

  Everything here needs an explicit permission (`core:window:allow-minimize` and
  friends); `core:default` grants only the read-only half of the window API. The
  four grants are listed in `capabilities/main.json` and on the reviewed
  allowlist in `xtask/src/desktop.rs`.
-->
<script lang="ts">
  import Logo from "./Logo.svelte";
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";

  let { children }: { children?: import("svelte").Snippet } = $props();

  let maximized = $state(false);
  const win = getCurrentWindow();

  async function refresh() {
    try {
      maximized = await win.isMaximized();
      document.documentElement.dataset.windowMaximized = String(maximized);
    } catch {
      // A window that cannot answer is not a reason to fail the chrome.
    }
  }

  onMount(() => {
    void refresh();
    let unlisten: (() => void) | undefined;
    void win.onResized(() => void refresh()).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
      delete document.documentElement.dataset.windowMaximized;
    };
  });

  /** Drag the window, unless the press landed on a control. */
  function startDrag(e: MouseEvent) {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, a, input, select, [role='menu']")) return;
    void win.startDragging();
  }
</script>

<header class="bar">
  <!-- The drag gesture belongs to a surface, not to the header.
       
       Putting mousedown on the <header> makes an interactive element out of a
       landmark, and `role="toolbar"` would then demand a tab stop for what is
       only a place to grab. This layer sits behind the controls, is hidden from
       assistive technology, and carries the gesture on its own. -->
  <div
    class="drag-surface"
    aria-hidden="true"
    onmousedown={startDrag}
    ondblclick={() => win.toggleMaximize()}
  ></div>

  <span class="brand"><Logo size={18} />OpenConvert</span>

  {@render children?.()}

  <div class="controls">
    <button
      type="button"
      class="ctl"
      aria-label="Minimise"
      title="Minimise"
      onclick={() => win.minimize()}
    >
      <svg viewBox="0 0 10 10" aria-hidden="true"
        ><path d="M0 5h10" stroke="currentColor" stroke-width="1" /></svg
      >
    </button>

    <button
      type="button"
      class="ctl"
      aria-label={maximized ? "Restore" : "Maximise"}
      title={maximized ? "Restore" : "Maximise"}
      onclick={() => win.toggleMaximize()}
    >
      {#if maximized}
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M2.5 2.5h5v5h-5z" fill="none" stroke="currentColor" stroke-width="1" />
          <path d="M0.5 7.5v-7h7" fill="none" stroke="currentColor" stroke-width="1" />
        </svg>
      {:else}
        <svg viewBox="0 0 10 10" aria-hidden="true"
          ><path d="M0.5 0.5h9v9h-9z" fill="none" stroke="currentColor" stroke-width="1" /></svg
        >
      {/if}
    </button>

    <button
      type="button"
      class="ctl close"
      aria-label="Close"
      title="Close"
      onclick={() => win.close()}
    >
      <svg viewBox="0 0 10 10" aria-hidden="true"
        ><path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1" /></svg
      >
    </button>
  </div>
</header>

<style>
  .bar {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-4);
    /* Nothing in the bar is text to select; it is all grab area or control. */
    user-select: none;
    /* Breathing room above the app's own controls. The window buttons stretch
       to the full bar height on purpose — that is the Windows metric — so the
       padding sits on the bar and the buttons opt out of it below. */
    padding: var(--space-4) 0 var(--space-3) var(--space-6);
    flex: none;
  }

  .drag-surface {
    position: absolute;
    inset: 0;
    /* Behind every control, so buttons keep their own clicks. */
    z-index: 0;
  }

  /* Above the drag surface, which is the only thing this has to clear.

     `.tools` IS EXCLUDED, and that exclusion is the fix for a real bug.
     `z-index: 1` makes a STACKING CONTEXT, and the tools menu -- an overlay
     with `z-index: 30` -- lives inside `.tools`. Trapped in a context at 1, its
     30 was relative to nothing: the open menu could not rise above any sibling
     of the title bar at 1 or more. Settings' sticky header is `z-index: 1` and
     comes later in document order, so the word "Settings" and the rule under it
     painted straight through the open dropdown.

     `.tools` keeps `position: relative`, which is what the menu is positioned
     against and which does NOT create a stacking context on its own. The menu's
     30 then applies in the root context, where it means what it says. Being
     positioned and later in the DOM than `.drag-surface` already puts it above
     it, which is all the z-index was for. */
  .brand,
  .controls,
  .bar :global(> *:not(.drag-surface):not(.tools)) {
    position: relative;
    z-index: 1;
  }

  .brand {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-title-size);
    line-height: var(--text-title-lh);
    font-weight: var(--weight-medium);
    letter-spacing: -0.01em;
    color: var(--text-primary);
  }

  .controls {
    margin-left: auto;
    /* The app shell insets content by --space-5. Caption buttons are window
       chrome, not content, so give that inset back at the physical edge. */
    margin-right: calc(-1 * var(--space-5));
    display: flex;
    align-self: stretch;
    /* Cancel the bar's vertical padding: a caption button that stops short of
       the window's top edge is one users miss when the pointer is thrown to
       the corner. */
    margin-block: calc(-1 * var(--space-4)) calc(-1 * var(--space-3));
  }

  /* 46x32 is the Windows 11 caption-button metric; matching it is what makes
     a custom frame feel like a window rather than a web page. */
  .ctl {
    width: 46px;
    min-height: 32px;
    align-self: stretch;
    display: grid;
    place-items: center;
    color: var(--text-secondary);
    transition:
      background var(--motion-micro),
      color var(--motion-micro);
  }

  .ctl svg {
    width: 10px;
    height: 10px;
  }

  .ctl:hover {
    background: var(--surface-sunken);
    color: var(--text-primary);
  }

  .close:hover {
    background: #c42b1c;
    color: #ffffff;
  }

  @media (prefers-reduced-motion: reduce) {
    .ctl {
      transition: none;
    }
  }
</style>
