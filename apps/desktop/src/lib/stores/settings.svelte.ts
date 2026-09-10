/**
 * The configuration store.
 *
 * **Every control reads and writes through the config commands** — there is
 * no `localStorage` setting anywhere except the theme cache below. The store
 * holds the backend's *effective* config (defaults already applied there), so
 * a control renders what the app actually means rather than guessing.
 *
 * Applying the theme is done here, once, so every writer — settings
 * screen, header toggle, keyboard chord — lands identically.
 */

import { getConfig, setConfig, errorText, type Config, type ConfigPatch } from "../ipc";
import { apply as applyTheme, stored as cachedTheme } from "../theme";

class SettingsStore {
  /** The effective config, or null until the first load lands. */
  config = $state<Config | null>(null);
  error = $state<string | null>(null);
  loaded = $state(false);

  async load() {
    try {
      this.config = await getConfig();
      this.error = null;
    } catch (e) {
      // A config that cannot be read must not stop the UI. Fall back to the
      // localStorage theme cache (cache-only by design) and say nothing
      // until the user tries to change something.
      this.error = errorText(e);
    } finally {
      this.loaded = true;
      this.applyAppearance();
    }
  }

  /** Write one patch through to the backend and adopt the returned truth. */
  async patch(change: ConfigPatch): Promise<boolean> {
    try {
      this.config = await setConfig(change);
      this.error = null;
      this.applyAppearance();
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    }
  }

  /** Stamp appearance onto `<html>`. Config first, cache-only fallback. */
  applyAppearance() {
    applyTheme(this.config?.theme ?? cachedTheme());
  }

  get theme(): "system" | "light" | "dark" {
    return this.config?.theme ?? cachedTheme();
  }

  /** Cycle the theme from the header button or its chord. */
  async cycleTheme(next: "system" | "light" | "dark") {
    await this.patch({ theme: next });
  }
}

export const settings = new SettingsStore();
