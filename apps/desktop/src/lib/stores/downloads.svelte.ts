/**
 * Model downloads in flight, outside the panels that start them.
 *
 * # The defect this exists for
 *
 * **Closing Settings appeared to cancel a download.** It never did — the fetch
 * runs in the host process, and dismissing a webview panel has no way to reach
 * it. What closing destroyed was every trace of it: `downloads` and
 * `installing` were `$state` inside `SettingsView`, so the panel took the
 * progress with it, and reopening showed a row that offered to download a
 * feature that was at that moment halfway downloaded. Pressing it again is the
 * obvious next move, and the obvious next move was wrong.
 *
 * That is worse than cancelling, because a cancelled download at least stops.
 * This one kept going with nothing on screen accounting for it.
 *
 * So the progress lives here, above every panel, subscribed once. The panels
 * read it and render it; none of them owns it, and closing one is what it
 * always claimed to be — closing a window over work that continues.
 *
 * # What this does NOT do
 *
 * It does not cancel anything, and there is no method here that could. The
 * registry download is a host-side operation with its own pinning and
 * verification; the honest interface to it from the webview is "start it" and
 * "watch it". A Cancel button would need the backend to support one, and
 * inventing a frontend-side one that merely stops listening is exactly the
 * illusion this file exists to remove.
 */

import { onModelProgress, type ModelProgress } from "../ipc";

class DownloadStore {
  /** The latest progress event per artifact id. */
  progress = $state<Record<string, ModelProgress>>({});

  /**
   * The features and models a download has been STARTED for.
   *
   * Kept separately from `progress` because the two answer different
   * questions. Progress is per artifact and arrives from the backend; this is
   * per thing-the-user-asked-for and is known the instant they ask. A feature
   * is several artifacts, so between pressing the button and the first byte of
   * the first file there are no events at all — and a spinner driven purely by
   * events would not appear until then.
   */
  active = $state<Set<string>>(new Set());

  #stop: (() => void) | null = null;
  #listeners = 0;

  /**
   * Begin watching, if nothing is watching yet.
   *
   * Reference-counted rather than "subscribe on first import": both Settings
   * and the first-run screen mount and unmount independently, and either one
   * unsubscribing while the other is open would blank the other's progress.
   * Returns the release function; the last one out stops the listener.
   */
  watch(): () => void {
    this.#listeners += 1;
    if (this.#stop === null) {
      let off: (() => void) | undefined;
      let released = false;
      void onModelProgress((p) => {
        this.progress = { ...this.progress, [p.id]: p };
        // A finished artifact stops being interesting the moment the next one
        // starts, but the FEATURE it belongs to may have more to come -- so
        // the row is cleared by whoever started it, not here.
      }).then((fn) => {
        if (released) fn();
        else off = fn;
      });
      this.#stop = () => {
        released = true;
        off?.();
      };
    }
    return () => {
      this.#listeners -= 1;
      if (this.#listeners <= 0) {
        this.#listeners = 0;
        this.#stop?.();
        this.#stop = null;
      }
    };
  }

  /** Note that something was asked for, before any bytes arrive. */
  begin(id: string) {
    this.active = new Set([...this.active, id]);
  }

  /** Note that it finished, however it finished. */
  end(id: string) {
    const next = new Set(this.active);
    next.delete(id);
    this.active = next;
    const { [id]: _gone, ...rest } = this.progress;
    this.progress = rest;
  }

  /** Whether anything at all is being fetched right now. */
  readonly busy = $derived(this.active.size > 0);

  /**
   * How far `id` has got, 0 to 1, or `null` for "cannot say".
   *
   * `null` and not 0 when the source declares no length: a bar frozen at zero
   * looks broken, and an indeterminate one looks busy, which is the true one.
   */
  fractionFor(id: string): number | null {
    const p = this.progress[id];
    if (!p) return null;
    if (p.phase === "verifying") return 1;
    if (p.phase !== "downloading") return null;
    return p.totalBytes > 0 ? Math.min(p.receivedBytes / p.totalBytes, 1) : null;
  }
}

export const downloads = new DownloadStore();
