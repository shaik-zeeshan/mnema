// Presentation helpers for the Journal (DayTimeline + DaySpine + JournalRiver).
// Pure + bun-testable so the river ordering, spine geometry and side-column
// tallies stay out of the .svelte components and have a runnable check.

import type { Activity, ActivityCategory } from "$lib/types/recording";
import type { JournalCardSlot, JournalGap, Span } from "./journal-day";
import { humanizeMs } from "./activity-helpers";

/** One row of the river — an activity card, an away-gap, or a failed window. */
export type RiverRow =
  | ({ kind: "card"; atMs: number } & JournalCard)
  | { kind: "gap"; gap: JournalGap; atMs: number }
  /** A window whose summarizing failed past the retry cap (real bounds). */
  | { kind: "failed"; span: Span; atMs: number };

/** Merge activity cards, away-gaps and failed windows into one river, newest
 *  first. Same start: the shorter card sits above the longer one. */
export function buildRiver(
  cards: JournalCard[],
  gaps: JournalGap[],
  failed: Span[] = [],
): RiverRow[] {
  const rows: RiverRow[] = [
    ...cards.map((card): RiverRow => ({ kind: "card", ...card, atMs: card.startMs })),
    ...gaps.map((gap): RiverRow => ({ kind: "gap", gap, atMs: gap.startMs })),
    ...failed.map((span): RiverRow => ({ kind: "failed", span, atMs: span.startMs })),
  ];
  const len = (r: RiverRow) => (r.kind === "card" ? r.endMs - r.atMs : 0);
  rows.sort((a, b) => b.atMs - a.atMs || len(a) - len(b));
  return rows;
}

// ── Folding short and overlapping activities (display only) ────────────
/** Shown for less than this, an activity is "short": it folds into a neighbour. */
export const SHORT_ACTIVITY_MS = 120_000;
/** Backed by fewer screen frames than this, an activity is "short" too. */
export const SHORT_ACTIVITY_MIN_FRAMES = 3;
/** A short activity folds only into a card at most this far away. */
export const FOLD_NEIGHBOUR_MS = 300_000;

/** An activity shown as one line inside a neighbouring card. */
export interface SubLine {
  slot: JournalCardSlot;
  startMs: number;
  endMs: number;
}

/** One card as shown: its span after overlap clipping, the short activities
 *  folded into it, and `slim` = short with no neighbour (a one-line row). */
export interface JournalCard extends SubLine {
  subs: SubLine[];
  slim: boolean;
}

/**
 * The day's cards as shown. Nothing is dropped: an activity that overlaps an
 * earlier one starts where that one ends (or, with nothing left, becomes its
 * sub-line); a short one (`SHORT_ACTIVITY_MS` / `SHORT_ACTIVITY_MIN_FRAMES`)
 * becomes a sub-line of the full card before or after it within
 * `FOLD_NEIGHBOUR_MS` — same category first, then the closer, then the longer —
 * or, with none near, a slim row. Returned chronological.
 */
export function foldCards(slots: JournalCardSlot[]): JournalCard[] {
  const cards: JournalCard[] = [];
  let reach: JournalCard | null = null; // the card shown furthest to the right
  // Same start: the longer one keeps its span, the shorter is the "later" one.
  const order = [...slots].sort(
    (a, b) => a.activity.startedAtMs - b.activity.startedAtMs || b.activity.endedAtMs - a.activity.endedAtMs,
  );
  for (const slot of order) {
    const { startedAtMs, endedAtMs } = slot.activity;
    const startMs = reach ? Math.max(startedAtMs, reach.endMs) : startedAtMs;
    if (reach && startedAtMs < reach.endMs && startMs >= endedAtMs) {
      reach.subs.push({ slot, startMs: startedAtMs, endMs: endedAtMs });
      continue;
    }
    const card: JournalCard = { slot, startMs, endMs: Math.max(startMs, endedAtMs), subs: [], slim: false };
    cards.push(card);
    if (!reach || card.endMs >= reach.endMs) reach = card;
  }

  const short = (c: JournalCard) =>
    c.endMs - c.startMs < SHORT_ACTIVITY_MS ||
    (c.slot.media === "frames" && c.slot.frameCount < SHORT_ACTIVITY_MIN_FRAMES);
  const full = cards.filter((c) => !short(c));
  const len = (c: JournalCard) => c.endMs - c.startMs;
  const away = (c: JournalCard, h: JournalCard) => Math.max(0, c.startMs - h.endMs, h.startMs - c.endMs);
  for (const c of cards) {
    if (!short(c)) continue;
    const category = c.slot.activity.category ?? null;
    const same = (h: JournalCard) => category !== null && h.slot.activity.category === category;
    const next = full.findIndex((h) => h.startMs > c.startMs);
    const host = [full[(next < 0 ? full.length : next) - 1], full[next]]
      .filter((h) => h && away(c, h) <= FOLD_NEIGHBOUR_MS)
      .sort((x, y) => Number(same(y)) - Number(same(x)) || away(c, x) - away(c, y) || len(y) - len(x))[0];
    if (!host) {
      c.slim = true;
      continue;
    }
    host.subs.push({ slot: c.slot, startMs: c.startMs, endMs: c.endMs }, ...c.subs);
    c.subs = [];
  }
  const shown = cards.filter((c) => c.slim || !short(c));
  for (const c of shown) c.subs.sort((a, b) => a.startMs - b.startMs);
  return shown;
}

