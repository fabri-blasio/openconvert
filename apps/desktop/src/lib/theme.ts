/**
 * Light, dark, or whatever the system says.
 *
 * **`data-theme` is always one of `light` or `dark`** — never absent, never
 * `system`. The preference is resolved here and stamped on `<html>`, so
 * `tokens.css` carries exactly one dark palette.
 *
 * The previous arrangement left the attribute off for "system" and matched
 * `prefers-color-scheme` in CSS instead, which meant the dark palette had to be
 * written twice — once in a media query and once under `[data-theme="dark"]`.
 * Two copies of a palette is two palettes, and they drifted: the surfaces in
 * one of them pointed at the light end of the ramp while the text was
 * near-white. Resolving in JS removes the second copy and the whole class of
 * bug with it.
 *
 * **The setting itself lives in the user config** (the settings screen writes it
 * through `set_config`). `localStorage` is a cache-only fallback so the first
 * paint, before the config command answers, matches what the user chose last
 * time. It holds one enum. Nothing about a user's files is ever written to the
 * webview's storage: the webview is outside the trust boundary, and what it can
 * remember it can leak.
 */

export type Theme = "light" | "dark" | "system";

const KEY = "openconvert.theme";

const darkQuery = (): MediaQueryList | null =>
  typeof window !== "undefined" && window.matchMedia
    ? window.matchMedia("(prefers-color-scheme: dark)")
    : null;

/** Read the cached preference, defaulting to `system`. */
export function stored(): Theme {
  try {
    const value = localStorage.getItem(KEY);
    return value === "light" || value === "dark" ? value : "system";
  } catch {
    // Storage can be unavailable. A theme is not worth failing over.
    return "system";
  }
}

/** What the user would actually see for this preference, right now. */
export function effective(theme: Theme): "light" | "dark" {
  if (theme !== "system") return theme;
  return darkQuery()?.matches ? "dark" : "light";
}

/** The preference currently applied, so the OS listener knows when to act. */
let current: Theme = "system";
let watching = false;

/** Apply a preference to the document, and remember it. */
export function apply(theme: Theme) {
  current = theme;
  document.documentElement.setAttribute("data-theme", effective(theme));

  // Only "system" needs to follow the OS, and it has to follow it live —
  // a window open across sunset should not need restarting.
  if (!watching) {
    watching = true;
    darkQuery()?.addEventListener("change", () => {
      if (current === "system") {
        document.documentElement.setAttribute("data-theme", effective("system"));
      }
    });
  }

  try {
    localStorage.setItem(KEY, theme);
  } catch {
    // Not remembering is a smaller failure than not applying.
  }
}

/** The next theme in the cycle: system → light → dark → system. */
export function next(theme: Theme): Theme {
  return theme === "system" ? "light" : theme === "light" ? "dark" : "system";
}
