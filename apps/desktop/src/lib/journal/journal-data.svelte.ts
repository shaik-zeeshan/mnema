// Journal (direction 01 — Bento Native) — the one data load behind the day.
//
// Nothing here is new plumbing: the reads, the day model and the presentation
// helpers are the shipping Journal's (`$lib/insights/journal-day.ts`,
// `journal-view.ts`, `lede-stats.ts`), lifted out of `DayTimeline.svelte` into a
// store so this direction's page can stay a layout. Rendering makes ZERO LLM
// calls — it arranges four already-cheap reads (activities + frames + status +
// digest); the only model call is the explicit "re-read" button.
//
// Empty is ALWAYS `[]` / `null`, never an unresolved promise.
import { invoke } from "@tauri-apps/api/core";
import { captureControls } from "$lib/capture-controls.svelte";
import { humanizeHours, windowFor } from "$lib/insights/activity-helpers";
import { buildJournalDay } from "$lib/insights/journal-day";
import { computeLedeStats } from "$lib/insights/lede-stats";
import type { FrameSummaryDto } from "$lib/types/app-infra";
import type {
  Activity,
  AiRuntimeStatus,
  UserContextDigest,
  UserContextStatus,
} from "$lib/types/recording";
import { buildBands } from "./bands";

/** A day step can cost a paid model call (a fresh range misses the digest
 *  cache) and a user may flick through days — so the digest fetch debounces. */
const DIGEST_DEBOUNCE_MS = 500;

export class JournalData {
  /** The viewed day's anchor; the stepper writes it, everything derives from it. */
  anchorMs = $state<number>(Date.now());

  activities = $state<Activity[]>([]);
  frames = $state<FrameSummaryDto[]>([]);
  aiStatus = $state<AiRuntimeStatus | null>(null);
  ctxStatus = $state<UserContextStatus | null>(null);
  usage = $state<{ timePerApp: { activeMs: number }[] } | null>(null);
  digest = $state<UserContextDigest | null>(null);

  statusLoaded = $state(false);
  dayLoaded = $state(false);
  usageLoaded = $state(false);
  digestLoading = $state(false);
  digestRegenerating = $state(false);
  digestError = $state<string | null>(null);

  range = $derived(windowFor(this.anchorMs, "day"));
  /** Stepping forward stops at the current day (mirrors Overview's `atLatest`). */
  atLatest = $derived(Date.now() < this.range.endMs);
  dayLabel = $derived(
    new Date(this.range.startMs).toLocaleDateString(undefined, {
      weekday: "short",
      month: "short",
      day: "numeric",
    }),
  );

  engineOn = $derived(
    Boolean(this.aiStatus?.enabled && this.aiStatus?.available) ||
      Boolean(this.ctxStatus?.engineAvailable),
  );

  model = $derived(
    buildJournalDay({
      activities: this.activities,
      frames: this.frames,
      coveredUntilMs: this.ctxStatus?.coveredUntilMs ?? null,
      recording: captureControls.isRunning,
      engineAvailable: Boolean(this.ctxStatus?.engineAvailable),
      engineReason: this.ctxStatus?.reason ?? null,
      dayStartMs: this.range.startMs,
      dayEndMs: this.range.endMs,
    }),
  );

  bands = $derived(buildBands(this.model.slots, this.model.gaps, this.model.pending));
  hasCards = $derived(this.model.slots.length > 0);

  // Activities scoped exactly as Overview scopes its range (overlap,
  // start-inclusive) so the header stats derive from the same set.
  private rangeActivities = $derived(
    this.activities.filter(
      (a) => a.startedAtMs < this.range.endMs && a.endedAtMs >= this.range.startMs,
    ),
  );

  ledeStats = $derived(
    computeLedeStats({
      timePerApp: this.usage?.timePerApp ?? [],
      rangeActivities: this.rangeActivities,
      rangeStartMs: this.range.startMs,
      rangeEndMs: this.range.endMs,
      engineOn: this.engineOn,
    }),
  );
  trackedLabel = $derived(humanizeHours(this.ledeStats.trackedMs));

  // The two empties are NOT interchangeable: "being written" when the day has
  // capture the engine hasn't reached, "nothing captured" when it has none.
  showNothingCaptured = $derived(this.dayLoaded && !this.hasCards && !this.model.hasAnyCapture);
  showBeingWritten = $derived(this.dayLoaded && !this.hasCards && this.model.hasAnyCapture);

  private rangeToken = 0;
  private usageToken = 0;
  private digestToken = 0;
  private regenSeq = 0;
  private digestTimer: ReturnType<typeof setTimeout> | null = null;

