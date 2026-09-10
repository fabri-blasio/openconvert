/** Bounded, in-memory edit undo. Saving output has its own undo mechanism. */
export class EditHistory<T> {
  past = $state<T[]>([]);
  future = $state<T[]>([]);
  record(value: T) { this.past = [...this.past.slice(-49), JSON.parse(JSON.stringify(value))]; this.future = []; }
  undo(current: T): T | null { const previous = this.past.at(-1); if (!previous) return null; this.past = this.past.slice(0,-1); this.future = [...this.future, JSON.parse(JSON.stringify(current))]; return JSON.parse(JSON.stringify(previous)); }
  redo(current: T): T | null { const next = this.future.at(-1); if (!next) return null; this.future = this.future.slice(0,-1); this.past = [...this.past, JSON.parse(JSON.stringify(current))]; return JSON.parse(JSON.stringify(next)); }
  clear() { this.past = []; this.future = []; }
}
