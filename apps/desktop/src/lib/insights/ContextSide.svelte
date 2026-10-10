<script lang="ts" module>
  /** The fixed authored-context topics (IMPLEMENTATION.md §8 #9), each on a
   *  category swatch: Work · Preferences · People · Goals, or none. */
  export const TOPICS: { name: string; color: string }[] = [
    { name: "Work", color: "var(--cat-creating)" },
    { name: "Preferences", color: "var(--cat-organizing)" },
    { name: "People", color: "var(--cat-communication)" },
    { name: "Goals", color: "var(--cat-learning)" },
  ];
  export const topicColor = (t: string | null) =>
    TOPICS.find((x) => x.name === t)?.color ?? "var(--chart-grey-3)";
  /** guardrail.rs `SensitiveCategory` — five, in enum order. */
  const SENSITIVE: [string, string?][] = [
    ["health & mental health"],
    ["sexuality"],
    ["religion"],
    ["politics"],
    ["other protected", "race / ethnicity, disability, immigration status, financial distress"],
  ];
  export const GUARDED = SENSITIVE.length;
</script>

<script lang="ts">
  // Context side column (insights.html renderContext `.in-side`): what the
  // dossier is built from (authored per topic, counted client-side; inferred is
  // the grey share of the total), the strongest inferred views, the guardrail,
  // and the Dismissed archive. Presentational — Context owns the data.
  import { tip } from "$lib/components/tooltip";
  import type { AuthoredContext, Conclusion, DismissedView } from "$lib/types/recording";
  import IconLock from "~icons/lucide/lock";
  import IconInbox from "~icons/lucide/inbox";

  interface Props {
    authored: AuthoredContext[];
    /** Visible inferred conclusions; null while loading or unreadable. */
    inferred: Conclusion[] | null;
    dismissed: DismissedView[];
    dismissedError: string | null;
    restoringKey: string | null;
    openDismissed: boolean;
    onRestore: (d: DismissedView) => void;
    onOpenSubject?: (subject: string) => void;
  }
  let { authored, inferred, dismissed, dismissedError, restoringKey, openDismissed, onRestore, onOpenSubject }: Props =
    $props();

  const n = $derived(authored.length);
  const inferredCount = $derived(inferred?.length ?? 0);
  const byTopic = $derived(TOPICS.map((t) => authored.filter((s) => s.topic === t.name).length));
  const maxTopic = $derived(Math.max(1, ...byTopic));
  // Same pick as before the redesign: the top 3 visible conclusions by confidence.
  const strongest = $derived([...(inferred ?? [])].sort((a, b) => b.confidence - a.confidence).slice(0, 3));

  const key = (d: DismissedView) => `${d.subject}\0${d.statement}`;
  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  // Arriving from Subjects' "View dismissed": bring the archive into view.
  let dismissedEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (openDismissed && dismissedEl) dismissedEl.scrollIntoView({ block: "start", behavior: "smooth" });
  });
</script>

