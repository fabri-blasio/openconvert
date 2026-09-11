<script lang="ts">
  import type { Snippet } from "svelte";
  import Dropdown, { type Choice } from "./Dropdown.svelte";
  import Icon, { type IconName } from "./Icon.svelte";

  const LABELS: Record<string, string> = {
    "image-upscale": "Upscale",
    "image-invert": "Invert colours",
    "image-greyscale": "Black and white",
    "image-remove-background": "Remove background",
  };

  const ICONS: Record<string, IconName> = {
    "image-remove-background": "eraser",
    "image-upscale": "scaling",
    "image-invert": "contrast",
    "image-greyscale": "droplet-off",
  };

  const FORMATS: Choice[] = [
    { value: "png", label: "PNG" },
    { value: "jpeg", label: "JPEG" },
    { value: "webp", label: "WebP" },
    { value: "avif", label: "AVIF" },
  ];

  let {
    ops,
    format = $bindable("png"),
    selected,
    disabled = false,
    onchange,
    onselect,
    children,
  }: {
    ops: string[];
    format?: string;
    selected: string;
    disabled?: boolean;
    onchange: (ops: string[]) => void;
    onselect: (id: string) => void;
    children?: Snippet;
  } = $props();

  let dragging = $state<string | null>(null);
  let over = $state<string | null>(null);
  let dropAfter = $state(false);
  let dragPointer = $state<number | null>(null);
  let dragStartY = 0;
  let dragMoved = false;
  let suppressClickFor: string | null = null;

  const available = $derived(
    Object.entries(LABELS)
      .filter(([id]) => !ops.includes(id))
      .map(([value, label]) => ({ value, label })),
  );

  function add(id: string) {
    onchange([...ops, id]);
    onselect(id);
  }

  function remove(id: string) {
    const next = ops.filter((value) => value !== id);
    onchange(next);
    if (selected === id && next.length > 0) {
      onselect(next[Math.min(ops.indexOf(id), next.length - 1)]);
    }
  }

  function reorder(source: string, target: string, after: boolean) {
    if (source === target) return;
    const next = ops.filter((value) => value !== source);
    const targetIndex = next.indexOf(target);
    if (targetIndex < 0) return;
    next.splice(targetIndex + (after ? 1 : 0), 0, source);
    onchange(next);
  }

  function startDrag(event: PointerEvent, id: string) {
    if (disabled || event.button !== 0) return;
    dragging = id;
    over = id;
    dragPointer = event.pointerId;
    dragStartY = event.clientY;
    dragMoved = false;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  }

  function moveDrag(event: PointerEvent) {
    if (event.pointerId !== dragPointer || !dragging) return;
    if (Math.abs(event.clientY - dragStartY) > 4) dragMoved = true;
    if (!dragMoved) return;
    event.preventDefault();
    const row = document
      .elementFromPoint(event.clientX, event.clientY)
      ?.closest<HTMLElement>("[data-edit-id]");
    const target = row?.dataset.editId;
    if (!target || target === dragging) {
      over = target ?? null;
      return;
    }
    const box = row.getBoundingClientRect();
    over = target;
    dropAfter = event.clientY > box.top + box.height / 2;
  }

  function finishDrag(event: PointerEvent, cancelled = false) {
    if (event.pointerId !== dragPointer || !dragging) return;
    const source = dragging;
    if (!cancelled && dragMoved && over && over !== source) {
      reorder(source, over, dropAfter);
    }
    if (dragMoved) {
      suppressClickFor = source;
      setTimeout(() => {
        if (suppressClickFor === source) suppressClickFor = null;
      }, 0);
    }
    dragging = null;
    over = null;
    dragPointer = null;
    dragMoved = false;
  }

  function activate(event: MouseEvent, id: string) {
    if (suppressClickFor === id) {
      event.preventDefault();
      suppressClickFor = null;
      return;
    }
    onselect(id);
  }
</script>

