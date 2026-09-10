/**
 * Keyboard shortcuts.
 *
 * One table, so the set is inspectable rather than scattered across
 * `onkeydown` handlers, and so the help text and the behaviour cannot drift.
 *
 * The rule every binding obeys: **a shortcut never does something the user
 * cannot see about to happen.** Enter runs the plan that is on screen; it does
 * nothing while no plan is showing.
 */

export type Action =
  | "confirm"
  | "cancel"
  | "undo"
  | "reset"
  | "toggleAlternates"
  | "toggleTheme"
  | "openFiles"
  | "openSettings"
  | "openTools"
  | "panic"
  | `alternate:${number}`;

export interface Binding {
  /** What to show in the help line. */
  label: string;
  /** Which action it fires. */
  action: Action;
  /** What it does, in one line. Rendered by the settings screen. */
  does: string;
}

/** Shown in the footer, and explained in Settings, in this order. */
export const HELP: Binding[] = [
  { label: "Enter", action: "confirm", does: "Run the plan on screen" },
  { label: "Space", action: "toggleAlternates", does: "Show other target formats" },
  { label: "1–9", action: "alternate:0", does: "Pick a target by its number" },
  { label: "Esc", action: "cancel", does: "Go back, or stop a running batch" },
  { label: "Backspace", action: "reset", does: "Clear the drop and start over" },
  { label: "Ctrl+Z", action: "undo", does: "Remove the last outputs written" },
  { label: "Ctrl+O", action: "openFiles", does: "Choose files to convert" },
  { label: "Ctrl+D", action: "toggleTheme", does: "Cycle system, light, dark" },
  { label: "Ctrl+,", action: "openSettings", does: "Open settings" },
  { label: "Alt+T", action: "openTools", does: "Open the tools menu" },
];

/** What the panic chord does, beside {@link PANIC_LABEL}. */
export const PANIC_DOES = "Stop every queued batch at the next file";

/** The panic chord. A webview-handled chord rather than a global shortcut:
 *  the global-shortcut plugin would need a new entry on the gated capability
 *  allowlist for no gain — the window is where the user already is. */
export const PANIC_LABEL = "Ctrl+Shift+P";

/**
 * Map a key event to an action, or `null`.
 *
 * Typing in a field is never a shortcut: an editable target short-circuits the
 * whole table rather than each binding remembering to check.
 */
export function resolve(e: KeyboardEvent): Action | null {
  const target = e.target as HTMLElement | null;
  if (target?.isContentEditable) return null;
  const tag = target?.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return null;

  const mod = e.metaKey || e.ctrlKey;

  if (mod && e.shiftKey && e.key.toLowerCase() === "p") return "panic";
  if (mod && e.key.toLowerCase() === "z") return "undo";
  if (mod && e.key.toLowerCase() === "d") return "toggleTheme";
  if (mod && e.key.toLowerCase() === "o") return "openFiles";
  if (mod && e.key === ",") return "openSettings";
  if (e.altKey && !mod && e.key.toLowerCase() === "t") return "openTools";
  if (mod) return null;

  switch (e.key) {
    case "Enter":
      return "confirm";
    case "Escape":
      return "cancel";
    case " ":
      return "toggleAlternates";
    case "Backspace":
      return "reset";
    default:
      break;
  }

  // 1–9 pick an alternate by position, which is why the list is numbered.
  if (e.key >= "1" && e.key <= "9") {
    return `alternate:${Number(e.key) - 1}`;
  }
  return null;
}

/** The index behind an `alternate:N` action. */
export function alternateIndex(action: Action): number | null {
  return action.startsWith("alternate:") ? Number(action.slice("alternate:".length)) : null;
}
