// Context (page 10) — the one read pass plus the copy this surface has to get
// exactly right.
//
// Two kinds of knowing sit on this page: the standing statements you write
// (`user_context_authored`, never fades) and what the engine worked out
// (activities / conclusions / subjects, counted — never narrated). Every number
// here is a field that already exists on `get_user_context_status`; nothing is
// derived from a made-up entity (G8, and the reason page 01's "142 facts" is
// corrected — the backend has no `fact`).

import { invoke } from "@tauri-apps/api/core";

import { humanizeError } from "$lib/format-error";
import type {
  AuthoredContext,
  Conclusion,
  DismissedView,
  UserContextStatus,
} from "$lib/types/recording";

/** One pass over every read this surface makes. Failures are per-slice: a dead
 *  status command must not blank the statements you wrote. */
export interface ContextSnapshot {
  statements: AuthoredContext[];
  status: UserContextStatus | null;
  conclusions: Conclusion[];
  dismissed: DismissedView[];
  /** First failure across the four reads, already humanized. */
  error: string | null;
}

export const EMPTY_SNAPSHOT: ContextSnapshot = {
  statements: [],
  status: null,
  conclusions: [],
  dismissed: [],
  error: null,
};

/** Composer prefills. They only seed the textarea — topic is free text with no
 *  vocabulary behind it, so these are labelled `prefill:`, never categories. */
export const PREFILLS: { label: string; prompt: string }[] = [
  { label: "Your role", prompt: "I'm a … " },
  { label: "What you're working on", prompt: "I'm currently working on … " },
  { label: "How you work", prompt: "I prefer to work by … " },
  { label: "What you care about", prompt: "I care deeply about … " },
  { label: "Goals this quarter", prompt: "Goal: " },
];

async function slice<T>(load: Promise<T>, fallback: T): Promise<[T, string | null]> {
  try {
    return [await load, null];
  } catch (error) {
    return [fallback, humanizeError(error)];
  }
}

export async function loadContext(): Promise<ContextSnapshot> {
  const [[statements, e1], [status, e2], [conclusions, e3], [dismissed, e4]] = await Promise.all([
    slice(invoke<AuthoredContext[]>("list_user_context_authored"), [] as AuthoredContext[]),
    slice(invoke<UserContextStatus | null>("get_user_context_status"), null),
    slice(
      invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }),
      [] as Conclusion[],
    ),
    slice(invoke<DismissedView[]>("user_context_list_dismissed"), [] as DismissedView[]),
  ]);
  return { statements, status, conclusions, dismissed, error: e1 ?? e2 ?? e3 ?? e4 };
}

/** "12d ago" — coarse, never minute-precise beyond the first hour (G8). */
export function relativeAge(ms: number, now: number = Date.now()): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const diff = now - ms;
  if (diff < 60_000) return "just now";
  const min = Math.floor(diff / 60_000);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  // Days run to a month: a standing statement's age is read as "how long have I
  // believed this", and "3w ago" is a worse answer to that than "23d ago".
  if (day < 30) return `${day}d ago`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo ago`;
  return `${Math.floor(day / 365)}y ago`;
}

/** "added 12d ago" / "edited 3d ago" — an edit is an update meaningfully after
 *  creation (the row is stamped twice within a second on insert). */
export function statementStamp(s: AuthoredContext, now: number = Date.now()): string {
  return s.updatedAtMs > s.createdAtMs + 1000
    ? `edited ${relativeAge(s.updatedAtMs, now)}`
    : `added ${relativeAge(s.createdAtMs, now)}`;
}

/** "≈ 1.9M" — the token line's magnitude. Estimated counts deserve a rounded
 *  presentation; `estimate_tokens` is chars/4, never a billed number. */
export function compactCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

export function plural(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

/**
 * The wipe confirmation's message. Every line is checked against
 * `wipe_user_context` (apps/desktop/src-tauri/src/user_context/commands.rs) and
 * `UserContextStore::wipe_all`:
 *
 *   1. AI features are switched OFF first (`update_ai_runtime_settings`).
 *   2. `wipe_all` DELETEs activities + their evidence, conclusions + evidence +
 *      confidence history, subject vectors, dismissals, derivation runs, the
 *      authored (standing) statements, and digests.
 *   3. `conversation().wipe_all()` clears every persistent Quick Access / Chat
 *      conversation.
 *
 * The shipped settings confirmation names only activities, conclusions and
 * dismissals — it under-states two whole categories it destroys, which is the
 * bug this copy fixes. Raw captures and every setting are untouched.
 */
export function wipeMessage(status: UserContextStatus | null, standingCount: number): string {
  const activities = status?.activityCount ?? 0;
  const conclusions = status?.conclusionCount ?? 0;
  const subjects = status?.subjectCount ?? 0;
  const dismissed = status?.dismissedCount ?? 0;
  return [
    "This clears the derived understanding and turns AI features off. It cannot be undone.",
    "",
    "CLEARED",
    `• ${plural(activities, "activity", "activities")} and ${plural(
      conclusions,
      "conclusion",
      "conclusions",
    )} across ${plural(subjects, "subject", "subjects")}`,
    "• All confidence history, subject vectors and digests",
    `• Your ${plural(dismissed, "dismissal", "dismissals")} — the vetoes go too, so those beliefs can form again`,
    `• Your ${plural(standingCount, "standing statement", "standing statements")}`,
    "• All Quick Access and Chat ask history",
    "",
    "KEPT",
    "• Every recording, frame and audio segment on disk",
    "• All capture, privacy and retention settings",
  ].join("\n");
}
