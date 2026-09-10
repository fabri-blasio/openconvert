/**
 * The 60-second undo window.
 *
 * The countdown is presentation. **The backend owns what "undo" means**: it
 * holds the list of files the last batch created, and `undo_last` deletes
 * exactly those. Nothing here names a path, which is what keeps the one
 * deletion path in the process unable to be pointed at anything else.
 *
 * When the window expires the frontend simply stops offering the button. It
 * does not tell the backend to forget — a batch that is superseded is replaced
 * by the next `convert_batch`, and a stale record that nothing can reach is
 * harmless.
 */

import { undoLast, errorText } from "../ipc";

/** How long the offer stands, in seconds. */
export const UNDO_WINDOW_SECONDS = 60;

class UndoStore {
  /** Seconds left, or 0 when nothing is offered. */
  remaining = $state(0);
  /** How many outputs the offer covers, for the toast's wording. */
  count = $state(0);
  error = $state<string | null>(null);
  done = $state(false);
  /** The window ran out. Undo is gone, but the record lives in History —
   *  which is exactly what the expired toast links to (02 §12.2). */
  expired = $state(false);

  /** How long "Outputs removed." stays up. */
  static readonly DONE_SECONDS = 5;

  readonly offered = $derived(this.remaining > 0);

  #timer: ReturnType<typeof setInterval> | null = null;
  /**
   * Clears the "Outputs removed." toast.
   *
   * **IT NEVER WENT AWAY.** `done` was set once and only `reset()` cleared it,
   * so the confirmation of an undo sat across the window until the next
   * conversion started -- covering the screen it was reporting on. Every other
   * transient message in this app clears itself; this was the one that did
   * not, and it is the one that says an operation is FINISHED, which is
   * exactly the kind that should not need dismissing.
   */
  #doneTimer: ReturnType<typeof setTimeout> | null = null;

  /** Start the window after a batch that wrote something. */
  offer(outputCount: number) {
    this.reset();
    if (outputCount <= 0) return;
    this.count = outputCount;
    this.remaining = UNDO_WINDOW_SECONDS;
    this.done = false;
    this.expired = false;
    this.error = null;
    this.#timer = setInterval(() => {
      this.remaining -= 1;
      if (this.remaining <= 0) {
        this.clear();
        // Only an offer that actually stood expires; a withdrawn one
        // (reset before acting on it) must not advertise History.
        if (this.count > 0) {
          this.expired = true;
          this.#doneTimer = setTimeout(() => { this.expired = false; this.#doneTimer = null; }, UndoStore.DONE_SECONDS * 1000);
        }
      }
    }, 1000);
  }

  /** Remove the last batch's outputs. */
  async invoke() {
    if (!this.offered) return;
    const covered = this.count;
    this.clear();
    try {
      await undoLast();
      this.count = covered;
      this.done = true;
      if (this.#doneTimer !== null) clearTimeout(this.#doneTimer);
      this.#doneTimer = setTimeout(() => {
        this.done = false;
        this.#doneTimer = null;
      }, UndoStore.DONE_SECONDS * 1000);
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Withdraw the offer without acting on it. */
  clear() {
    if (this.#timer !== null) {
      clearInterval(this.#timer);
      this.#timer = null;
    }
    this.remaining = 0;
  }

  dismissNotice() {
    this.expired = false;
    this.done = false;
    if (this.#doneTimer !== null) clearTimeout(this.#doneTimer);
    this.#doneTimer = null;
  }

  reset() {
    this.clear();
    if (this.#doneTimer !== null) {
      clearTimeout(this.#doneTimer);
      this.#doneTimer = null;
    }
    this.count = 0;
    this.error = null;
    this.done = false;
    this.expired = false;
  }
}

export const undo = new UndoStore();
