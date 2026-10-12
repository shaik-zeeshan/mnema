<script lang="ts">
  // JournalRiver — the Journal's lower half (insights.html `.in-feed`): "What
  // you did", newest first (the live edge on top, then cards and away-gaps),
  // each card expanding in place into its receipt; beside it the side column
  // (This week day bars, Day mix, Apps). Pure presentation: the parent builds
  // the rows (journal-view.ts) and owns all loading.
  import type { ActivityFocus, ActivityMeta, UserContextSummarizingFailure } from "$lib/types/recording";
  import type { JournalPending } from "$lib/insights/journal-day";
  import { pendingReasonCopy, riverRowKey, type RiverRow, type WeekDay } from "$lib/insights/journal-view";
  import { formatClock, formatClockRange } from "$lib/time-format";
  import { CATEGORY_COLOR, UNCATEGORIZED_COLOR, categoryLabel, humanizeMs } from "$lib/insights/activity-helpers";
  import { openSettings } from "$lib/surface-windows";
  import { tip } from "$lib/components/tooltip";
  import ReceiptViewer from "$lib/insights/ReceiptViewer.svelte";
  import IconPlay from "~icons/lucide/play";
  import IconOff from "~icons/lucide/circle-off";
  import IconInbox from "~icons/lucide/inbox";
  import IconAlert from "~icons/lucide/triangle-alert";

  type Mix = { category: keyof typeof CATEGORY_COLOR | null; ms: number }[];
  interface Props {
    rows: RiverRow[];
    pending: JournalPending;
    /** Per-activity app chips + speaker count; absent ids have neither. */
    meta: Record<number, ActivityMeta>;
    /** Summarizing keeps failing (status field); null when healthy. */
    failure: UserContextSummarizingFailure | null;
    dayLabel: string;
    /** Capture-run day: why nothing was written up (replaces the river). */
    nothingWrittenWhy: string | null;
    week: WeekDay[];
    dayStartMs: number;
    mix: Mix;
    apps: { app: string; activeMs: number }[];
    hot: number | null;
    onHot: (id: number | null) => void;
    onPickDay: (startMs: number) => void;
  }
  let { rows, pending, meta, failure, dayLabel, nothingWrittenWhy, week, dayStartMs, mix, apps, hot, onHot, onPickDay }: Props =
    $props();

  let openId = $state<number | null>(null);

  const cat = (c: Mix[number]["category"]) => `var(${c ? CATEGORY_COLOR[c] : UNCATEGORIZED_COLOR})`;
  const catName = (c: Mix[number]["category"]) => (c ? categoryLabel(c) : "Uncategorized");
  const FOCUS: Record<ActivityFocus, [string, string]> = {
    deep: ["--focus-deep", "Deep"],
    mixed: ["--focus-mid", "Mixed"],
    distracted: ["--focus-distracted", "Scattered"],
  };
  const cards = $derived(rows.filter((r) => r.kind === "card").length);
  const mixTotal = $derived(mix.reduce((t, m) => t + m.ms, 0));
  /** "7h" — the week bar's caption drops the minutes. */
  const hoursOnly = (ms: number) => humanizeMs(ms).replace(/ \d+m$/, "");
  const BAR_MS = 8 * 3_600_000; // a full 48px bar = 8h
  const weekday = (ms: number) => new Date(ms).toLocaleDateString(undefined, { weekday: "short" });
  const longDay = (ms: number) => new Date(ms).toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" });
</script>

