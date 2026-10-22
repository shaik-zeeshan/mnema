<script lang="ts" module>
  import type { DigestAbsentReason, UserContextDigest } from "$lib/types/recording";
  import type { FocusCounts, LedeTopCategory } from "$lib/insights/lede-stats";
  import type { OverviewState } from "$lib/insights/OverviewStates.svelte";

  /** The period the lede speaks about. */
  export interface LedePeriod {
    unit: "day" | "week" | "month";
    current: boolean;
    /** "this week" · "today" · "Oct 5 – 11". */
    scope: string;
    /** "week 41" · "October" · "Fri, Oct 9". */
    when: string;
  }
  /** Capture-side figures (get_usage_charts + capture_presence). */
  export interface LedeCapture {
    loading: boolean;
    failed: boolean;
    trackedMs: number;
    /** Change vs the matched slice of the previous period; null = no baseline. */
    deltaMs: number | null;
    dailyAvgMs: number;
    capturedDays: number;
    appCount: number;
    topApp: string | null;
    /** Why there is no app time (audio only / removed by retention). */
    noAppTime: { cap: string; note: string } | null;
    partlyRemoved: boolean;
    /** Nothing captured at all in the period. */
    nothing: boolean;
    /** 2a: the current period is empty and Mnema isn't recording. */
    idle: OverviewState | null;
  }
  /** Engine-side state: the read + activity figures. */
  export interface LedeEngine {
    /** AI off / never set up / derivation off: capture-only. */
    capOnly: boolean;
    /** Engine on but unreachable / signed out: nothing new is written. */
    paused: boolean;
    loading: boolean;
    failed: boolean;
    activityCount: number;
    focus: FocusCounts;
    deepDelta: number | null;
    top: LedeTopCategory | null;
    digest: UserContextDigest | null;
    /** Why the digest command served no read (null while loading / on a read). */
    digestReason: DigestAbsentReason | null;
    digestLoading: boolean;
    regenerating: boolean;
    digestError: string | null;
    waitingFirst: boolean;
    recordingSince: string | null;
    /** "Older than the 30 days Mnema summarizes on its own." */
    olderNote: string | null;
    watermark: string | null;
    catchingUp: boolean;
  }
</script>

<script lang="ts">
  import { formatClock, localizeClockTimes } from "$lib/time-format";
  // The Overview lede (insights.html `.in-read`): kicker · mx-display headline ·
  // prose · foot on the left, four derived stats on the right. Every state of
  // the read is one honest sentence in the headline's place.
  import OverviewStates from "$lib/insights/OverviewStates.svelte";
  import { turnErrorCopy } from "$lib/insights/engine-state";
  import { fmtDuration } from "$lib/insights/lede-stats";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconAlert from "~icons/lucide/triangle-alert";

  interface Props {
    period: LedePeriod;
    capture: LedeCapture;
    engine: LedeEngine;
    onReread: () => void;
  }
  let { period, capture, engine, onReread }: Props = $props();

  /** Mirrors `digest::DIGEST_SENSITIVE_HOLD`. */
  const SENSITIVE_HOLD = "digest_sensitive_hold";

  const p = $derived(period);
  const e = $derived(engine);
  const c = $derived(capture);
  const unitWord = $derived(p.unit === "day" ? "day" : p.unit);
  const kicker = $derived(`${p.current ? "The read so far" : "The read"} · ${p.when}`);
  const plainKicker = $derived(`This ${unitWord} · ${p.when}`);
  const prevWord = $derived(p.unit === "day" ? "yesterday" : `last ${p.unit}`);
  const held = $derived(e.digestError === SENSITIVE_HOLD);
  const failText = $derived(e.digestError && !held ? turnErrorCopy(e.digestError, null, null).text : "");
  const tooFew = $derived(e.digestReason === "too_few");
  /** What stops a new read: the status guess, then the digest's own reason. */
  const pausedBy = $derived(
    e.digestReason === "derivation_off"
      ? "continuous derivation is off"
      : e.paused || e.digestReason === "engine_not_ready"
        ? "AI is paused"
        : null,
  );
  // IN-B1 joins `headline — turn`; the turn reads de-emphasized.
  const headline = $derived.by(() => {
    const h = localizeClockTimes(e.digest?.headline?.trim() ?? "");
    const k = h.indexOf(" — ");
    return k < 0 ? { head: h, turn: "" } : { head: h.slice(0, k), turn: h.slice(k + 1) };
  });
  const written = $derived.by(() => {
    if (!e.digest) return "";
    const d = new Date(e.digest.generatedAtMs);
    const t = formatClock(d);
    return d.toDateString() === new Date().toDateString()
      ? t
      : `${d.toLocaleDateString("en-US", { month: "short", day: "numeric" })}, ${t}`;
  });
  const signed = (ms: number) => `${ms < 0 ? "−" : "+"}${fmtDuration(Math.abs(ms))}`;
  /** "30h 47m" → [["30","h"],["47","m"]] for the stat's small units. */
  const parts = (s: string) => [...s.matchAll(/(\d+)([a-z]+)/g)].map((m) => [m[1], m[2]]);
