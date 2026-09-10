/**
 * Stand-in for `@tauri-apps/api/window` in the browser preview. **Not shipped.**
 *
 * A browser tab has no window to minimise, so the controls render and report
 * their state without doing anything. `isMaximized` is tracked locally so the
 * maximise/restore icon still swaps — the part of the title bar that has a
 * visual state worth checking.
 */

let maximized = false;
const resizeHandlers = new Set<() => void>();

export function getCurrentWindow() {
  return {
    isMaximized: () => Promise.resolve(maximized),
    minimize: () => Promise.resolve(),
    toggleMaximize: () => {
      maximized = !maximized;
      for (const h of resizeHandlers) h();
      return Promise.resolve();
    },
    close: () => Promise.resolve(),
    startDragging: () => Promise.resolve(),
    startResizeDragging: () => Promise.resolve(),
    onResized: (handler: () => void) => {
      resizeHandlers.add(handler);
      return Promise.resolve(() => resizeHandlers.delete(handler));
    },
  };
}
