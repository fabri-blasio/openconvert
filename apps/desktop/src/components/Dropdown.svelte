<!--
  A menu that looks like the rest of the app's menus.

  # Why this exists

  There was exactly ONE `<select>` in the whole product — the signature picker
  — and it was the only control that did not look like anything else on screen.
  A native `<select>`'s popup is drawn by the operating system: it cannot take
  the app's material, its radius, its type or its spacing, and on Windows it
  arrives as a grey list with a system chevron in the middle of a screen that
  has neither. Every other menu here is a button and a floating panel.

  So the same shape, and now in one place: the trigger and the panel are
  `ToolsMenu`'s, down to the tokens, and anything else that needs a menu can
  have one without a second copy.

  # What it is not

  Not a `<select>` replacement in general. It has no type-ahead, no multiple
  selection and no native form participation, because nothing here needs them
  and each would be a feature to maintain for one caller. It keeps the parts
  that matter: a real `aria-haspopup` listbox, arrow-key movement, Enter and
  Space to choose, Escape to leave, and focus returning to the trigger.
-->
<script lang="ts">
  import Icon from "./Icon.svelte";
  /**
   * One choice. `hint` renders quieter, on the same line.
   *
   * `preview` is a data URI shown as a small thumbnail before the label, for
   * lists whose items ARE pictures -- a signature is recognised by its shape
   * long before its name is read.
   */
  export type Choice = { value: string; label: string; hint?: string; preview?: string; removable?: boolean };

  let {
    options,
    value = null,
    placeholder = "Choose…",
    label,
    disabled = false,
    onchange,
    onremove,
  }: {
    options: Choice[];
    /** The selected value, or null when nothing is chosen. */
    value?: string | null;
    /** What the trigger says with nothing chosen. */
    placeholder?: string;
    /** The accessible name; there is no visible label. */
    label: string;
    disabled?: boolean;
    onchange: (value: string) => void;
    onremove?: (value: string) => void;
  } = $props();

  let open = $state(false);
  let trigger = $state<HTMLButtonElement | null>(null);
  let panel = $state<HTMLElement | null>(null);
  /** Which option the keyboard is on. -1 while the pointer is in charge. */
  let active = $state(-1);

  const selected = $derived(options.find((o) => o.value === value) ?? null);

  function show() {
    if (disabled) return;
    open = true;
    active = Math.max(
      0,
      options.findIndex((o) => o.value === value),
    );
  }

  function hide(refocus = true) {
    open = false;
    active = -1;
    if (refocus) trigger?.focus();
  }

  function choose(option: Choice) {
    onchange(option.value);
    hide();
  }

  function onKeydown(event: KeyboardEvent) {
    if (!open) {
      if (event.key === "ArrowDown" || event.key === "Enter" || event.key === " ") {
        event.preventDefault();
        show();
      }
      return;
    }
    switch (event.key) {
      case "Escape":
        event.preventDefault();
        hide();
        break;
      case "ArrowDown":
        event.preventDefault();
        active = (active + 1) % options.length;
        break;
      case "ArrowUp":
        event.preventDefault();
        active = (active - 1 + options.length) % options.length;
        break;
      case "Home":
        event.preventDefault();
        active = 0;
        break;
      case "End":
        event.preventDefault();
        active = options.length - 1;
        break;
      case "Enter":
      case " ": {
        event.preventDefault();
        const option = options[active];
        if (option) choose(option);
        break;
      }
      default:
        break;
    }
  }

  /**
   * Close when focus or a click leaves the whole control.
   *
   * `focusout` alone is not enough: a pointer press on the surrounding page
   * moves focus to the body in some browsers and to nothing in others.
   */
  function onWindowPointerDown(event: PointerEvent) {
    if (!open) return;
    const target = event.target as Node | null;
    if (target && (trigger?.contains(target) || panel?.contains(target))) return;
    hide(false);
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<span class="dropdown">
  <button
    type="button"
    bind:this={trigger}
    class="trigger"
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label={label}
    {disabled}
    onclick={() => (open ? hide(false) : show())}
    onkeydown={onKeydown}
  >
    {#if selected?.preview}
      <img class="preview" src={selected.preview} alt="" />
    {/if}
    <span class="value" class:placeholder={!selected}>{selected?.label ?? placeholder}</span>
    <span class="caret" aria-hidden="true">▾</span>
  </button>

  {#if open}
    <div
      bind:this={panel}
      class="menu material-menu"
      role="listbox"
      aria-label={label}
      tabindex="-1"
    >
      {#each options as option, i (option.value)}
        <div class="option-row">
        <button
          type="button"
          role="option"
          aria-selected={option.value === value}
          class:active={i === active}
          onpointerenter={() => (active = i)}
          onclick={() => choose(option)}
        >
          {#if option.preview}
            <img class="preview" src={option.preview} alt="" />
          {/if}
          <span class="label">{option.label}</span>
          {#if option.hint}<span class="hint">{option.hint}</span>{/if}
        </button>
        {#if option.removable && onremove}
          <button type="button" class="remove" aria-label={`Delete ${option.label}`} onclick={() => onremove?.(option.value)}>
            <Icon name="trash-2" size={15} />
          </button>
        {/if}
        </div>
      {/each}
    </div>
  {/if}
</span>

<style>
  .option-row { display: flex; align-items: center; }
  .option-row > button:first-child { flex: 1; min-width: 0; }
  .option-row .remove { width: 30px; flex: none; padding: var(--space-2); color: var(--text-secondary); }
  .option-row .remove:hover { color: var(--accent-critical); }
  .dropdown {
    position: relative;
    display: inline-block;
    min-width: 0;
  }

  .trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: 30px;
    min-width: 170px;
    max-width: 100%;
    padding: 0 var(--space-3);
    border: 0.5px solid var(--hairline-strong);
    border-radius: var(--radius-sm);
    background: var(--surface-sunken);
    color: var(--text-primary);
    font-size: var(--text-small-size);
    text-align: left;
  }

  .trigger:hover:not(:disabled) {
    background: var(--surface-raised);
  }

  .trigger:disabled {
    color: var(--text-disabled);
  }

  /* A thumbnail of the thing the row IS. Contained, not cropped: a signature
     is mostly whitespace and `cover` would show the middle of a stroke. */
  .preview {
    flex: none;
    width: 28px;
    height: 18px;
    object-fit: contain;
    border-radius: 2px;
  }

  .value {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .value.placeholder {
    color: var(--text-secondary);
  }

  .caret {
    color: var(--text-secondary);
    flex: none;
  }

  /* `ToolsMenu`'s panel, to the token. Two menus that look almost the same is
     worse than one that looks different. */
  .material-menu {
    position: absolute;
    top: calc(100% + var(--space-2));
    left: 0;
    z-index: 30;
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
    overflow-x: hidden;
    max-height: 280px;
    overflow-y: auto;
    background: var(--material-thick);
    backdrop-filter: var(--blur-thick);
    box-shadow: var(--shadow-float);
    border-radius: var(--radius-md);
    padding: var(--space-2);
    display: grid;
    gap: var(--space-1);
  }

  .material-menu button {
    display: flex;
    /* CENTRED, not baseline-aligned. With a thumbnail in the row, aligning on
       the text baseline pushes the image down by its descender. */
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    min-height: 32px;
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-primary);
    font-size: var(--text-small-size);
    text-align: left;
  }

  .material-menu button.active {
    background: var(--surface-sunken);
  }

  .material-menu button[aria-selected="true"] .label {
    font-weight: 550;
  }

  .label {
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    flex: none;
    font-size: var(--text-caption-size);
    color: var(--text-secondary);
  }

  @media (prefers-reduced-transparency: reduce) {
    .material-menu {
      background: var(--surface-raised);
      backdrop-filter: none;
    }
  }
</style>