<div class="workflow">
  <Dropdown
    options={available}
    placeholder="Choose an edit…"
    label="Choose an edit"
    {disabled}
    onchange={add}
  />

  <ol aria-label="Edits in execution order">
    {#each ops as id (id)}
      <li
        data-edit-id={id}
        class:open={selected === id}
        class:dragging={dragging === id}
        class:over={over === id && dragging !== id}
      >
        <div class="cmd-row">
          <button
            type="button"
            class="cmd on"
            class:current={selected === id}
            aria-pressed="true"
            aria-expanded={selected === id}
            onclick={(event) => activate(event, id)}
          >
            <span
              class="cmd-icon"
              role="presentation"
              title="Drag to reorder"
              onpointerdown={(event) => startDrag(event, id)}
              onpointermove={moveDrag}
              onpointerup={(event) => finishDrag(event)}
              onpointercancel={(event) => finishDrag(event, true)}
            >
              <Icon name={ICONS[id]} size={16} />
            </span>
            <span class="cmd-text">
              <span class="cmd-title">{LABELS[id]}</span>
            </span>
          </button>
          <button
            type="button"
            class="trash"
            {disabled}
            aria-label={`Remove ${LABELS[id]}`}
            onclick={() => remove(id)}
          >
            <Icon name="trash-2" size={15} />
          </button>
        </div>

        {#if selected === id}
          <div class="edit-settings">{@render children?.()}</div>
        {/if}
      </li>
    {/each}
  </ol>

  <div class="output-format">
    <span>Output format</span>
    <Dropdown
      options={FORMATS}
      value={format}
      label="Output format"
      {disabled}
      onchange={(value) => (format = value)}
    />
  </div>
</div>

<style>
  .workflow {
    display: grid;
    gap: var(--space-4);
    padding: var(--space-4);
    min-width: 0;
    font-size: var(--text-small-size);
  }

  .workflow > :global(.dropdown),
  .workflow > :global(.dropdown .trigger) { width: 100%; }

  ol {
    padding: 0;
    margin: 0;
    list-style: none;
    display: grid;
    gap: 2px;
  }

  li {
    min-width: 0;
    transition: opacity var(--motion-micro);
  }

  li.open {
    background: var(--surface-base);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-md);
    box-shadow: var(--shadow-card);
  }

  li.dragging { opacity: 0.55; }
  li.over { box-shadow: inset 0 2px 0 var(--emphasis); }

  .cmd-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 32px;
    align-items: center;
  }

  .cmd {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-width: 0;
    min-height: 36px;
    padding: var(--space-2);
    border: 1px solid var(--hairline-strong);
    border-radius: var(--radius-sm);
    background: var(--surface-base);
    box-shadow: var(--shadow-card);
    color: var(--text-primary);
    text-align: left;
    font-size: var(--text-small-size);
    line-height: var(--text-small-lh);
    transition: background var(--motion-micro), border-color var(--motion-micro);
  }

  .cmd:hover:not(:disabled) { background: var(--surface-raised); }
  li.open .cmd {
    background: none;
    border-color: transparent;
    box-shadow: none;
  }

  .cmd-icon {
    flex: none;
    width: 26px;
    height: 26px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    border: 0.5px solid var(--hairline-strong);
    color: var(--text-primary);
    cursor: grab;
    touch-action: none;
    user-select: none;
  }
  .cmd-icon:active { cursor: grabbing; }
  .cmd-text { display: grid; min-width: 0; text-align: left; }
  .cmd-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--weight-medium);
  }

  .trash {
    display: grid;
    place-items: center;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
  }

  .trash:hover:not(:disabled) {
    color: var(--accent-critical);
    background: var(--surface-raised);
  }

  .trash:disabled { opacity: 0.45; }

  .edit-settings {
    padding: 0 var(--space-3) var(--space-3);
  }

  .edit-settings :global(.tool-settings) { padding: 0; }

  .output-format {
    display: grid;
    gap: var(--space-3);
    padding-top: var(--space-4);
    border-top: 0.5px solid var(--hairline);
  }

  .output-format > :global(.dropdown),
  .output-format > :global(.dropdown .trigger) { width: 100%; }
</style>