  async loadStatus(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    this.aiStatus = ai ?? null;
    this.ctxStatus = ctx ?? null;
    this.statusLoaded = true;
  }

  async loadDay(): Promise<void> {
    const token = ++this.rangeToken;
    const { startMs, endMs } = this.range;
    try {
      const [activities, frames] = await Promise.all([
        invoke<Activity[]>("list_user_context_activities", { startMs, endMs }),
        invoke<FrameSummaryDto[]>("list_frame_summaries_in_range", {
          request: {
            capturedAtStart: new Date(startMs).toISOString(),
            capturedAtEnd: new Date(endMs).toISOString(),
          },
        }),
      ]);
      if (token !== this.rangeToken) return; // the day moved on — stale
      this.activities = activities ?? [];
      this.frames = frames ?? [];
    } catch {
      // Best-effort: a failed read leaves the previous river standing.
    } finally {
      if (token === this.rangeToken) this.dayLoaded = true;
    }
  }

  async loadUsage(): Promise<void> {
    const token = ++this.usageToken;
    const { startMs, endMs } = this.range;
    try {
      const next = await invoke<{ timePerApp: { activeMs: number }[] }>("get_usage_charts", {
        startMs,
        endMs,
      });
      if (token !== this.usageToken) return;
      this.usage = next ?? null;
    } catch {
      if (token === this.usageToken) this.usage = null;
    } finally {
      if (token === this.usageToken) this.usageLoaded = true;
    }
  }

  async loadDigest(): Promise<void> {
    if (!this.statusLoaded || !this.engineOn) {
      this.digest = null;
      this.digestLoading = false;
      return;
    }
    const token = ++this.digestToken;
    this.digestLoading = true;
    this.digestError = null;
    const { startMs, endMs } = this.range;
    try {
      const next = await invoke<UserContextDigest | null>("get_user_context_digest", {
        rangeKind: "day",
        startMs,
        endMs,
      });
      if (token !== this.digestToken) return;
      this.digest = next ?? null;
    } catch {
      if (token === this.digestToken) this.digest = null;
    } finally {
      if (token === this.digestToken) this.digestLoading = false;
    }
  }

  /** The one model call on this surface, and only on an explicit click. */
  async regenerateDigest(): Promise<void> {
    if (!this.engineOn || this.digestRegenerating) return;
    // The busy flag gets its own sequence: `digestToken` is shared with
    // `loadDigest`, which the `user_context_changed` listener fires on every
    // worker beat — a token-gated reset would leave the button stuck on
    // "reading…". Result writes stay token-gated so a newer load still wins.
    const token = ++this.digestToken;
    const regen = ++this.regenSeq;
    this.digestRegenerating = true;
    this.digestLoading = false;
    this.digestError = null;
    const { startMs, endMs } = this.range;
    try {
      const next = await invoke<UserContextDigest | null>("regenerate_user_context_digest", {
        rangeKind: "day",
        startMs,
        endMs,
      });
      if (token !== this.digestToken) return;
      this.digest = next ?? null;
      if (!next) this.digestError = "Not enough activity in this day to write a read.";
    } catch (error) {
      if (token === this.digestToken) {
        this.digestError = error instanceof Error ? error.message : "Couldn't write a read.";
      }
    } finally {
      if (regen === this.regenSeq) this.digestRegenerating = false;
    }
  }

  /** Mount / live-refresh: everything, digest included, undebounced. */
  async reloadAll(): Promise<void> {
    await this.loadStatus();
    await Promise.all([this.loadDay(), this.loadUsage(), this.loadDigest()]);
  }

  /** A day step: re-read the day now, and the digest after the debounce. */
  loadForNewDay(): void {
    this.dayLoaded = false;
    this.usageLoaded = false;
    void this.loadDay();
    void this.loadUsage();
    this.digestToken += 1; // invalidate any in-flight/queued load for the old day
    this.digest = null;
    this.digestRegenerating = false;
    this.digestError = null;
    this.cancelDigestDebounce();
    if (!this.statusLoaded || !this.engineOn) {
      this.digestLoading = false;
      return;
    }
    this.digestLoading = true; // the placeholder spans the debounce window too
    this.digestTimer = setTimeout(() => {
      this.digestTimer = null;
      void this.loadDigest();
    }, DIGEST_DEBOUNCE_MS);
  }

  cancelDigestDebounce(): void {
    if (this.digestTimer != null) clearTimeout(this.digestTimer);
    this.digestTimer = null;
  }
}