<div class="in-side">
  <div class="mx-panel in-mix mx-reveal" style="--i:4">
    <div class="in-sec"><span class="mx-kicker">What the dossier is built from</span></div>
    <div class="in-mix__total">
      <i style="--c:var(--app-text-strong);flex:{n || 0.001}" use:tip={`Authored · ${n} — you said it, never fades`}></i>
      <i style="--c:var(--chart-grey-3);flex:{inferredCount || 0.001}" use:tip={`Inferred · ${inferredCount} — read from activity, scored, can fade`}></i>
    </div>
    <div class="in-mix__l">
      <span><span class="in-sw" style="--c:var(--app-text-strong)"></span>authored <b>{n}</b></span>
      <span><span class="in-sw in-sw--hatch" style="--c:var(--chart-grey-4)"></span>inferred <b>{inferredCount}</b></span>
    </div>
    {#if n}
      {#each TOPICS as t, i (t.name)}
        <div class="in-mix__row">
          <span><span class="in-sw" style="--c:{t.color}"></span>{t.name}</span>
          <div class="in-mix__bar" style="width:{(byTopic[i] / maxTopic) * 100}%;--c:{t.color}">
            {#if byTopic[i]}<i class="a" style="--d:{i}"></i>{/if}
          </div>
          <span class="num"><b>{byTopic[i]}</b></span>
        </div>
      {/each}
    {/if}
  </div>

  <div class="mx-reveal" style="--i:5">
    <div class="in-sec">
      <span class="mx-kicker">Strongest inferred views</span><span class="mx-spacer"></span><span class="mx-label">by confidence</span>
    </div>
    {#if strongest.length}
      <div class="in-steer">
        {#each strongest as c (c.id)}
          <button type="button" onclick={() => onOpenSubject?.(c.subject)} disabled={!onOpenSubject}>
            {c.statement}<span class="num">{Math.round(c.confidence * 100)}%</span>
            <small>{c.subject} · inferred — say it here to make it standing</small>
          </button>
        {/each}
      </div>
    {:else}
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconInbox width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">No inferred views yet</b>
        <p class="mx-empty__text">As Mnema forms views, the strongest show here.</p>
      </div>
    {/if}
  </div>

  <div class="mx-panel mx-panel--inset in-guard mx-reveal" style="--i:6">
    <span class="mx-kicker"><IconLock width="12" height="12" aria-hidden="true" />Sensitive Category Guardrail</span>
    <div class="in-guard__chips">
      {#each SENSITIVE as [label, more] (label)}
        {#if more}<span class="mx-chip" use:tip={more}>{label}</span>{:else}<span class="mx-chip">{label}</span>{/if}
      {/each}
    </div>
    Mnema never infers these categories about you. What you write here is sent as written.
  </div>

  {#if dismissed.length || dismissedError}
    <div class="mx-reveal" style="--i:7" bind:this={dismissedEl}>
      <div class="in-sec">
        <span class="mx-kicker">Dismissed beliefs</span><span class="mx-spacer"></span><span class="mx-label">won't be re-inferred</span>
      </div>
      {#if dismissedError}<p class="mx-inline" data-tone="danger">{dismissedError}</p>{/if}
      {#each dismissed as d (key(d))}
        {@const busy = restoringKey === key(d)}
        <div class="in-dismiss">
          <div><p>{d.statement}</p><small>{d.subject} · dismissed {day(d.dismissedAtMs)}</small></div>
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" disabled={busy} aria-busy={busy} onclick={() => onRestore(d)}>
            {#if busy}<span class="mx-spin mx-spin--sm"></span>Restoring…{:else}Restore{/if}
          </button>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  /* insights.html Context side: `.in-mix` · `.in-steer` · `.in-guard` · `.in-dismiss`. */
  .in-side {
    display: grid;
    gap: var(--s-6);
    align-content: start;
  }
  .in-sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin: 0 0 var(--s-3);
  }
  .in-sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
    flex: none;
  }
  .in-sw--hatch {
    background: repeating-linear-gradient(135deg, var(--c) 0 1.5px, color-mix(in srgb, var(--c) 25%, transparent) 1.5px 4px);
  }
  .in-mix {
    padding: var(--s-5);
  }
  .in-mix__total {
    display: flex;
    height: 10px;
    gap: 2px;
    margin: var(--s-4) 0 var(--s-2);
  }
  .in-mix__total i,
  .in-mix__bar i {
    border-radius: 2px;
    background: var(--c);
    transform-origin: left;
    animation: grow-x var(--t-slow) var(--ease-expo) both;
    animation-delay: 200ms;
  }
  .in-mix__l {
    display: flex;
    gap: var(--s-4);
    font: 500 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
    margin-bottom: var(--s-4);
  }
  .in-mix__l > span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .in-mix__l b {
    color: var(--app-text-strong);
    font: 500 var(--text-sm) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
  }
  .in-mix__row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr) 56px;
    gap: var(--s-3);
    align-items: center;
    padding: 8px 0;
    border-top: 1px solid var(--mx-hairline);
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text);
  }
  .in-mix__row > span:first-child {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .in-mix__bar {
    display: flex;
    gap: 2px;
    height: 14px;
  }
  .in-mix__bar i {
    flex: 1;
    animation-delay: calc(240ms + var(--d) * 40ms);
  }
  .in-mix__row .num {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    text-align: right;
  }
  .in-mix__row .num b {
    color: var(--app-text-strong);
    font-weight: 500;
  }
  .in-steer {
    display: grid;
  }
  .in-steer button {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 5px var(--s-3);
    padding: 10px 0;
    border: 0;
    border-top: 1px solid var(--mx-hairline);
    background: none;
    font: inherit;
    font-size: var(--text-base);
    text-align: left;
    color: var(--app-text-strong);
    cursor: pointer;
  }
  .in-steer button:disabled {
    cursor: default;
  }
  .in-steer button:not(:disabled):hover {
    color: var(--app-accent);
  }
  .in-steer button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .in-steer small {
    grid-column: 1 / -1;
    font: 400 var(--text-sm) / 1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .in-steer .num {
    color: var(--app-text-strong);
    font-size: var(--text-sm);
  }
  .in-side .mx-empty--compact {
    max-width: none;
    padding-inline: 0;
  }
  .in-guard {
    padding: var(--s-4);
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    line-height: 1.55;
  }
  .in-guard .mx-kicker {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .in-guard__chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: var(--s-3) 0;
  }
  .in-guard__chips .mx-chip {
    text-decoration: line-through;
    text-decoration-color: var(--app-text-faint);
    color: var(--app-text-subtle);
  }
  .in-dismiss {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-3);
    align-items: center;
    padding: 10px 0;
    border-top: 1px solid var(--mx-hairline);
  }
  .in-dismiss p {
    margin: 0;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    text-decoration: line-through;
    text-decoration-color: var(--app-text-faint);
  }
  .in-dismiss small {
    display: block;
    margin-top: 4px;
    font: 400 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  @keyframes grow-x {
    from {
      transform: scaleX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .in-mix__total i,
    .in-mix__bar i {
      animation: none;
    }
  }
</style>
