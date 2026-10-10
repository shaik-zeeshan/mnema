// Timeline rail category bands: an app run takes the category of the engine
// Activity episode it overlaps the most. Frames carry no category of their
// own, so a run no episode covers (engine off, not yet distilled, failed)
// stays neutral — never a guessed colour.
import type { Activity, ActivityCategory } from "$lib/types/recording";
import { CATEGORY_COLOR, UNCATEGORIZED_COLOR } from "$lib/insights/activity-helpers";

type Episode = Pick<Activity, "startedAtMs" | "endedAtMs" | "category">;

/** Category of the episode overlapping [startMs, endMs] the most (a tie goes
 *  to the earlier-starting episode); null when nothing overlaps. Spans are
 *  closed, so a single-frame run (startMs === endMs) inside an episode counts. */
export function categoryForSpan(
  startMs: number,
  endMs: number,
  episodes: readonly Episode[],
): ActivityCategory | null {
  let best: Episode | null = null;
  let bestOverlap = -1;
  for (const e of episodes) {
    const overlap = Math.min(e.endedAtMs, endMs) - Math.max(e.startedAtMs, startMs);
    if (overlap < 0) continue;
    if (overlap > bestOverlap || (overlap === bestOverlap && e.startedAtMs < best!.startedAtMs)) {
      best = e;
      bestOverlap = overlap;
    }
  }
  return best?.category ?? null;
}

/** The `--c` value for a band or the readout: the Insights category token, or
 *  the same neutral grey Insights uses for uncategorized time. */
export function categoryColor(category: ActivityCategory | null): string {
  return `var(${category ? CATEGORY_COLOR[category] : UNCATEGORIZED_COLOR})`;
}
