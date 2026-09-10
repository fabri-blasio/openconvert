/**
 * Stand-in for `@tauri-apps/api/webview` in the browser preview. **Not shipped.**
 *
 * A browser has no OS drag-drop channel and a `DataTransfer` never carries a
 * real path, so nothing here pretends otherwise. What it does do is keep the
 * handler the app registers and expose a way to CALL it with paths — which is
 * precisely what Tauri does on a real drop, and the only difference is where
 * the list of paths comes from.
 *
 * This used to return a no-op. That was fine while the only subscriber was the
 * drop screen and the Browse button exercised the same function — and it
 * stopped being fine the moment the shell took the subscription over, because
 * "a drop onto a loaded batch adds to it" then had no way to be driven at all.
 * A harness that cannot reach a path is a harness that cannot catch it
 * breaking.
 */

type DragDropPayload =
  | { type: "over" }
  | { type: "drop"; paths: string[] }
  | { type: "leave" };

type Handler = (event: { payload: DragDropPayload }) => void;

const handlers = new Set<Handler>();

export function getCurrentWebview() {
  return {
    onDragDropEvent(handler: Handler) {
      handlers.add(handler);
      return Promise.resolve(() => {
        handlers.delete(handler);
      });
    },
  };
}

/** Fire a drag-drop payload at every registered handler. */
export function emitDragDrop(payload: DragDropPayload): void {
  for (const handler of handlers) handler({ payload });
}

// Reachable from the browser console and from a driving script, so the drop
// path can be exercised on every screen rather than only where a button
// happens to call the same function.
declare global {
  interface Window {
    __previewDrop?: (paths: string[]) => void;
  }
}
if (typeof window !== "undefined") {
  window.__previewDrop = (paths: string[]) => {
    emitDragDrop({ type: "over" });
    emitDragDrop({ type: "drop", paths });
  };
}
