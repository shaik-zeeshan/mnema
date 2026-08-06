// The Subjects list's one data owner: load the dossier, group it into subjects,
// keep the real per-conclusion confidence history behind each sparkline, and
// hold engine-driven reloads behind the refresh pill.
//
// Everything that decides *shape* (tiering, trend, summary counts, the staged
// refresh rules, ranking) is imported from the tested modules the old Insights
// Subjects surface already ships — `subjectsTiers.ts` and `subjectSearch.ts`.
// This class is only the fetch + assembly around them.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Activity,
  AiRuntimeStatus,
  Conclusion,
  SubjectView,
  UserContextStatus,
} from "$lib/types/recording";
import { humanizeError } from "$lib/format-error";
import {
  buildTiers,
  debounce,
  decideRefresh,
  deriveTrend,
  subjectsDiff,
  summaryCounts,
  type Axis,
  type TierSubject,
  type Trend,
} from "$lib/insights/subjectsTiers";
import { rankSubjects } from "$lib/insights/subjectSearch";

/** One polyline in a row's sparkline: a conclusion's confidence history. */
export interface SubjectSpark {
  points: number[];
  lead: boolean;
  faded: boolean;
}

/** A subject row. Satisfies `TierSubject`, so the tiering helpers group it
 *  directly with no separate projection. */
export interface SubjectRow extends TierSubject {
  subject: string;
  conclusions: Conclusion[];
  conclusionCount: number;
  pinned: boolean;
  faded: boolean;
  headline: string;
  lastMovedAtMs: number;
  trend: Trend;
  spark: SubjectSpark[];
  topConfidence: number;
}

/** A resolved piece of evidence under a subject — the inspector's "grounded in"
 *  rows. Only ever built from an Activity that actually resolved. */
export interface GroundingRow {
  activityId: number;
  source: "screen" | "audio";
  title: string;
  atMs: number;
  frameId: number | null;
  audioSegmentId: number | null;
}

/** A tier that already knows how many of its rows are revealed. */
const TIER_PAGE = 6;

export class SubjectsData {
  conclusions = $state<Conclusion[] | null>(null);
  loadError = $state<string | null>(null);
  loading = $state(true);
  /** null until the first status probe resolves — lets the empty state tell
   *  "engine off" apart from "engine on, nothing formed yet". */
  engineOn = $state<boolean | null>(null);

  /** subject → (conclusion id → oldest-first confidence points). */
  trajectories = $state<Map<string, Map<number, number[]>>>(new Map());
  /** Lazily resolved evidence Activities, keyed by id. */
  activities = $state<Map<number, Activity>>(new Map());
  private resolvedSubjects = new Set<string>();

  axis = $state<Axis>("conviction");
  query = $state("");
  appliedQuery = $state("");
  selected = $state<string | null>(null);
  tierShown = $state<Map<string, number>>(new Map());

  /** A newer dossier held behind the tool strip's "N views updated" pill. */
  staged = $state<Conclusion[] | null>(null);
  pendingCount = $state(0);

  private trajectoriesGen = 0;
  private applySearch = debounce((q: string) => {
    this.appliedQuery = q;
  }, 200);

