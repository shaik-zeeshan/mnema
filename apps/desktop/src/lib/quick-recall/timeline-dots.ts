// Pure timeline-strip math for Quick Recall search (Slice 6 of the redesign,
// per docs/quick-recall/mockups/search-redesign.html): an 8-day axis with one
// dot per fetched result at its true time, plus the mockup's min-gap pass so
// clustered dots stay individually hoverable. Plain TS so it's bun-testable
// without Svelte.

const DAY_MS = 86_400_000;

// 8 day slots: day 0 = seven days ago, day 7 = today (mockup SPAN = 8).
export const TIMELINE_DAY_SPAN = 8;

// The mockup's min-gap: each dot sits at least 1.1% of the axis after the
// previous one ("honest time mapping, then a min-gap pass so today's cluster
// stays hoverable").
export const TIMELINE_MIN_GAP_PC = 1.1;

export interface TimelineDotSource {
  /** Stable identity carried through positioning (e.g. "frame:<groupKey>"). */
  key: string;
  /** The result's true time (epoch ms): groupStartAt / absoluteStartAt. */
  timeMs: number;
}

export interface TimelineDot {
  key: string;
  /** Horizontal position as a percentage of the axis, 0 (left) … 100 (right). */
  pc: number;
}

/** Epoch ms of the axis origin: local midnight seven days before `now`. */
export function timelineAxisStartMs(now: Date): number {
  return new Date(
    now.getFullYear(),
    now.getMonth(),
    now.getDate() - (TIMELINE_DAY_SPAN - 1),
  ).getTime();
}

/** The 8 day labels, oldest first. The last two stay human ("Yesterday" /
 *  "Today", per the mockup); earlier days are short dates ("Jun 30"). */
export function timelineDayLabels(now: Date): string[] {
  const labels: string[] = [];
  for (let day = 0; day < TIMELINE_DAY_SPAN; day++) {
    const back = TIMELINE_DAY_SPAN - 1 - day;
    if (back === 0) {
      labels.push("Today");
    } else if (back === 1) {
      labels.push("Yesterday");
    } else {
      labels.push(
        new Date(
          now.getFullYear(),
          now.getMonth(),
          now.getDate() - back,
        ).toLocaleDateString(undefined, { month: "short", day: "numeric" }),
      );
    }
  }
  return labels;
}

/** Which day slot (0–7) a result falls in, clamped to the axis — results older
 *  than the axis render pinned at the left edge, so they count toward day 0
 *  (their dot visually sits under that tick). */
export function timelineDayIndex(timeMs: number, axisStartMs: number): number {
  return Math.max(
    0,
    Math.min(TIMELINE_DAY_SPAN - 1, Math.floor((timeMs - axisStartMs) / DAY_MS)),
  );
}

/** Map results to dot positions: honest time mapping (clamped to the axis),
 *  sorted ascending, then the mockup's min-gap pass pushing each dot to at
 *  least `minGapPc` after its predecessor. A final backward pass (not in the
 *  mockup, which never overflowed) pins the run inside 100% when a right-edge
 *  cluster would otherwise push dots past the strip. */
export function computeTimelineDots(
  sources: TimelineDotSource[],
  axisStartMs: number,
  minGapPc: number = TIMELINE_MIN_GAP_PC,
): TimelineDot[] {
  const spanMs = TIMELINE_DAY_SPAN * DAY_MS;
  const dots = sources
    .map(({ key, timeMs }) => ({
      key,
      pc: Math.max(0, Math.min(100, ((timeMs - axisStartMs) / spanMs) * 100)),
    }))
    .sort((a, b) => a.pc - b.pc);
  for (let k = 1; k < dots.length; k++) {
    if (dots[k].pc < dots[k - 1].pc + minGapPc) {
      dots[k].pc = dots[k - 1].pc + minGapPc;
    }
  }
  // ponytail: backward clamp assumes n*minGap <= 100 (max 36 fetched results
  // → 38.5% worst case); revisit if fetch limits ever grow past ~90 results.
  if (dots.length > 0 && dots[dots.length - 1].pc > 100) {
    dots[dots.length - 1].pc = 100;
    for (let k = dots.length - 2; k >= 0; k--) {
      if (dots[k].pc > dots[k + 1].pc - minGapPc) {
        dots[k].pc = dots[k + 1].pc - minGapPc;
      }
    }
  }
  return dots;
}

/** The filtered-view legend line (mockup `#tlLegend`), or null when no chips
 *  are active. DIVERGENCE from the mockup, by design: the backend applies
 *  chips to the fetched set, so previously-fetched-now-filtered results (the
 *  mockup's dimmed dots and its "4 of 17" unfiltered total) don't exist
 *  client-side — the legend states the shown count + the active scope instead,
 *  e.g. "4 shown · app: Chrome". */
export function timelineLegend(
  shownCount: number,
  chipTexts: string[],
): string | null {
  if (chipTexts.length === 0) {
    return null;
  }
  return `${shownCount} shown · ${chipTexts.join(" · ")}`;
}