/**
 * Unique `{#each}` key for a river row. Card rows key off the Activity's
 * database id — NEVER its start time: two Activities can share a `startedAtMs`
 * (e.g. derivation splitting windows across a stop/restart-heavy day), and a
 * `kind + atMs` key then collides, throwing Svelte's `each_key_duplicate`
 * mid-flush and leaving the river stuck on the loading skeleton.
 */
export function riverRowKey(row: RiverRow): string {
  return row.kind === "card" ? `card${row.slot.activity.id}` : `${row.kind}${row.atMs}`;
}

/** "Away · 1h 10m" — gaps carry no label, only their length. */
export function awayLabel(span: Span): string {
  return `Away · ${humanizeMs(span.endMs - span.startMs)}`;
}

/** The shown cards as slots over their displayed spans — what the spine draws
 *  and the Day mix tallies, so overlapping time is never counted twice. */
export function shownSlots(cards: JournalCard[]): JournalCardSlot[] {
  return cards.map((c) => ({ ...c.slot, activity: { ...c.slot.activity, startedAtMs: c.startMs, endedAtMs: c.endMs } }));
}

// ── Day spine ───────────────────────────────────────────────────────────
const HOUR = 3_600_000;

export interface SpineAxis {
  startMs: number;
  endMs: number;
  /** Gridline instants, one every 1h / 2h / 3h depending on the span. */
  ticks: number[];
}

/** The spine's axis: the real first→last span, padded 10 min and rounded out
 *  to whole (local) hours, so a past day shows its own hours, not 08–20. */
export function spineAxis(firstMs: number, lastMs: number): SpineAxis {
  const pad = 600_000;
  const startMs = floorHour(firstMs - pad);
  const endMs = Math.max(startMs + HOUR, ceilHour(lastMs + pad));
  const len = endMs - startMs;
  const step = len <= 6 * HOUR ? HOUR : len <= 12 * HOUR ? 2 * HOUR : 3 * HOUR;
  const ticks: number[] = [];
  for (let t = startMs; t <= endMs; t += step) ticks.push(t);
  return { startMs, endMs, ticks };
}
function floorHour(ms: number): number {
  const d = new Date(ms);
  d.setMinutes(0, 0, 0);
  return d.getTime();
}
function ceilHour(ms: number): number {
  const f = floorHour(ms);
  return f === ms ? ms : f + HOUR;
}

/** Overlapping / same-start spans stacked greedily into sub-lanes: longest
 *  first on a tie, each into the first lane that's free by its start. */
export function stackLanes(spans: Span[]): { lane: number[]; lanes: number } {
  const order = spans
    .map((_, i) => i)
    .sort((x, y) => spans[x].startMs - spans[y].startMs || spans[y].endMs - spans[x].endMs);
  const laneEnd: number[] = [];
  const lane: number[] = new Array(spans.length);
  for (const i of order) {
    let l = laneEnd.findIndex((e) => e <= spans[i].startMs);
    if (l < 0) l = laneEnd.length;
    laneEnd[l] = spans[i].endMs;
    lane[i] = l;
  }
  return { lane, lanes: Math.max(1, laneEnd.length) };
}

// ── Side column ─────────────────────────────────────────────────────────
/** Activity time per category (null = uncategorized), largest first. */
export function categoryMix(activities: Activity[]): { category: ActivityCategory | null; ms: number }[] {
  const totals = new Map<ActivityCategory | null, number>();
  for (const a of activities) {
    const ms = Math.max(0, a.endedAtMs - a.startedAtMs);
    if (ms > 0) totals.set(a.category ?? null, (totals.get(a.category ?? null) ?? 0) + ms);
  }
  return [...totals].map(([category, ms]) => ({ category, ms })).sort((a, b) => b.ms - a.ms);
}