<div class="in-feed">
  <div class="mx-reveal" style="--i:4">
    <div class="in-sec">
      <h2 class="mx-h2">What you did</h2>
      <span class="mx-label">
        {nothingWrittenWhy ? "nothing written up" : `${cards} activit${cards === 1 ? "y" : "ies"} · newest first`}
      </span>
    </div>
    {#if nothingWrittenWhy}
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconOff width="14" height="14" aria-hidden="true" /></span>
        <b class="mx-empty__title">Nothing written up on {dayLabel}</b>
        <p class="mx-empty__text">{nothingWrittenWhy} The Timeline has everything captured.</p>
      </div>
    {:else}
      {#if pending.active && pending.reason && pending.sinceMs !== null}
        {@const since = formatClock(pending.sinceMs)}
        <div class="in-pending">
          <time class="num">{since}</time>
          <div>
            {#if pending.reason.kind === "summarizing" && failure}
              <span class="mx-kicker">Not written up · {since} → now</span>
              <span class="mx-inline" data-tone="danger">
                <IconAlert width="13" height="13" aria-hidden="true" />
                Summaries are failing — {failure.provider ?? "the AI provider"} turned away the last
                {failure.failures === 1 ? "try" : `${failure.failures} tries`}.
                <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => void openSettings("intelligence")}>Open engine settings</button>
              </span>
            {:else if pending.reason.kind === "summarizing"}
              <span class="mx-kicker mx-kicker--live">Writing up {since} → now</span>
              <div class="mx-skel" style="width:46%;height:12px"></div>
              <div class="mx-skel" style="width:72%"></div>
              <span class="mx-label">
                {pending.audioOnly
                  ? "Screen capture is off, so this stretch is built from what the mic and system audio picked up."
                  : "The journal trails live capture by a few minutes — the footage is already on the Timeline."}
              </span>
            {:else}
              <span class="mx-kicker">Not written up · {since} → now</span>
              <span class="mx-inline"><IconOff width="13" height="13" aria-hidden="true" />{pendingReasonCopy(pending.reason.reason)}</span>
            {/if}
          </div>
        </div>
      {/if}
      {#each rows as row (riverRowKey(row))}
        {#if row.kind === "gap"}
          <div class="in-gap">Away · {humanizeMs(row.gap.endMs - row.gap.startMs)} · {formatClockRange(row.gap.startMs, row.gap.endMs)}{row.gap.audio ? " · audio only" : ""}</div>
        {:else if row.kind === "failed"}
          <div class="in-gap">Captured, but summarizing it failed · {formatClockRange(row.span.startMs, row.span.endMs)}</div>
        {:else}
          {@const a = row.slot.activity}
          {@const open = openId === a.id}
          <article
            class="in-act"
            id="in-act-{a.id}"
            data-hot={hot === a.id ? "" : undefined}
            onmouseenter={() => onHot(a.id)}
            onmouseleave={() => onHot(null)}
          >
            <div class="in-act__t"><time>{formatClock(a.startedAtMs)}</time><span class="num">{humanizeMs(a.endedAtMs - a.startedAtMs)}</span></div>
            <div>
              <h4><span class="in-sw" style="--c:{cat(a.category ?? null)}"></span>{a.title}</h4>
              {#if a.summary}<p>{a.summary}</p>{/if}
              <div class="in-act__meta">
                <span class="mx-chip mx-chip--cat" style="--c:{cat(a.category ?? null)}">{catName(a.category ?? null)}</span>
                {#if a.focus}
                  <span class="mx-chip in-focus" style="--c:var({FOCUS[a.focus][0]})">{FOCUS[a.focus][1]}</span>
                {/if}
                {#each meta[a.id]?.apps ?? [] as app (app)}<span class="mx-chip">{app}</span>{/each}
                {#if meta[a.id]?.speakers}
                  <span class="mx-chip mx-chip--mic">{meta[a.id].speakers} speaker{meta[a.id].speakers === 1 ? "" : "s"}</span>
                {/if}
                {#if row.slot.media === "expired"}<span class="mx-chip">footage expired</span>{/if}
                <button
                  type="button"
                  class="mx-btn mx-btn--ghost mx-btn--sm"
                  aria-pressed={open}
                  aria-expanded={open}
                  onclick={() => (openId = open ? null : a.id)}
                ><IconPlay width="11" height="11" aria-hidden="true" />Receipt</button>
              </div>
              {#if open}<ReceiptViewer activity={a} />{/if}
            </div>
          </article>
        {/if}
      {/each}
      {#if cards === 0 && pending.active}
        <div class="mx-empty mx-empty--compact">
          <span class="mx-empty__glyph"><IconInbox width="14" height="14" aria-hidden="true" /></span>
          <b class="mx-empty__title">Your day is being written</b>
          <p class="mx-empty__text">Capture is landing. The first card appears within a few minutes.</p>
        </div>
      {/if}
    {/if}
  </div>

  <div class="in-side mx-reveal" style="--i:5">
    <div>
      <div class="in-sec"><span class="mx-kicker">This week</span><span class="in-spacer"></span><span class="mx-label"><kbd>[</kbd> <kbd>]</kbd> to step</span></div>
      <div class="in-wk">
        {#each week as d (d.startMs)}
          {#if d.future}
            <button type="button" disabled><span class="in-wk__bar"></span><b>{weekday(d.startMs)}</b>—</button>
          {:else}
            {@const capOnly = d.activities === 0 && d.capturedHours > 0}
            <button
              type="button"
              aria-pressed={d.startMs === dayStartMs}
              use:tip={`${longDay(d.startMs)} · ${d.activities ? `${humanizeMs(d.activityMs)} · ${d.activities} activities` : capOnly ? `~${d.capturedHours}h captured · nothing written up` : "nothing captured"}`}
              onclick={() => onPickDay(d.startMs)}
            >
              <span class="in-wk__bar">
                {#if capOnly}
                  <i style="--c:var(--chart-grey-3);height:{Math.min(48, (d.capturedHours / 8) * 48)}px"></i>
                {:else}
                  {#each [...d.mix].reverse() as m (m.category ?? "none")}<i style="--c:{cat(m.category)};height:{(m.ms / BAR_MS) * 48}px"></i>{/each}
                {/if}
              </span>
              <b>{weekday(d.startMs)}</b>{d.activities ? hoursOnly(d.activityMs) : capOnly ? `${d.capturedHours}h` : "—"}
            </button>
          {/if}
        {/each}
      </div>
    </div>
    {#if mix.length > 0 && !nothingWrittenWhy}
      <div>
        <div class="in-sec"><span class="mx-kicker">Day mix</span><span class="in-spacer"></span><span class="mx-label">activity time</span></div>
        <div class="in-mixrows">
          {#each mix as m, i (m.category ?? "none")}
            <div class="in-mixrow">
              <span class="in-sw" style="--c:{cat(m.category)}"></span><span>{catName(m.category)}</span>
              <span class="in-cat__bar"><i style="--c:{cat(m.category)};--w:{(m.ms / mix[0].ms) * 100}%;--d:{i}"></i></span>
              <span class="num">{humanizeMs(m.ms)}</span><span class="num">{Math.round((m.ms / mixTotal) * 100)}%</span>
            </div>
          {/each}
        </div>
      </div>
    {/if}
    {#if apps.length > 0}
      <div>
        <div class="in-sec"><span class="mx-kicker">Apps</span><span class="in-spacer"></span><span class="mx-label">app time</span></div>
        <dl class="mx-spec in-apps">
          {#each apps as x, i (x.app)}
            <div>
              <dt>{x.app}</dt>
              <dd><span class="in-cat__bar"><i style="--w:{(x.activeMs / apps[0].activeMs) * 100}%;--d:{i + 4}"></i></span></dd>
              <dd class="mx-spec__note num">{humanizeMs(x.activeMs)}</dd>
            </div>
          {/each}
        </dl>
      </div>
    {/if}
  </div>
</div>

<style>
  /* insights.html `.in-feed` / `.in-act` / `.in-side` (kit tokens only). */
  .in-feed { display: grid; grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr); gap: var(--s-6); align-items: start; }
  .in-sec { display: flex; align-items: baseline; gap: var(--s-3); margin: 0 0 var(--s-3); }
  .in-spacer { flex: 1; }
  .in-sw { display: inline-block; width: 8px; height: 8px; border-radius: 2px; background: var(--c); flex: none; }
  .in-act { display: grid; grid-template-columns: 76px minmax(0, 1fr); gap: var(--s-4); padding: var(--s-4) var(--s-3); margin: 0 calc(var(--s-3) * -1); border-top: 1px solid var(--mx-hairline); border-radius: var(--r-sm); transition: background-color var(--t-fast) var(--ease-quart); scroll-margin-top: 80px; }
  .in-act:hover, .in-act[data-hot] { background: color-mix(in srgb, var(--app-fg) 2.5%, transparent); }
  .in-act__t { display: grid; gap: 5px; align-content: start; padding-top: 2px; }
  .in-act__t time { font: 500 var(--text-md)/1 var(--font-mono); color: var(--app-text-strong); }
  .in-act__t span { font-size: var(--text-xs); color: var(--app-text-subtle); }
  .in-act h4 { display: flex; align-items: center; gap: 9px; margin: 0; font: 600 var(--text-lg)/1.3 var(--font-sans); letter-spacing: -0.01em; color: var(--app-text-strong); }
  .in-act h4 .in-sw { width: 9px; height: 9px; }
  .in-act p { margin: 6px 0 10px; color: var(--app-text-muted); font-size: var(--text-md); line-height: 1.5; }
  .in-act__meta { display: flex; flex-wrap: wrap; gap: 5px; align-items: center; margin-top: 10px; }
  .in-act p + .in-act__meta { margin-top: 0; }
  .in-act__meta .mx-btn { margin-left: auto; }
  .in-focus { color: var(--app-text); border-color: color-mix(in srgb, var(--c) 35%, var(--app-border)); background: color-mix(in srgb, var(--c) 10%, var(--app-surface)); }
  .in-focus::before { content: ""; width: 6px; height: 6px; border-radius: 50%; background: var(--c); }
  .in-gap { display: flex; align-items: center; gap: var(--s-3); padding: 10px 0 10px 92px; border-top: 1px solid var(--mx-hairline); font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-faint); }
  .in-gap::after { content: ""; flex: 1; border-top: 1px dashed var(--app-border); }
  .in-pending { display: grid; grid-template-columns: 76px minmax(0, 1fr); gap: var(--s-4); padding: var(--s-4) 0; }
  .in-pending > div { display: grid; gap: 8px; justify-items: start; }
  .in-pending > div > .mx-skel { justify-self: stretch; }
  .in-pending time { font: 500 var(--text-md)/1 var(--font-mono); color: var(--app-accent); padding-top: 2px; }
  .in-feed .mx-empty--compact { max-width: none; padding-inline: var(--s-3); }
  .in-side { display: grid; gap: var(--s-6); align-content: start; }
  .in-wk { display: grid; grid-template-columns: repeat(7, 1fr); gap: 6px; margin-top: var(--s-3); }
  .in-wk button { display: grid; gap: 6px; justify-items: center; padding: 8px 0 7px; border: 1px solid transparent; border-radius: var(--r-md); background: none; cursor: pointer; font: 500 var(--text-xs)/1 var(--font-mono); color: var(--app-text-subtle); transition: background-color var(--t-fast) var(--ease-quart); }
  .in-wk button:hover:not(:disabled) { background: var(--app-surface-hover); }
  .in-wk button:focus-visible { outline: 0; box-shadow: var(--app-ring); }
  .in-wk button[aria-pressed="true"] { border-color: var(--app-border-strong); background: var(--app-surface); color: var(--app-text-strong); }
  .in-wk button:disabled { cursor: default; color: var(--app-text-faint); }
  .in-wk__bar { width: 14px; height: 48px; display: flex; flex-direction: column-reverse; gap: 1px; }
  .in-wk__bar i { display: block; flex: none; background: var(--c); transform-origin: bottom; animation: in-grow-y var(--t-slow) var(--ease-expo) both; }
  .in-wk__bar i:last-child { border-radius: 2px 2px 0 0; }
  .in-wk__bar:empty { height: 20px; margin-top: 28px; border: 1px dashed var(--app-border); border-radius: 2px; }
  .in-wk b { font-weight: 500; color: inherit; }
  .in-mixrows { display: grid; }
  .in-mixrow { display: grid; grid-template-columns: 10px 104px 1fr 56px 36px; align-items: center; gap: var(--s-3); padding: 7px 0; font: 500 var(--text-base)/1 var(--font-sans); color: var(--app-text); }
  .in-mixrow .num { font-size: var(--text-sm); color: var(--app-text-strong); text-align: right; }
  .in-mixrow .num + .num { color: var(--app-text-subtle); }
  .in-cat__bar { height: 6px; border-radius: 3px; background: color-mix(in srgb, var(--app-fg) 6%, transparent); overflow: hidden; }
  .in-cat__bar i { display: block; height: 100%; width: var(--w); background: var(--c); border-radius: 3px; transform-origin: left; animation: in-grow-x var(--t-slow) var(--ease-expo) both; animation-delay: calc(200ms + var(--d) * 40ms); }
  .in-apps { grid-template-columns: 82px 1fr auto; }
  .in-apps dt { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .in-apps dd:not(.mx-spec__note) { display: flex; align-items: center; }
  .in-apps .in-cat__bar { flex: 1; height: 4px; }
  .in-apps .in-cat__bar i { background: var(--chart-grey-3); }
  @keyframes in-grow-x { from { transform: scaleX(0); } }
  @keyframes in-grow-y { from { transform: scaleY(0); } }
  @container mx-window (max-width: 1180px) { .in-feed { grid-template-columns: 1fr; } }
  @media (prefers-reduced-motion: reduce) { .in-wk__bar i, .in-cat__bar i { animation: none; } }
</style>
