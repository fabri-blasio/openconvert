/**
 * Where a file dropped on the window goes.
 *
 * **One listener, one owner at a time.** The OS drag-drop subscription used to
 * live in `App.svelte` alone and route everything into the conversion batch. A
 * tool workspace open over the top got nothing: dropping a picture onto the
 * image editor either did nothing visible, or — before the shell learned to
 * check — was quietly turned into a conversion behind the open workspace,
 * which is worse, because it wrote files nobody asked for.
 *
 * The fix is not a second listener. Two subscriptions to the same OS event
 * both fire, both think they own the drop, and which one wins is whichever
 * order they happened to subscribe in. Instead there is one subscription here
 * and a STACK of registered targets: whoever is on screen registers, the most
 * recently registered enabled target takes the drop, and unregistering on
 * unmount hands it back to whatever was underneath.
 *
 * `dragging` is shared deliberately. Every surface that can receive a drop
 * shows the same overlay, and a single flag is what keeps them from disagreeing
 * about whether a drag is in progress.
 *
 * **Nothing here reads a file.** Tauri hands over paths; the webview never sees
 * bytes, and a `DataTransfer` is never consulted — which is what keeps the
 * decoder boundary where `09` §1 puts it.
 */

import { getCurrentWebview } from "@tauri-apps/api/webview";

/** One surface that can accept a drop. */
export interface DropTarget {
  /**
   * Whether this target will take a drop right now.
   *
   * A target that is mounted but busy — mid-conversion, or still probing the
   * last drop — returns false, and the drop falls through to whatever is
   * beneath it rather than being swallowed.
   */
  enabled: () => boolean;
  /** Called with the dropped paths. */
  onFiles: (paths: string[]) => void;
}

class DropState {
  /** True while a drag is over the window and some target would take it. */
  dragging = $state(false);

  /** Registered targets, oldest first. The last enabled one wins. */
  #targets: DropTarget[] = [];

  /** Unsubscribe from the OS event, once nothing is listening. */
  #unlisten: (() => void) | null = null;
  #starting = false;

  /**
   * Claim the drop for `target` until the returned function is called.
   *
   * Registration order is mount order, so a workspace opening over the
   * converter lands on top and takes drops; closing it pops it off and the
   * converter has them back. No component has to know what else exists.
   */
  register(target: DropTarget): () => void {
    this.#targets.push(target);
    this.#start();
    return () => {
      const at = this.#targets.indexOf(target);
      if (at >= 0) this.#targets.splice(at, 1);
      if (this.#targets.length === 0) this.#stop();
    };
  }

  /** The target a drop would go to right now, or null. */
  get active(): DropTarget | null {
    for (let i = this.#targets.length - 1; i >= 0; i -= 1) {
      const target = this.#targets[i];
      if (target.enabled()) return target;
    }
    return null;
  }

  #start() {
    if (this.#unlisten !== null || this.#starting) return;
    this.#starting = true;
    void getCurrentWebview()
      .onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === "over") {
          // Only light up when something would actually take the files.
          // Promising a drop the app will then ignore is worse than no
          // feedback at all.
          this.dragging = this.active !== null;
          return;
        }
        this.dragging = false;
        if (payload.type !== "drop") return;
        if (payload.paths.length === 0) return;
        this.active?.onFiles(payload.paths);
      })
      .then((fn) => {
        this.#starting = false;
        // Everything unregistered while the subscription was in flight: drop
        // it immediately rather than leaving a listener nothing can reach.
        if (this.#targets.length === 0) {
          fn();
          return;
        }
        this.#unlisten = fn;
      })
      .catch(() => {
        // No drag-drop channel (a browser harness without the shim). Drag is
        // simply never reported; every other way of choosing files still works.
        this.#starting = false;
      });
  }

  #stop() {
    this.#unlisten?.();
    this.#unlisten = null;
    this.dragging = false;
  }
}

export const drop = new DropState();
