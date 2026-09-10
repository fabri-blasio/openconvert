<!--
  The Tools menu: three doors, not seventeen.

  # Why it is not a list of tools any more

  It used to list every tool in every category — five headings and fourteen
  items in a panel 260 px wide, which is a second copy of the rail you land in
  the moment you choose one of them. Two lists of the same tools, one of which
  you can only see for as long as you hold the menu open, and the choice you
  make in the transient one decides nothing you cannot change in the permanent
  one.

  So the menu now answers the only question the rail cannot: WHICH KIND OF
  THING are you working on. It opens that category on its first tool, and the
  rail — which is where the tools live, beside the document, with their
  settings under them — takes it from there.

  Items still come exclusively from `list_tools()`; there is not a hardcoded
  entry here. A category with nothing available on this machine does not
  render, which is the same availability honesty as before, one level up.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { listTools, errorText, type ToolDescriptor } from "../lib/ipc";

  let { onpick }: { onpick: (tool: ToolDescriptor) => void } = $props();

  let open = $state(false);
  let tools = $state<ToolDescriptor[]>([]);
  let loadError = $state<string | null>(null);
  let loading = $state(false);
  let button = $state<HTMLButtonElement | null>(null);
  let menu = $state<HTMLDivElement | null>(null);

  const ORDER = ["image", "pdf", "audio", "video"];

  /** What each category is called to a user.

      The registry's ids are singular and lower-case because they are keys.
      "image" is a key; "Images" is what the door says. Anything not listed
      falls back to its own id capitalised, so a new category appears in the
      menu the day it appears in the registry rather than the day someone
      remembers to add it here. */
  const CATEGORY_LABEL: Record<string, string> = {
    image: "Images",
    pdf: "PDF",
    audio: "Audio",
    video: "Video",
  };

  function labelFor(category: string): string {
    return CATEGORY_LABEL[category] ?? category[0].toUpperCase() + category.slice(1);
  }

  const grouped = $derived.by(() => {
    const groups = new Map<string, ToolDescriptor[]>();
    for (const tool of tools) {
      const list = groups.get(tool.category) ?? [];
      list.push(tool);
      groups.set(tool.category, list);
    }
    return [...groups.entries()].sort(
      (a, b) =>
        (ORDER.indexOf(a[0]) + 1 || ORDER.length + 1) -
        (ORDER.indexOf(b[0]) + 1 || ORDER.length + 1),
    );
  });

  const anyAvailable = $derived(tools.some((t) => t.available));

  async function refresh() {
    loading = true;
    try {
      tools = await listTools();
      loadError = null;
    } catch (e) {
      loadError = errorText(e);
    } finally {
      loading = false;
    }
  }

  export function toggle(): void {
    if (!open) void openMenu();
    else close();
  }

  async function openMenu() {
    open = true;
    await refresh();
    queueMicrotask(() => {
      menu?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    });
  }

  function close(returnFocus = true) {
    open = false;
    if (returnFocus) button?.focus();
  }

  /** Open a category on its first available tool.

      FIRST AVAILABLE, not first: opening a category on a tool the machine
      cannot run would land the user on a lit button with a reason under it
      and nothing to do, when there were four working tools below it. */
  function pickCategory(list: ToolDescriptor[]) {
    const first = list.find((t) => t.available);
    if (!first) return;
    close();
    onpick(first);
  }

  function onkeydown(e: KeyboardEvent) {
    if (!open) return;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  /** Close when focus leaves the whole control (menu or its button). */
  function onFocusoutLoss(node: HTMLElement) {
    function handler(e: FocusEvent) {
      if (!node.contains(e.relatedTarget as Node | null)) close(false);
    }
    node.addEventListener("focusout", handler);
    return {
      destroy() {
        node.removeEventListener("focusout", handler);
      },
    };
  }

  onMount(() => {
    // Warm the registry so the disabled/empty state is honest from first paint.
    void refresh();
  });
</script>

<svelte:window {onkeydown} />

<span class="tools" use:onFocusoutLoss>
  <button
    type="button"
    bind:this={button}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={toggle}
    title={anyAvailable ? "Tools (Alt+T)" : "No tools are registered in this build yet"}
  >
    Tools <span class="caret" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <div class="menu material-menu" role="menu" aria-label="Tools" bind:this={menu}>
      {#if loadError}
        <p class="note error" role="alert">{loadError}</p>
      {:else if loading && tools.length === 0}
        <p class="note" role="status">Asking the backend what it can do…</p>
      {:else if grouped.length === 0}
        <p class="note">
          No tools are registered in this build yet. Conversions stay on the main screen;
          parameterized operations land here as their handlers ship.
        </p>
      {:else}
        <!-- No headings and no groups any more: the categories ARE the items,
             so a heading would be the same word twice. The count under each
             one is the only thing the old list said that this does not, and
             it says it in three words instead of fourteen rows. -->
        <ul role="group" aria-label="Categories">
          {#each grouped as [category, list] (category)}
            {@const usable = list.filter((t) => t.available)}
            {#if usable.length > 0}
              <li role="none">
                <button
                  type="button"
                  role="menuitem"
                  title={`${labelFor(category)} — ${usable.length} tools`}
                  onclick={() => pickCategory(list)}
                >
                  <span class="cat-name">{labelFor(category)}</span>
                  <span class="cat-count" aria-hidden="true">{usable.length}</span>
                </button>
              </li>
            {/if}
          {/each}
        </ul>
        {#if !anyAvailable}
          <p class="note">
            Registered tools are unavailable on this machine; their reasons are on each item.
          </p>
        {/if}
      {/if}
    </div>
  {/if}
</span>

<style>
  .tools {
    position: relative;
  }

  button {
    min-height: 28px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-full);
    border: 0.5px solid var(--hairline-strong);
    font-size: var(--text-small-size);
    color: var(--text-primary);
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  button:hover:not(:disabled) {
    background: var(--surface-sunken);
  }

  .caret {
    color: var(--text-secondary);
  }

  .material-menu {
    position: absolute;
    top: calc(100% + var(--space-2));
    left: 0;
    z-index: 30;
    min-width: 200px;
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    box-shadow: var(--shadow-float);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    display: grid;
    gap: var(--space-1);
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
  }

  li button {
    width: 100%;
    border: none;
    /* The name at one edge and the count at the other, which is what makes
       the count read as an aside rather than as part of the name. */
    justify-content: space-between;
    min-height: 36px;
    border-radius: var(--radius-sm);
    padding: 0 var(--space-3);
    text-align: left;
  }

  .cat-count {
    font-size: var(--text-caption-size);
    color: var(--text-tertiary);
    font-variant-numeric: tabular-nums;
  }

  li button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }

  .note {
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    color: var(--text-secondary);
    padding: var(--space-2) var(--space-3);
    max-width: 34ch;
  }

  .error {
    color: var(--accent-critical);
  }

  @media (prefers-reduced-transparency: reduce) {
    .material-menu {
      background: var(--surface-raised);
      backdrop-filter: none;
    }
  }
</style>
