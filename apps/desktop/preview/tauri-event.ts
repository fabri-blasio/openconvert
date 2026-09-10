/**
 * Stand-in for `@tauri-apps/api/event` in the browser preview. **Not shipped.**
 *
 * A one-file event bus with the same shape as Tauri's: `listen` returns an
 * unlisten function, and handlers receive `{ payload }`.
 */

export type UnlistenFn = () => void;

type Handler = (event: { payload: unknown }) => void;

const handlers = new Map<string, Set<Handler>>();

export function listen<T>(
  name: string,
  handler: (event: { payload: T }) => void,
): Promise<UnlistenFn> {
  const set = handlers.get(name) ?? new Set<Handler>();
  set.add(handler as Handler);
  handlers.set(name, set);
  return Promise.resolve(() => set.delete(handler as Handler));
}

/** Preview-only: what the Rust side would emit. */
export function emit(name: string, payload: unknown) {
  for (const h of handlers.get(name) ?? []) h({ payload });
}
