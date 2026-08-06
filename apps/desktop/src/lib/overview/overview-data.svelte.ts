// The Overview bento's one reader, and the inspector's one selection.
//
// Every tile on this surface is a headline over a read that already exists —
// there is no Overview aggregate command and this file does not invent one. The
// day-scoped reads (`get_moments`, `get_conversations`) re-run when the day
// changes; everything else is loaded once per mount.
//
// Round-4 decision **G8** governs the failure shape: a read that fails leaves
// its slice `null` and its tile renders a quiet reason, never a zero and never
// a placeholder number. `systemFacts` is reached through the app-global
// singleton (the status strip already warms it — `ensureLoaded` is idempotent).

import { invoke } from "@tauri-apps/api/core";
import type { ConversationCluster, Moment } from "$lib/highlights";
import type { ConversationSummary } from "$lib/insights/conversation";
import type { DayCoverage } from "$lib/types/app-infra";
import type {
  AuthoredContext,
  Conclusion,
  UserContextDigest,
  UserContextStatus,
} from "$lib/types/recording";
import { dayKeyOf, dayWindow } from "./overview-format";

/** One key/value line in the inspector. */
export interface InspectorRow {
  k: string;
  v: string;
  mono?: boolean;
}

export interface InspectorSection {
  label: string;
  rows: InspectorRow[];
}

/**
 * What the inspector shows. Each tile builds this for its own row — the panel
 * stays dumb, which is what lets a new tile arrive without touching it.
 */
export interface Selection {
  /** Stable within a tile; drives the `.ss-row--sel` comparison. */
  key: string;
  /** The tile the row came from, echoed in the inspector's Selection header. */
  source: string;
  title: string;
  /** Optional one-line lede under the title (a statement, a question). */
  lede?: string;
  sections: InspectorSection[];
}

/** A load that has not finished yet is neither empty nor failed. */
export type LoadState<T> = { status: "loading" } | { status: "ok"; value: T } | { status: "failed" };

function ok<T>(value: T): LoadState<T> {
  return { status: "ok", value };
}

async function read<T>(run: () => Promise<T>): Promise<LoadState<T>> {
  try {
    return ok(await run());
  } catch {
    return { status: "failed" };
  }
}

export class OverviewData {
  /** The local day every day-scoped tile is about. */
  dayKey = $state(dayKeyOf(new Date()));

  moments = $state<LoadState<Moment[]>>({ status: "loading" });
  conversations = $state<LoadState<ConversationCluster[]>>({ status: "loading" });
  coverage = $state<LoadState<DayCoverage[]>>({ status: "loading" });
  digest = $state<LoadState<UserContextDigest | null>>({ status: "loading" });
  conclusions = $state<LoadState<Conclusion[]>>({ status: "loading" });
  contextStatus = $state<LoadState<UserContextStatus | null>>({ status: "loading" });
  /** What the user WROTE. The Context tile counts these; inferred beliefs are
   *  the Subjects tile's business, because they are a different thing. */
  authored = $state<LoadState<AuthoredContext[]>>({ status: "loading" });
  asks = $state<LoadState<ConversationSummary[]>>({ status: "loading" });

  selection = $state<Selection | null>(null);

  // Drops a stale response when the day is stepped faster than the reads land.
  #dayGeneration = 0;

  select(next: Selection): void {
    // Clicking the selected row again clears it — the inspector's empty state is
    // reachable without hunting for a close button.
    this.selection = this.selection?.key === next.key ? null : next;
  }

  setDay(key: string): void {
    if (key === this.dayKey) return;
    this.dayKey = key;
    this.selection = null;
    void this.loadDay();
  }

  /** Reads that answer "what about this day?". */
  async loadDay(): Promise<void> {
    const generation = ++this.#dayGeneration;
    const { startMs, endMs } = dayWindow(this.dayKey);
    this.moments = { status: "loading" };
    this.conversations = { status: "loading" };

    const [moments, conversations] = await Promise.all([
      read(() => invoke<Moment[]>("get_moments", { startMs, endMs, limit: 5 })),
      read(() => invoke<ConversationCluster[]>("get_conversations", { startMs, endMs })),
    ]);
    if (generation !== this.#dayGeneration) return;
    this.moments = moments;
    this.conversations = conversations;
  }

  /** Reads that are the same whatever day is showing. */
  async loadStanding(): Promise<void> {
    const [coverage, digest, conclusions, contextStatus, asks, authored] = await Promise.all([
      read(() => invoke<DayCoverage[]>("list_day_coverage")),
      // Read-only by construction: the day digest tile must never start a
      // generation just because it mounted (G11 — Open Threads v1 is prose).
      read(() => invoke<UserContextDigest | null>("get_latest_user_context_digest")),
      read(() => invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false })),
      read(() => invoke<UserContextStatus | null>("get_user_context_status")),
      read(() => invoke<ConversationSummary[]>("list_conversations", { limit: 6, offset: 0 })),
      read(() => invoke<AuthoredContext[]>("list_user_context_authored")),
    ]);
    this.coverage = coverage;
    this.digest = digest;
    this.conclusions = conclusions;
    this.contextStatus = contextStatus;
    this.asks = asks;
    this.authored = authored;
  }
}