</script>

<div class="in-read">
  <div class="mx-reveal lede" style="--i:1">
    {#if c.loading}
      <div aria-busy="true">
        <span class="mx-sr">Loading the overview</span>
        <div class="skel-read">
          <div class="mx-skel" style="width:140px;height:9px"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
        </div>
      </div>
    {:else if c.nothing && e.activityCount === 0}
      <span class="mx-kicker">{plainKicker}</span>
      <h1 class="mx-display">Nothing captured {p.scope}.</h1>
      {#if c.idle}
        <OverviewStates state={c.idle} />
      {:else}
        <p class="mx-prose">
          Mnema wasn't recording {p.current ? `${p.scope} yet` : "in this period"}, so there's nothing
          to read. Step to another {unitWord}, or start recording.
        </p>
      {/if}
    {:else if e.capOnly}
      <span class="mx-kicker">{plainKicker}</span>
      {#if c.failed}
        <h1 class="mx-display">Couldn't load app time {p.scope}.</h1>
      {:else if c.noAppTime}
        <h1 class="mx-display">No app time {p.scope}.</h1>
        <p class="mx-prose">{c.noAppTime.note}</p>
      {:else}
        <h1 class="mx-display">
          {fmtDuration(c.trackedMs)} tracked across {c.appCount}
          {c.appCount === 1 ? "app" : "apps"}.{#if c.topApp}{" "}<em>Most of it in {c.topApp}.</em>{/if}
        </h1>
      {/if}
    {:else if pausedBy && !e.digest}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">No new read while {pausedBy}.</h1>
      <p class="mx-prose">
        Everything below was written before.{e.digestReason === "derivation_off" ? "" : " The read comes back on its own."}
      </p>
    {:else if e.digest}
      <span class="mx-kicker">{kicker}</span>
      {#key e.digest.generatedAtMs}
        {#if headline.head}
          <h1 class="mx-display">{headline.head}{#if headline.turn}{" "}<em>{headline.turn}</em>{/if}</h1>
        {/if}
        {#if !e.digestError || e.regenerating || !headline.head}
          <p class="mx-prose">{localizeClockTimes(e.digest.narrative)}</p>
        {/if}
      {/key}
      <div class="foot">
        {#if e.digestError && !e.regenerating}
          <span class="mx-inline" data-tone="danger" role="alert"
            ><IconAlert width="13" height="13" aria-hidden="true" />Couldn't re-read: {held
              ? "the new read touched a private category."
              : failText}<button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onReread}
              >Retry</button
            ></span
          >
        {:else}
          <span class="mx-label num">written {written}</span>
          {#if e.watermark}<span class="mx-label num">· {e.watermark}</span>{/if}
          {#if !e.paused}{@render reread()}{/if}
        {/if}
      </div>
    {:else if e.digestLoading || e.regenerating || e.loading}
      <span class="mx-kicker">{kicker}</span>
      <div aria-busy="true">
        <span class="mx-sr">Writing the read</span>
        <div class="skel-read">
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel mx-skel--title"></div>
          <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div>
        </div>
      </div>
      <div class="foot">
        <span class="mx-inline"><span class="mx-spin mx-spin--sm"></span>Writing the read for {p.scope}…</span>
      </div>
    {:else if held}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">The read for {p.scope} was held back.</h1>
      <p class="mx-prose">
        It touched a private category, so Mnema dropped it rather than show it. Your activities are
        untouched. Re-read, or step to another {unitWord}.
      </p>
      <div class="foot">{@render reread()}</div>
    {:else if e.digestError}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">Couldn't write the read for {p.scope}.</h1>
      <div class="foot">
        <span class="mx-inline" data-tone="danger" role="alert"
          ><IconAlert width="13" height="13" aria-hidden="true" />{failText}<button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            onclick={onReread}>Retry</button
          ></span
        >
      </div>
    {:else if e.waitingFirst}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">Waiting for the first summary.</h1>
      <p class="mx-prose">
        {e.recordingSince ? `Recording since ${e.recordingSince}. ` : ""}The first summary usually lands
        within about 10 minutes, and the read is written once there are two activities.
      </p>
    {:else if e.olderNote}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">No read for this {unitWord}.</h1>
      <p class="mx-prose">{e.olderNote}</p>
    {:else if tooFew}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">Too little captured {p.scope} to write a read.</h1>
      <p class="mx-prose">
        A read needs at least two activities{e.activityCount < 2
          ? `; this ${unitWord} has ${e.activityCount === 1 ? "one" : "none"} so far`
          : " it can use"}. The numbers on the right still count.
      </p>
    {:else}
      <span class="mx-kicker">{kicker}</span>
      <h1 class="mx-display">No read for {p.scope} yet.</h1>
      <p class="mx-prose">It's written on its own once there's enough. Re-read asks for one now.</p>
      <div class="foot">{@render reread()}</div>
    {/if}
    {#if e.catchingUp && !e.capOnly}
      <div class="foot"><OverviewStates state={{ kind: "catching-up" }} /></div>
    {/if}
  </div>

  <div class="stats mx-reveal" style="--i:2">
    {#if c.loading}
      {#each Array.from({ length: 4 }) as _, i (i)}
        <div class="skel-stat"><div class="mx-skel"></div><div class="mx-skel"></div></div>
      {/each}
    {:else}
      <div class="mx-stat">
        <span class="mx-stat__label">Tracked · app time</span>
        {#if c.failed}
          <span class="mx-stat__value">—</span>
          <span class="mx-stat__delta">couldn't load</span>
        {:else if c.noAppTime}
          <span class="mx-stat__value">—</span><span class="mx-stat__delta">{c.noAppTime.cap}</span>
        {:else}
          <span class="mx-stat__value">{@render big(c.trackedMs)}</span>
          {#if c.nothing}
            <span class="mx-stat__delta">no capture</span>
          {:else if e.capOnly || !tooFew || e.digest}
            {#if c.partlyRemoved}
              <span class="mx-stat__delta">partly removed by retention</span>
            {:else if c.deltaMs != null}
              <span class="mx-stat__delta" class:mx-stat__delta--up={c.deltaMs > 0 && !e.capOnly}
                >{signed(c.deltaMs)} vs {prevWord}</span
              >
            {/if}
          {:else}
            <span class="mx-stat__delta">app active time</span>
          {/if}
        {/if}
      </div>
      {#if !e.capOnly && tooFew && !e.digest && !c.nothing && !e.loading}
        <div class="mx-stat">
          <span class="mx-stat__label">Activities</span>
          <span class="mx-stat__value">{e.activityCount}</span>
          <span class="mx-stat__delta">a read needs 2</span>
        </div>
      {:else}
        <div class="mx-stat">
          <span class="mx-stat__label">Daily avg</span>
          {#if c.failed}
            <span class="mx-stat__value">—</span><span class="mx-stat__delta">couldn't load</span>
          {:else if c.nothing || c.noAppTime}
            <span class="mx-stat__value">—</span>
          {:else}
            <span class="mx-stat__value">{@render big(c.dailyAvgMs)}</span>
            <span class="mx-stat__delta">over {c.capturedDays} captured day{c.capturedDays === 1 ? "" : "s"}</span>
          {/if}
        </div>
        {#if !e.capOnly && !c.nothing}
          <div class="mx-stat">
            <span class="mx-stat__label">Deep focus · by activity</span>
            {#if e.loading}
              <div class="mx-skel stat-skel"></div>
            {:else if e.failed}
              <span class="mx-stat__value">—</span>
              <span class="mx-stat__delta">couldn't load</span>
            {:else if !e.focus.n}
              <span class="mx-stat__value">—</span><span class="mx-stat__delta">no focus read yet</span>
            {:else}
              <span class="mx-stat__value"
                >{Math.round((e.focus.deep / e.focus.n) * 100)}<small>%</small></span
              >
              <span class="mx-stat__delta" class:mx-stat__delta--up={(e.deepDelta ?? 0) > 0}
                >{e.focus.deep} of {e.focus.n} activit{e.focus.n === 1 ? "y" : "ies"}{e.deepDelta != null
                  ? ` · ${e.deepDelta >= 0 ? "+" : "−"}${Math.abs(e.deepDelta)} pts`
                  : ""}</span
              >
            {/if}
          </div>
          <div class="mx-stat">
            <span class="mx-stat__label">Top category</span>
            {#if e.loading}
              <div class="mx-skel stat-skel"></div>
            {:else if e.failed || !e.top}
              <span class="mx-stat__value">—</span>
              {#if e.failed}<span class="mx-stat__delta">couldn't load</span>{/if}
            {:else}
              <span class="mx-stat__value tval"
                ><span class="sw" style="--c:var({e.top.colorVar})"></span>{e.top.label}</span
              >
              <span class="mx-stat__delta">{e.top.pct}% of activity time</span>
            {/if}
          </div>
        {/if}
      {/if}
    {/if}
  </div>
</div>

{#snippet big(ms: number)}{#each parts(fmtDuration(ms)) as [n, u], i (i)}{i ? " " : ""}{n}<small>{u}</small>{/each}{/snippet}

{#snippet reread()}
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--sm"
    aria-busy={e.regenerating || undefined}
    disabled={e.regenerating || e.digestLoading}
    onclick={onReread}
  >
    {#if e.regenerating}<span class="mx-spin mx-spin--sm"></span>Re-reading…{:else}<IconRetry
        width="13"
        height="13"
        aria-hidden="true"
      />Re-read{/if}
  </button>
{/snippet}

<style>
  .in-read {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-7);
    align-items: end;
    padding: var(--s-4) 0 var(--s-6);
    margin-bottom: var(--s-6);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .lede .mx-display {
    margin: var(--s-4) 0;
    max-width: 24ch;
  }
  .lede .mx-display em {
    font-style: normal;
    font-weight: 500;
    color: var(--app-text-subtle);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--s-3);
    margin-top: var(--s-5);
  }
  .skel-read {
    display: grid;
    gap: 12px;
    padding-top: var(--s-4);
  }
  .skel-read .mx-skel--title {
    height: 34px;
    width: 78%;
  }
  .skel-read .mx-skel--title + .mx-skel--title {
    width: 52%;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(132px, auto));
    gap: var(--s-5) var(--s-6);
    padding-bottom: 4px;
  }
  .skel-stat {
    display: grid;
    gap: 8px;
    width: 132px;
  }
  .skel-stat .mx-skel:first-child {
    height: 8px;
    width: 60%;
  }
  .skel-stat .mx-skel:last-child {
    height: 24px;
    width: 90%;
  }
  .stat-skel {
    height: 28px;
    width: 90px;
  }
  .tval {
    font: 600 22px / 1 var(--font-sans);
    letter-spacing: -0.02em;
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .sw {
    width: 10px;
    height: 10px;
    border-radius: 2px;
    background: var(--c);
    flex: none;
  }
</style>
