<script lang="ts">
  // Subjects — the browsable Subjects index sub-surface (issue #106).
  //
  // A Subject is a browsable entity that holds MULTIPLE individual Conclusions,
  // each with its OWN confidence-over-time trajectory — NEVER a single rolled-up
  // sentiment score. The grid mirrors `docs/user-context/mockups/subjects-index.html`:
  // each card shows the Subject name + a trend glyph, a meta line, a multi-line
  // micro-sparkline (one faint line per Conclusion), the top conclusion headline,
  // and a pin glyph if any conclusion is pinned. Faded subjects (all conclusions
  // faded) render dimmed but stay listed ("kept for history").
  //
  // Subjects are derived CLIENT-SIDE from `list_user_context_conclusions`,
  // grouped by `subject`. To draw HONEST per-conclusion trajectories (not flat
  // baselines), we lazily fetch `get_user_context_subject` per subject and use
  // each trajectory's real Confidence History; if that fetch fails or has no
  // history we fall back to a flat single-point baseline from current confidence.
  //
  // Props:
  //   onOpenSubject: (subject: string) => void

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import type { Conclusion, SubjectView } from "$lib/types/recording";
  import Sparkline from "$lib/insights/charts/Sparkline.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  // Number of placeholder cards shown while the conclusions load.
  const SKELETON_COUNT = 6;

  interface Props {
    onOpenSubject: (subject: string) => void;
  }

  let { onOpenSubject }: Props = $props();

  // Category palette cycled to colour each conclusion's sparkline line.
  const CAT_PALETTE = [
    "--cat-coding",
    "--cat-research",
    "--cat-communication",
    "--cat-design",
    "--cat-testing",
    "--cat-personal",
    "--cat-distractions",
  ] as const;

  const FLOOR = 0.15;

  type SortMode = "active" | "moved" | "az";

  type Trend = "up" | "steady" | "down";

  interface SubjectSpark {
    colorVar: string;
    faded: boolean;
    points: number[];
  }

  interface SubjectRow {
    subject: string;
    conclusions: Conclusion[];
    conclusionCount: number;
    pinned: boolean;
    faded: boolean; // all conclusions faded
    headline: string; // top (highest-confidence) conclusion statement
    lastMovedAtMs: number; // most recent updated/last-supported across conclusions
    trend: Trend;
    spark: SubjectSpark[];
  }

  let conclusions = $state<Conclusion[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);
  let sort = $state<SortMode>("active");

  // Per-subject real trajectory history, fetched lazily. Maps subject → (map of
  // conclusionId → oldest-first confidence points). Used to draw honest spark
  // lines + derive warming/steady/cooling from the start-vs-end of the arc.
  let trajectories = $state<Map<string, Map<number, number[]>>>(new Map());

  function groupSubjects(list: Conclusion[]): Map<string, Conclusion[]> {
    const groups = new Map<string, Conclusion[]>();
    for (const c of list) {
      const key = c.subject;
      const bucket = groups.get(key);
      if (bucket) bucket.push(c);
      else groups.set(key, [c]);
    }
    return groups;
  }

  // Derive a Subject's trend from its conclusions' movement. Prefer real
  // trajectory history (first vs last point); otherwise fall back to status
  // (a faded subject is cooling, an otherwise-fresh one is steady).
  function deriveTrend(
    subject: string,
    cs: Conclusion[],
    history: Map<number, number[]> | undefined,
  ): Trend {
    let delta = 0;
    let measured = 0;
    for (const c of cs) {
      const pts = history?.get(c.id);
      if (pts && pts.length >= 2) {
        delta += pts[pts.length - 1] - pts[0];
        measured += 1;
      }
    }
    if (measured > 0) {
      const avg = delta / measured;
      if (avg > 0.04) return "up";
      if (avg < -0.04) return "down";
      return "steady";
    }
    // No history available — infer from status.
    const allFaded = cs.every((c) => c.status === "faded");
    if (allFaded) return "down";
    return "steady";
  }

  function buildSpark(
    cs: Conclusion[],
    history: Map<number, number[]> | undefined,
  ): SubjectSpark[] {
    // One line per Conclusion, coloured by cycling the category palette. Prefer
    // real history points; fall back to a flat baseline from current confidence.
    return cs.map((c, i) => {
      const pts = history?.get(c.id);
      const points =
        pts && pts.length > 0
          ? pts
          : [c.confidence, c.confidence]; // flat baseline (2 pts so a line draws)
      return {
        colorVar: CAT_PALETTE[i % CAT_PALETTE.length],
        faded: c.status === "faded",
        points,
      };
    });
  }

  const rows = $derived.by<SubjectRow[]>(() => {
    if (!conclusions) return [];
    const groups = groupSubjects(conclusions);
    const out: SubjectRow[] = [];
    for (const [subject, cs] of groups) {
      const history = trajectories.get(subject);
      const sorted = [...cs].sort((a, b) => b.confidence - a.confidence);
      const top = sorted[0];
      const lastMovedAtMs = cs.reduce(
        (acc, c) => Math.max(acc, c.updatedAtMs, c.lastSupportedAtMs),
        0,
      );
      out.push({
        subject,
        conclusions: sorted,
        conclusionCount: cs.length,
        pinned: cs.some((c) => c.pinned),
        faded: cs.every((c) => c.status === "faded"),
        headline: top?.statement ?? subject,
        lastMovedAtMs,
        trend: deriveTrend(subject, cs, history),
        spark: buildSpark(sorted, history),
      });
    }
    return out;
  });

  const sortedRows = $derived.by<SubjectRow[]>(() => {
    const list = [...rows];
    switch (sort) {
      case "active":
        // Most active = most conclusions, then most-recently moved.
        list.sort(
          (a, b) =>
            b.conclusionCount - a.conclusionCount ||
            b.lastMovedAtMs - a.lastMovedAtMs ||
            a.subject.localeCompare(b.subject),
        );
        break;
      case "moved":
        list.sort(
          (a, b) =>
            b.lastMovedAtMs - a.lastMovedAtMs ||
            a.subject.localeCompare(b.subject),
        );
        break;
      case "az":
        list.sort((a, b) => a.subject.localeCompare(b.subject));
        break;
    }
    // Keep faded subjects listed but sink them below live ones in every mode.
    list.sort((a, b) => Number(a.faded) - Number(b.faded));
    return list;
  });

  const countLabel = $derived(
    `${sortedRows.length} ${sortedRows.length === 1 ? "subject" : "subjects"}`,
  );

  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    if (day < 7) return `${day}d ago`;
    const wk = Math.floor(day / 7);
    if (wk < 5) return `${wk}w ago`;
    const mo = Math.floor(day / 30);
    if (mo < 12) return `${mo}mo ago`;
    const yr = Math.floor(day / 365);
    return `${yr}y ago`;
  }

  function trendLabel(t: Trend): string {
    return t === "up" ? "▲ warming" : t === "down" ? "▼ cooling" : "– steady";
  }

  async function loadConclusions(): Promise<void> {
    loading = true;
    try {
      const list = await invoke<Conclusion[]>("list_user_context_conclusions", {
        includeFaded: true,
      });
      conclusions = list;
      loadError = null;
      void loadTrajectories(list);
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
      conclusions = conclusions ?? [];
    } finally {
      loading = false;
    }
  }

  // Lazily fetch real per-subject Confidence History so the sparklines + trend
  // glyphs reflect actual movement rather than flat baselines. Best-effort: a
  // failed fetch just leaves that subject on its baseline. Bounded concurrency
  // keeps a large dossier responsive.
  async function loadTrajectories(list: Conclusion[]): Promise<void> {
    const subjects = [...new Set(list.map((c) => c.subject))];
    const next = new Map<string, Map<number, number[]>>();
    const CONCURRENCY = 4;
    let cursor = 0;
    async function worker(): Promise<void> {
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
          // Best-effort; subject keeps its flat baseline.
        }
      }
    }
    await Promise.all(
      Array.from({ length: Math.min(CONCURRENCY, subjects.length) }, worker),
    );
    trajectories = next;
  }

  $effect(() => {
    void untrack(() => loadConclusions());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadConclusions();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

<section class="subjects" aria-label="Subjects">
  <!-- Header -->
  <div class="subj-header">
    <div class="titles">
      <h1>Subjects</h1>
      <p class="subtitle">
        Things Mnema has formed a view about — browse how each one has moved.
      </p>
    </div>
    <div class="subj-controls">
      <div class="sort-seg" role="group" aria-label="Sort subjects">
        <button
          type="button"
          class:active={sort === "active"}
          aria-pressed={sort === "active"}
          onclick={() => (sort = "active")}>Most active</button
        >
        <button
          type="button"
          class:active={sort === "moved"}
          aria-pressed={sort === "moved"}
          onclick={() => (sort = "moved")}>Recently moved</button
        >
        <button
          type="button"
          class:active={sort === "az"}
          aria-pressed={sort === "az"}
          onclick={() => (sort = "az")}>A–Z</button
        >
      </div>
      <span class="subj-count">{countLabel}</span>
    </div>
  </div>

  {#if loadError}
    <div class="state state--error">
      <p class="state-title">Couldn't load Subjects.</p>
      <p class="state-detail">{loadError}</p>
    </div>
  {:else if loading && !conclusions}
    <!-- Loading skeleton grid — mirrors the real card shape so the swap to
         loaded content causes no layout shift. Distinct from the empty state
         below, which only renders AFTER loading completes with no subjects. -->
    <div class="subj-grid" aria-label="Loading subjects" aria-busy="true">
      {#each Array.from({ length: SKELETON_COUNT }) as _, i (i)}
        <div class="card scard scard--skeleton">
          <div class="scard-top">
            <Skeleton variant="text" width="46%" height="14px" />
            <Skeleton variant="text" width="58px" height="14px" radius="999px" />
          </div>
          <Skeleton variant="text" width="64%" height="11px" />
          <div class="scard-spark">
            <Skeleton height="34px" radius="6px" />
          </div>
          <Skeleton variant="text" width="88%" height="12px" />
          <Skeleton variant="text" width="52%" height="12px" />
        </div>
      {/each}
    </div>
  {:else if sortedRows.length === 0}
    <div class="state">
      <p class="state-title">No subjects yet.</p>
      <p class="state-detail">
        As the Reasoning Engine forms views about you, the things it concludes
        appear here — each with its own confidence trajectory. If the engine is
        off, turn it on in Settings → Access to start.
      </p>
    </div>
  {:else}
    <!-- Grid of subject cards -->
    <div class="subj-grid">
      {#each sortedRows as row (row.subject)}
        <button
          type="button"
          class="card scard"
          class:scard--faded={row.faded}
          onclick={() => onOpenSubject(row.subject)}
        >
          <div class="scard-top">
            <span class="scard-name">
              {row.subject}
              {#if row.pinned}
                <span class="scard-pin" title="Pinned">📌</span>
              {/if}
            </span>
            <span class="trend-glyph trend-glyph--{row.trend}">
              {trendLabel(row.trend)}
            </span>
          </div>
          <div class="scard-meta">
            {row.conclusionCount}
            {row.conclusionCount === 1 ? "conclusion" : "conclusions"}
            <span class="dot-sep">·</span>
            {relativeTime(row.lastMovedAtMs)}
          </div>
          <div class="scard-spark">
            <Sparkline series={row.spark} floor={FLOOR} />
          </div>
          <div class="scard-headline">{row.headline}</div>
        </button>
      {/each}
    </div>

    <p class="subj-foot">
      Subjects form as evidence accumulates · faded subjects are kept for history.
    </p>
  {/if}
</section>

<style>
  .subjects {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  /* ---- Header ---- */
  .subj-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    flex-wrap: wrap;
  }
  .titles {
    min-width: 0;
  }
  .subj-header h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .subtitle {
    margin: 3px 0 0;
    font-size: 12px;
    color: var(--app-text-muted);
  }
  .subj-controls {
    display: inline-flex;
    align-items: center;
    gap: 12px;
    flex: 0 0 auto;
  }
  .subj-count {
    font-size: 11.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }

  /* Segmented sort control — mirrors the canonical segmented control look. */
  .sort-seg {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .sort-seg button {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    padding: 0 11px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .sort-seg button:hover {
    color: var(--app-text-strong);
  }
  .sort-seg button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  /* ---- Grid ---- */
  .subj-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 14px;
  }

  /* ---- Card ---- */
  .card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 13px 14px;
  }
  .scard {
    display: flex;
    flex-direction: column;
    gap: 10px;
    text-align: left;
    font: inherit;
    color: inherit;
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease;
  }
  .scard:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }
  .scard--faded {
    opacity: 0.62;
  }
  .scard--skeleton {
    gap: 11px;
    cursor: default;
  }
  .scard--faded:hover {
    opacity: 0.85;
  }

  .scard-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .scard-name {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 14px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
    min-width: 0;
  }
  .scard-pin {
    color: var(--app-text-faint);
    font-size: 11px;
    flex: 0 0 auto;
  }

  .trend-glyph {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 1px 7px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    flex: 0 0 auto;
    white-space: nowrap;
  }
  .trend-glyph--up {
    color: var(--app-accent-strong);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .trend-glyph--down {
    color: var(--app-danger);
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .trend-glyph--steady {
    color: var(--app-text-muted);
  }

  .scard-meta {
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .scard-meta .dot-sep {
    color: var(--app-text-faint);
    margin: 0 2px;
  }

  .scard-spark {
    width: 100%;
  }

  .scard-headline {
    font-size: 12.5px;
    line-height: 1.4;
    color: var(--app-text-strong);
    font-weight: 600;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  /* ---- Footer ---- */
  .subj-foot {
    font-size: 11px;
    color: var(--app-text-muted);
    margin: 4px 0 0;
    padding-top: 12px;
    border-top: 1px solid var(--app-border);
  }

  /* ---- States ---- */
  .state {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .state--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .state-title {
    margin: 0;
    font-size: 13px;
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
    line-height: 1.6;
  }
</style>
