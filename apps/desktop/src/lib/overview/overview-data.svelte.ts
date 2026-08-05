// Overview (direction 01 — Bento Native) — the one data load behind the bento.
//
// Every tile on the Overview reads from this single store so the surface makes
// ONE burst of reads on mount instead of nine independent ones, and so a tile
// with no data can tell "not loaded yet" from "loaded, and there is nothing" —
// the difference between a skeleton and a designed empty state (page 07).
//
// No command here is new: all nine are already registered in
// `src-tauri/src/lib.rs`. Each read is individually `catch`-guarded, so one
// failing command (a disabled Reasoning Engine, say) leaves the other eight
// tiles rendering real data rather than blanking the page.
//
// Empty is ALWAYS `[]` / `null`, never an unresolved promise: an unresolved
// `null` handed into the Svelte tree freezes rendering.
import { invoke } from "@tauri-apps/api/core";
import type { ConversationCluster, Moment } from "$lib/highlights";
import type { ConversationSummary } from "$lib/insights/conversation";
import type { DayCoverage } from "$lib/types/app-infra";
import type { SystemFacts } from "$lib/types/system-facts";
import type { Conclusion, UserContextDigest, UserContextStatus } from "$lib/types/recording";
import { localDayKey, localDayWindow } from "./overview-format";

/** How many frames the moments strip asks for. The strip scrolls; the extra
 *  frames past the fold are what make its last tile half-cut (the direction's
 *  horizontal-scroll signifier) rather than a decoration. */
const MOMENTS_LIMIT = 10;

export class OverviewData {
  /** False until the first burst settles — tiles show a skeleton until then. */
  loaded = $state(false);

  coverage = $state<DayCoverage[]>([]);
  moments = $state<Moment[]>([]);
  conversations = $state<ConversationCluster[]>([]);
  digest = $state<UserContextDigest | null>(null);
  contextStatus = $state<UserContextStatus | null>(null);
  conclusions = $state<Conclusion[]>([]);
  asks = $state<ConversationSummary[]>([]);
  facts = $state<SystemFacts | null>(null);

  /** The local day every day-scoped read above was taken for. */
  dayStartMs = $state(localDayWindow(new Date()).startMs);

  async load(now: Date = new Date()): Promise<void> {
    const { startMs, endMs } = localDayWindow(now);
    this.dayStartMs = startMs;

    const [coverage, moments, conversations, digest, status, conclusions, asks, facts] =
      await Promise.all([
        invoke<DayCoverage[]>("list_day_coverage").catch(() => []),
        invoke<Moment[]>("get_moments", {
          startMs,
          endMs,
          limit: MOMENTS_LIMIT,
        }).catch(() => []),
        invoke<ConversationCluster[]>("get_conversations", { startMs, endMs }).catch(() => []),
        invoke<UserContextDigest | null>("get_latest_user_context_digest").catch(() => null),
        invoke<UserContextStatus>("get_user_context_status").catch(() => null),
        invoke<Conclusion[]>("list_user_context_conclusions", {
          includeFaded: false,
        }).catch(() => []),
        invoke<ConversationSummary[]>("list_conversations", { limit: 4, offset: 0 }).catch(
          () => [],
        ),
        invoke<SystemFacts>("get_system_facts").catch(() => null),
      ]);

    this.coverage = coverage ?? [];
    this.moments = moments ?? [];
    this.conversations = conversations ?? [];
    this.digest = digest ?? null;
    this.contextStatus = status ?? null;
    this.conclusions = conclusions ?? [];
    this.asks = asks ?? [];
    this.facts = facts ?? null;
    this.loaded = true;
  }

  /** Wall-clock capture on the local day the load was taken for; null when that
   *  day is absent from coverage (i.e. holds no capture at all). */
  get todayCoveredMs(): number | null {
    const dayKey = localDayKey(new Date(this.dayStartMs));
    return this.coverage.find((d) => d.day === dayKey)?.coveredMs ?? null;
  }
}
