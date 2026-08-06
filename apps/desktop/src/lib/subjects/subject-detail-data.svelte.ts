// One opened subject: its conclusions, their trajectories, the selected
// belief's story spine, and the two real actions (pin, dismiss).
//
// The ordering (`sortConclusions`) and the story spine (`buildTimeline`) come
// from the tested `subjectTimeline.ts` the old Subject-detail surface ships —
// this class only fetches and holds selection.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";
import type {
  Activity,
  Conclusion,
  SubjectTrajectory,
  SubjectView,
} from "$lib/types/recording";
import { humanizeError } from "$lib/format-error";
import { toast } from "$lib/toast.svelte";
import { openEvidenceRef } from "./open-evidence";
import { debounce } from "$lib/insights/subjectsTiers";
import {
  buildTimeline,
  sortConclusions,
  type ConclusionSort,
} from "$lib/insights/subjectTimeline";

export class SubjectDetailData {
  readonly subject: string;

  view = $state<SubjectView | null>(null);
  loadError = $state<string | null>(null);
  loading = $state(true);
  selectedId = $state<number | null>(null);
  sort = $state<ConclusionSort>("confidence");
  /** In-flight pin/dismiss guard; `actionKind` says which button is busy. */
  actionId = $state<number | null>(null);
  actionKind = $state<"pin" | "dismiss" | null>(null);
  activities = $state<Map<number, Activity>>(new Map());

  constructor(subject: string) {
    this.subject = subject;
  }

  trajectoryById = $derived.by<Map<number, SubjectTrajectory>>(() => {
    const m = new Map<number, SubjectTrajectory>();
    for (const t of this.view?.trajectories ?? []) m.set(t.conclusionId, t);
    return m;
  });

  ordered = $derived(
    sortConclusions(this.view?.conclusions ?? [], this.trajectoryById, this.sort),
  );

  selected = $derived.by<Conclusion | null>(
    () => this.ordered.find((c) => c.id === this.selectedId) ?? null,
  );

  selectedTrajectory = $derived(
    this.selectedId === null ? undefined : this.trajectoryById.get(this.selectedId),
  );

  /** 1-based position of the selection in the strip — the inspector's
   *  "Conclusion 1 of 6". */
  selectedIndex = $derived(this.ordered.findIndex((c) => c.id === this.selectedId));

  events = $derived(
    this.selected
      ? buildTimeline(this.selected, this.selectedTrajectory, this.activities)
      : [],
  );

  conclusionCount = $derived(this.view?.conclusions.length ?? 0);

  fadedCount = $derived(
    this.view?.conclusions.filter((c) => c.status === "faded").length ?? 0,
  );

  linkedActivityCount = $derived.by<number>(() => {
    const ids = new Set<number>();
    for (const c of this.view?.conclusions ?? [])
      for (const e of c.evidence) ids.add(e.activityId);
    return ids.size;
  });

  /** Evidence tallies for the selected belief — counted, never estimated. */
  evidenceCounts = $derived.by(() => {
    const c = this.selected;
    let supports = 0;
    let contradicts = 0;
    for (const e of c?.evidence ?? []) {
      if (e.stance === "contradict") contradicts += 1;
      else supports += 1;
    }
    return { supports, contradicts };
  });

  snapshotCount(id: number): number {
    return this.trajectoryById.get(id)?.history.length ?? 0;
  }

  historyOf(id: number): number[] {
    return (this.trajectoryById.get(id)?.history ?? []).map((h) => h.confidence);
  }

  start(): () => void {
    void this.load();
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

  async load(): Promise<void> {
    this.loading = true;
    try {
      const next = await invoke<SubjectView>("get_user_context_subject", {
        subject: this.subject,
      });
      this.view = next;
      this.loadError = null;
      const stillThere =
        this.selectedId !== null &&
        next.conclusions.some((c) => c.id === this.selectedId);
      if (!stillThere) {
        this.selectedId =
          [...next.conclusions].sort((a, b) => b.confidence - a.confidence)[0]?.id ??
          null;
      }
      void this.loadActivities(next);
    } catch (error) {
      this.loadError = humanizeError(error);
    } finally {
      this.loading = false;
    }
  }

  /** Resolve cited Activities so the story spine carries real titles, times and
   *  source types, and so "view frame" has something to open. */
  private async loadActivities(view: SubjectView): Promise<void> {
    const wanted = new Set<number>();
    for (const c of view.conclusions) for (const e of c.evidence) wanted.add(e.activityId);
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
    this.activities = resolved;
  }

  async togglePin(c: Conclusion): Promise<void> {
    if (this.actionId !== null) return;
    this.actionId = c.id;
    this.actionKind = "pin";
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      await this.load();
    } catch (error) {
      // A write failure must not blank the surface.
      toast({
        tone: "error",
        title: c.pinned ? "Couldn't unpin conclusion" : "Couldn't pin conclusion",
        message: humanizeError(error),
      });
    } finally {
      this.actionId = null;
      this.actionKind = null;
    }
  }

  /** Dismiss is not a delete — the belief leaves the dossier and can re-form if
   *  the activity still supports it. Confirmed through the Tauri dialog plugin
   *  (never `window.confirm`). */
  async dismiss(c: Conclusion): Promise<void> {
    if (this.actionId !== null) return;
    const ok = await confirm(
      `“${c.statement}”\n\nIt leaves the dossier. If your activity still supports it, it can form again.`,
      { title: "Dismiss this conclusion?", kind: "warning", okLabel: "Dismiss" },
    );
    if (!ok) return;
    this.actionId = c.id;
    this.actionKind = "dismiss";
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
      await this.load();
    } catch (error) {
      toast({
        tone: "error",
        title: "Couldn't dismiss conclusion",
        message: humanizeError(error),
      });
    } finally {
      this.actionId = null;
      this.actionKind = null;
    }
  }

  /** "view frame →" / "view in Timeline →" — hand the Activity's first raw
   *  evidence ref to the main window. */
  async openActivity(activityId: number): Promise<void> {
    await openEvidenceRef(this.activities.get(activityId)?.evidence?.[0]);
  }
}