  rows = $derived.by<SubjectRow[]>(() => {
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
        spark: sorted.map((c, i) => ({
          points: pointsFor(c, history?.get(c.id)),
          lead: i === 0,
          faded: c.status === "faded",
        })),
        topConfidence: top?.confidence ?? 0,
      });
    }
    return out;
  });

  /** One ordering feeds both the tiers and the search results: active first by
   *  confidence, faded sunk to the bottom. */
  displayRows = $derived.by<SubjectRow[]>(() =>
    [...this.rows].sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.topConfidence - a.topConfidence ||
        a.subject.localeCompare(b.subject),
    ),
  );

  tiers = $derived(
    buildTiers(this.displayRows, this.axis).filter((t) => t.items.length > 0),
  );

  summary = $derived(summaryCounts(this.displayRows));

  searching = $derived(this.appliedQuery.trim().length > 0);

  searchResults = $derived.by<SubjectRow[]>(() =>
    this.searching ? rankSubjects(this.displayRows, this.appliedQuery) : [],
  );

  selectedRow = $derived.by<SubjectRow | null>(
    () => this.displayRows.find((r) => r.subject === this.selected) ?? null,
  );

  /** The selected subject's resolved evidence, newest first — never a row for
   *  an Activity that did not resolve (G8: real or absent). */
  grounding = $derived.by<GroundingRow[]>(() => {
    const row = this.selectedRow;
    if (!row) return [];
    const seen = new Set<number>();
    const out: GroundingRow[] = [];
    for (const c of row.conclusions) {
      for (const e of c.evidence) {
        if (seen.has(e.activityId)) continue;
        seen.add(e.activityId);
        const activity = this.activities.get(e.activityId);
        if (!activity) continue;
        const ref = activity.evidence?.[0];
        out.push({
          activityId: activity.id,
          source: ref?.subjectType === "audio_segment" ? "audio" : "screen",
          title: activity.title,
          atMs: activity.startedAtMs,
          frameId: ref?.subjectType === "frame" ? ref.subjectId : null,
          audioSegmentId:
            ref?.subjectType === "audio_segment" ? ref.subjectId : null,
        });
      }
    }
    out.sort((a, b) => b.atMs - a.atMs);
    return out.slice(0, 4);
  });

  // ---- Search + paging -----------------------------------------------------

  search(q: string): void {
    this.query = q;
    this.applySearch(q);
  }

  shownCount(tierId: string): number {
    return this.tierShown.get(tierId) ?? TIER_PAGE;
  }

  showMore(tierId: string): void {
    const next = new Map(this.tierShown);
    next.set(tierId, this.shownCount(tierId) + TIER_PAGE);
    this.tierShown = next;
  }

  select(subject: string | null): void {
    this.selected = subject;
    if (subject) void this.resolveActivitiesFor(subject);
  }

  // ---- Loading -------------------------------------------------------------

  /** Mount: first read + the engine listener. Returns the teardown. */
  start(): () => void {
    void this.load();
    void this.probeEngine();

    const reload = debounce(() => void this.onContextChanged(), 500);
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
      this.applySearch.cancel();
    };
  }

  async load(): Promise<void> {
    this.loading = true;
    try {
      this.apply(await this.fetch());
    } finally {
      this.loading = false;
    }
  }

  applyStaged(): void {
    if (!this.staged) return;
    this.apply(this.staged);
    this.staged = null;
    this.pendingCount = 0;
  }

  private apply(list: Conclusion[]): void {
    this.conclusions = list;
    void this.loadTrajectories(list);
    if (this.selected) void this.resolveActivitiesFor(this.selected);
  }

  private async fetch(): Promise<Conclusion[]> {
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      this.loadError = null;
      return list;
    } catch (error) {
      // Only blank the surface when there is nothing to preserve. A background
      // refetch failure keeps the rendered rows.
      if (!this.conclusions?.length) this.loadError = humanizeError(error);
      return this.conclusions ?? [];
    }
  }

  private async probeEngine(): Promise<void> {
    const [ai, ctx] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
    ]);
    this.engineOn =
      Boolean(ai?.enabled && ai?.available) || Boolean(ctx?.engineAvailable);
  }

  /** The real per-conclusion confidence history, four subjects at a time. A
   *  failed fetch leaves that subject on its flat baseline. */
  private async loadTrajectories(list: Conclusion[]): Promise<void> {
    const gen = ++this.trajectoriesGen;
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
          // Best-effort — the subject keeps its baseline.
        }
      }
    };
    await Promise.all(
      Array.from({ length: Math.min(4, subjects.length) }, worker),
    );
    if (gen !== this.trajectoriesGen) return; // a newer load won
    this.trajectories = next;
  }

  /** Resolve the Activities a subject's conclusions cite, once per subject, via
   *  the same bounded paged scan the old surface uses. */
  private async resolveActivitiesFor(subject: string): Promise<void> {
    if (this.resolvedSubjects.has(subject)) return;
    const row = this.rows.find((r) => r.subject === subject);
    if (!row) return;
    this.resolvedSubjects.add(subject);

    const wanted = new Set<number>();
    for (const c of row.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
    if (wanted.size === 0) return;

    const resolved = new Map<number, Activity>();
    const PAGE = 200;
    for (let page = 0; page < 6; page++) {
      let batch: Activity[];
      try {
        batch = await invoke<Activity[]>("list_user_context_activities", {
          limit: PAGE,
          offset: page * PAGE,
        });
      } catch {
        break;
      }
      if (batch.length === 0) break;
      for (const a of batch) if (wanted.has(a.id)) resolved.set(a.id, a);
      if (resolved.size >= wanted.size || batch.length < PAGE) break;
    }
    if (resolved.size > 0) {
      const merged = new Map(this.activities);
      for (const [id, a] of resolved) merged.set(id, a);
      this.activities = merged;
    }
  }

  /** Engine-driven reload: diff it in display order, then apply, stage, or
   *  refresh the figures in place. Never reflows the list under the reader. */
  private async onContextChanged(): Promise<void> {
    const next = await this.fetch();
    const before = subjectOrder(this.conclusions ?? []);
    const after = subjectOrder(next);
    const diff = subjectsDiff(before, after);
    const action = decideRefresh({
      changed: diff.changed,
      expanded: this.selected !== null,
      atTop: true,
    });
    if (action === "stage") {
      this.staged = next;
      this.pendingCount = diff.count;
      return;
    }
    // "ignore" still applies: membership and order are unchanged, but the
    // FIGURES may have moved. Swapping the data reorders nothing.
    this.apply(next);
    this.staged = null;
    this.pendingCount = 0;
  }
}

/** A polyline needs two points to draw; one snapshot (or none) flattens into a
 *  baseline at that confidence rather than rendering an invisible line. */
function pointsFor(c: Conclusion, history: number[] | undefined): number[] {
  if (history && history.length >= 2) return history;
  if (history && history.length === 1) return [history[0], history[0]];
  return [c.confidence, c.confidence];
}

/** The display order of a raw conclusion list — the same sort `displayRows`
 *  applies, so a staged reload is diffed in the order the user sees. */
function subjectOrder(list: Conclusion[]): string[] {
  const groups = new Map<string, Conclusion[]>();
  for (const c of list) {
    const bucket = groups.get(c.subject);
    if (bucket) bucket.push(c);
    else groups.set(c.subject, [c]);
  }
  return [...groups.entries()]
    .map(([subject, cs]) => ({
      subject,
      faded: cs.every((c) => c.status === "faded"),
      top: Math.max(...cs.map((c) => c.confidence)),
    }))
    .sort(
      (a, b) =>
        Number(a.faded) - Number(b.faded) ||
        b.top - a.top ||
        a.subject.localeCompare(b.subject),
    )
    .map((s) => s.subject);
}