export interface WeekDay {
  startMs: number;
  /** Activity time by category, largest first (empty when nothing was written up). */
  mix: { category: ActivityCategory | null; ms: number }[];
  activityMs: number;
  activities: number;
  /** Capture-only fallback: hours with any capture (from the usage heatmap). */
  capturedHours: number;
  future: boolean;
}

/** The Mon–Sun week holding `dayStartMs`: per-day activity mix, plus captured
 *  hours for days with nothing written up. `capturedHours` maps local day-start
 *  ms → hours with capture. */
export function weekDays(
  dayStartMs: number,
  activities: Activity[],
  capturedHours: Record<number, number>,
  nowMs: number,
): WeekDay[] {
  const d = new Date(dayStartMs);
  d.setDate(d.getDate() - ((d.getDay() + 6) % 7));
  const out: WeekDay[] = [];
  for (let i = 0; i < 7; i++) {
    const startMs = d.getTime();
    d.setDate(d.getDate() + 1);
    const endMs = d.getTime();
    const own = activities.filter((a) => a.startedAtMs >= startMs && a.startedAtMs < endMs);
    // Count and tally cards as the Journal shows them (no frame counts here,
    // so only the duration and overlap rules fold).
    const cards = foldCards(own.map((activity) => ({ activity, frameCount: 0, media: "none" as const })));
    const mix = categoryMix(shownSlots(cards).map((s) => s.activity));
    out.push({
      startMs,
      mix,
      activityMs: mix.reduce((t, m) => t + m.ms, 0),
      activities: cards.length,
      capturedHours: capturedHours[startMs] ?? 0,
      future: startMs > nowMs,
    });
  }
  return out;
}

/** Hours with any capture per local day, from `get_usage_charts`' hourly heatmap.
 *  ponytail: hour granularity — a bucket with one frame counts a full hour; use
 *  per-day active ms if the capture-only week bars ever read wrong. */
export function capturedHoursByDay(buckets: { bucketStartMs: number; intensityCount: number }[]): Record<number, number> {
  const out: Record<number, number> = {};
  for (const b of buckets) {
    if (b.intensityCount <= 0) continue;
    const d = new Date(b.bucketStartMs);
    d.setHours(0, 0, 0, 0);
    out[d.getTime()] = (out[d.getTime()] ?? 0) + 1;
  }
  return out;
}

/**
 * Human copy for the pending slot when the engine is unavailable. Codes mirror
 * `AiRuntimeStatus.reason` / `UserContextStatus.reason` (see
 * `settings/state/ai-runtime.svelte.ts`'s `aiRuntimeReasonLabel`), reshaped into
 * "Summaries are paused — <why>" sentences with a safe generic default.
 */
export function pendingReasonCopy(reason: string): string {
  if (reason.startsWith("no_provider_key:")) {
    return "Summaries are paused — no API key is saved for the engine's provider.";
  }
  if (reason.startsWith("provider_not_connected:")) {
    return "Summaries are paused — the engine's default provider isn't connected.";
  }
  if (reason.startsWith("needs_reconnect:")) {
    return "Summaries are paused — sign in with ChatGPT again in Settings.";
  }
  if (reason.startsWith("provider_unreachable:")) {
    // Deliberately NOT the sign-in sentence: the login is fine, the network
    // isn't. Telling this user to sign in again invites them to disconnect a
    // working account.
    return "Summaries are paused — can't reach ChatGPT right now. They'll resume on their own.";
  }
  switch (reason) {
    case "user_context_disabled":
      return "Summaries are paused — continuous derivation is turned off.";
    case "ai_runtime_disabled":
      return "Summaries are paused — AI features are turned off.";
    case "no_providers":
      return "Summaries are paused — no AI provider is connected yet.";
    case "no_default_model":
      return "Summaries are paused — no default model is chosen.";
    case "no_base_url":
      return "Summaries are paused — the local provider needs a base URL.";
    case "local_endpoint_unreachable":
      return "Summaries are paused — the local engine can't be reached.";
    default:
      return "Summaries are paused — the Reasoning Engine isn't available right now.";
  }
}

/** Which panel the river shows when there's nothing to draw on the spine. */
export type JournalEmpty =
  /** Today, nothing captured, not recording (or user-paused). */
  | { kind: "idle"; paused: boolean }
  | { kind: "nothing" }
  /** Nothing left: retention removed this day's capture. */
  | { kind: "retention"; days: number }
  /** Captured, but older than the window the worker summarizes on its own. */
  | { kind: "older"; days: number; footageKept: boolean }
  /** Captured, history backfill hasn't reached it yet. */
  | { kind: "queued" }
  /** Captured and covered, but no card came out of it. */
  | { kind: "unsummarized" };
