// Delayed-commit dismiss shared by Overview and Subjects. Dismissing doesn't
// call the backend at once: the item collapses to a "Dismissed · Undo" line and
// the commit fires after DISMISS_UNDO_MS. Undo just cancels the timer — no
// backend call was made, so there's nothing to un-dismiss.
export const DISMISS_UNDO_MS = 5000;

interface Pending<T> {
  timer: ReturnType<typeof setTimeout>;
  item: T;
}

export class DelayedDismiss<T extends { id: number }> {
  // Reassigned (not mutated) so `$state` reacts — plain Maps aren't deep-proxied.
  #pending = $state<Map<number, Pending<T>>>(new Map());
  #commit: (item: T) => void;

  constructor(commit: (item: T) => void) {
    this.#commit = commit;
  }

  has(id: number): boolean {
    return this.#pending.has(id);
  }

  start(item: T): void {
    if (this.#pending.has(item.id)) return;
    const timer = setTimeout(() => this.#fire(item), DISMISS_UNDO_MS);
    const next = new Map(this.#pending);
    next.set(item.id, { timer, item });
    this.#pending = next;
  }

  undo(item: T): void {
    clearTimeout(this.#pending.get(item.id)?.timer);
    this.#drop(item.id);
  }

  // Commit every pending dismiss now (e.g. the user navigates away mid-window:
  // they did dismiss, leaving shouldn't quietly undo it).
  flush(): void {
    for (const { timer, item } of this.#pending.values()) {
      clearTimeout(timer);
      this.#fire(item);
    }
  }

  #fire(item: T): void {
    this.#drop(item.id);
    this.#commit(item);
  }

  #drop(id: number): void {
    const next = new Map(this.#pending);
    next.delete(id);
    this.#pending = next;
  }
}
