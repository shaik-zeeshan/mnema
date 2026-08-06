// subjects-index.svelte.ts — the Subjects index's data layer for direction 01.
// One class so the route component stays markup: it owns the two reads the
// shipping index already uses (`list_user_context_conclusions` +
// `get_user_context_subject` for the real per-conclusion Confidence History),
// the engine-status probe, and the `user_context_changed` refresh.
//
// Everything derived — tiers, trend, the summary counts, the search ranking —
// is delegated to the shared, unit-tested helpers in `$lib/insights`. Nothing
// here re-derives a threshold.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AiRuntimeStatus,
  Conclusion,
  SubjectView,
  UserContextStatus,
} from "$lib/types/recording";
import {
  type Trend,
  type TierSubject,
  debounce,
  deriveTrend,
} from "$lib/insights/subjectsTiers";
import { humanizeError } from "$lib/format-error";

/** One subject row. Satisfies `TierSubject`, so `buildTiers` groups it directly. */
export interface SubjectRow extends TierSubject {
  subject: string;
  conclusions: Conclusion[];
  conclusionCount: number;
  pinned: boolean;
  faded: boolean;
  headline: string;
  lastMovedAtMs: number;
  trend: Trend;
  /** One trajectory per conclusion (highest confidence first), oldest point
   *  first. Drives the sparkline: one polyline per conclusion. */
  tracks: { points: number[]; faded: boolean }[];
  topConfidence: number;
}

export class SubjectsIndexData {
  conclusions = $state<Conclusion[] | null>(null);
  /** subject → (conclusionId → oldest-first confidence points). */
  trajectories = $state<Map<string, Map<number, number[]>>>(new Map());
  loading = $state(true);
  loadError = $state<string | null>(null);
  /** null until the first probe resolves — the empty state waits for it. */
  engineOn = $state<boolean | null>(null);

  #gen = 0;

  readonly rows = $derived.by<SubjectRow[]>(() => {
    const list = this.conclusions;
    if (!list) return [];
    const groups = new Map<string, Conclusion[]>();
    for (const c of list) {
      const bucket = groups.get(c.subject);
      if (bucket) bucket.push(c);
      else groups.set(c.subject, [c]);
    }
    const out: SubjectRow[] = [];
    for (const [subject, cs] of groups) {
      const history = this.trajectories.get(subject);
      const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
      const top = sorted[0];
      out.push({
        subject,
        conclusions: sorted,
        conclusionCount: cs.length,
        pinned: cs.some((c) => c.pinned),
        faded: cs.every((c) => c.status === "faded"),
        headline: top?.statement ?? subject,
        lastMovedAtMs: cs.reduce(
          (acc, c) => Math.max(acc, c.updatedAtMs, c.lastSupportedAtMs),
          0,
        ),
        trend: deriveTrend(cs, history),
        // A conclusion with no stored history falls back to a flat baseline at
        // its current confidence — an honest "we have one reading", not a
        // fabricated arc.
        tracks: sorted.map((c) => ({
          points: history?.get(c.id) ?? [c.confidence],
          faded: c.status === "faded",
        })),
        topConfidence: top?.confidence ?? 0,
      });
    }
    // Non-faded by confidence desc, faded sunk — the one ordering both the flat
    // list and `buildTiers` start from.
    out.sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.topConfidence - a.topConfidence ||
        a.subject.localeCompare(b.subject),
    );
    return out;
  });

  async #fetchConclusions(): Promise<Conclusion[]> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      this.loadError = null;
      return list;
    } catch (error) {
      // A background refetch failure keeps the rendered rows; only a cold load
      // with nothing to preserve raises the error state.
      if (!this.conclusions?.length) {
        this.loadError = humanizeError(error);
      }
      return this.conclusions ?? [];
    }
  }

  /** Real per-conclusion Confidence History, bounded-concurrency, best-effort:
   *  a subject whose fetch fails keeps its flat baseline. A newer load wins. */
  async #loadTrajectories(list: Conclusion[]): Promise<void> {
    const gen = ++this.#gen;
    const subjects = [...new Set(list.map((c) => c.subject))];
    const next = new Map<string, Map<number, number[]>>();
    let cursor = 0;
    const worker = async (): Promise<void> => {
      while (cursor < subjects.length) {
        const subject = subjects[cursor++];
        try {
          const view = await invoke<SubjectView>("get_user_context_subject", {
            subject,
          });
          const byId = new Map<number, number[]>();
          for (const t of view.trajectories) {
            byId.set(
              t.conclusionId,
              t.history.map((h) => h.confidence),
            );
          }
          next.set(subject, byId);
        } catch {
          // Best-effort.
        }
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(4, subjects.length) }, worker),
    );
    if (gen !== this.#gen) return;
    this.trajectories = next;
  }

  async #loadEngineStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    this.engineOn =
      Boolean(ai?.enabled && ai?.available) || Boolean(ctx?.engineAvailable);
  }

  async load(): Promise<void> {
    this.loading = true;
    try {
      const list = await this.#fetchConclusions();
      this.conclusions = list;
      void this.#loadTrajectories(list);
    } finally {
      this.loading = false;
    }
    void this.#loadEngineStatus();
  }

  /** Mount hook: first load + a debounced reload on engine changes. Returns the
   *  teardown for the caller's `$effect`. */
  start(): () => void {
    void this.load();
    // `user_context_changed` fires per derivation pass; coalesce bursts. The
    // index is a read-only page, so a reload never yanks an edit out from under
    // anyone — no staging pill needed here.
    const reload = debounce(() => void this.load(), 500);
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => reload()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
      reload.cancel();
    };
  }
}
