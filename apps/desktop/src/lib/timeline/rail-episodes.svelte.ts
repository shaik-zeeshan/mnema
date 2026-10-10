// Engine Activity episodes for the Timeline rail's loaded frame range, used
// only to colour app runs (see rail-categories.ts).
import { invoke } from "@tauri-apps/api/core";
import type { Activity } from "$lib/types/recording";
import { categoryColor, categoryForSpan } from "./rail-categories";

// The live refresh grows the newest edge every ~1.5 s. Fetching this far past
// it means a refetch at most every 10 min, which also picks up episodes the
// engine has distilled since.
const LIVE_EDGE_PAD_MS = 10 * 60_000;

export class RailEpisodes {
  episodes = $state.raw<Activity[]>([]);
  #covered: [number, number] | null = null;
  #seq = 0;

  /** Refetch only when the loaded range leaves the range last fetched. A
   *  failure is silent: bands keep what they had (neutral until a success). */
  sync(oldestMs: number, newestMs: number): void {
    const c = this.#covered;
    if (c && oldestMs >= c[0] && newestMs <= c[1]) return;
    const [startMs, endMs] = (this.#covered = [oldestMs, newestMs + LIVE_EDGE_PAD_MS]);
    const seq = ++this.#seq;
    invoke<Activity[]>("list_user_context_activities", { startMs, endMs }).then(
      (rows) => {
        if (seq === this.#seq) this.episodes = rows;
      },
      () => {},
    );
  }

  colorFor(startMs: number, endMs = startMs): string {
    return categoryColor(categoryForSpan(startMs, endMs, this.episodes));
  }
}
