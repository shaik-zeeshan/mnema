// Chat scope (CH3): the time range + "About you" switch a chat sends with every
// turn. Held in the chat view as local ISO days, never persisted; the backend
// clamps the data tools' windows into it (src-tauri/src/ask_ai/scope.rs).
import { addDays, monday, parseIso, presets, spanLabel, toIso } from "$lib/components/date-input";
import type { AskAiScope } from "$lib/insights/conversation";

export interface ChatScope {
  /** Local ISO days, inclusive. */
  start: string;
  end: string;
  aboutYou: boolean;
}

/** Where a turn goes, for the panel's "Goes to" row. */
export interface GoesTo {
  /** Provider display name ("Anthropic"); null before settings load. */
  name: string | null;
  where: "cloud" | "local" | null;
  /** "your API key" · "your ChatGPT sign-in" · "localhost:11434" … */
  via: string;
  webFetch: boolean;
  connectors: number;
}

const ISO_DAY = /^\d{4}-\d{2}-\d{2}$/;

/** The default scope: this week (Mon–Sun), About you on (chat.html). */
export function defaultScope(today: Date = new Date()): ChatScope {
  const m0 = monday(parseIso(toIso(today)));
  return { start: toIso(m0), end: toIso(addDays(m0, 6)), aboutYou: true };
}

/** `?from=&to=` (local ISO days) → a scope, keeping About you; null when invalid. */
export function scopeFromParams(from: string | null, to: string | null, aboutYou: boolean): ChatScope | null {
  if (!from || !ISO_DAY.test(from)) return null;
  const end = to && ISO_DAY.test(to) ? to : from;
  return from <= end ? { start: from, end, aboutYou } : null;
}

/** "this week" / "yesterday" for a preset, else the span ("Oct 3 – 9"). */
export function scopeLabel(scope: ChatScope, today: Date = new Date()): string {
  const [a, b] = [parseIso(scope.start), parseIso(scope.end)];
  const preset = presets(parseIso(toIso(today))).find(([, s, e]) => +s === +a && +e === +b);
  return preset ? preset[0].toLowerCase() : spanLabel(a, b);
}

/** The wire scope: local midnight of `start` through the last ms of `end`. */
export function toWireScope(scope: ChatScope): AskAiScope {
  const day = (iso: string, k: number) => {
    const d = parseIso(iso);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate() + k).getTime();
  };
  return { fromMs: day(scope.start, 0), toMs: day(scope.end, 1) - 1, aboutYou: scope.aboutYou };
}
