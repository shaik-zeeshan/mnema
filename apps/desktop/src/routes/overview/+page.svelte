<script lang="ts">
  // ══ OVERVIEW — the bento ═══════════════════════════════════════════════════
  //
  // Direction 05 "Tactile Instruments". Every tile is a FILL on the window
  // background (`.ti-tile`), never a bordered card: the window ring is this
  // surface's one border. Media leads the grid; tile labels are 10px mono
  // eyebrows so no two tiles compete; exactly ONE `--t-display` number on the
  // page (the Capture hero).
  //
  // Two tiles carry an instrument FACE, and only a face — on Overview an
  // instrument READS, you turn it in Settings:
  //   · Capture → the 24-hour coverage strip (`.ti-cov`), which hours hold capture
  //   · Storage → the day-budget gauge (`.ti-gauge`) with its 7-day-average notch
  //
  // Round-4 **G8** binds every number here: all of them come from the real
  // commands below, and a fact that is null renders NO number — not a
  // placeholder, not a zero (see the no-data faces on both instruments). None
  // of the mockup's invented figures (214 GB free, 2.1 GB/day, 6:42, a 3.0 GB
  // budget) survives into the app.
  //
  // Round-4 **G11** binds the tile list: This Week and Ask history are in;
  // Open Threads is DIGEST PROSE ONLY — the digest's own "one open thread…"
  // sentence, rendered where the mockup drew a structured tile. No entity, no
  // table, no extraction pipeline.
  //
  // Every read is best-effort: a failure leaves the tile in its empty face
  // rather than raising a banner, the same silent-failure contract
  // `system-facts.svelte.ts` documents for G8 denominators.
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import type { ConversationCluster, Moment } from "$lib/highlights";
  import type { DayCoverage } from "$lib/types/app-infra";
  import type { UserContextDigest, UserContextStatus } from "$lib/types";
  import type {
    AuthoredContext,
    ConfidenceSnapshot,
    Conclusion,
    SubjectView,
  } from "$lib/types/recording";
  import type { ConversationSummary } from "$lib/insights/conversation";
  import ConfidenceTrace from "$lib/overview/ConfidenceTrace.svelte";
  import { captureControls } from "$lib/capture-controls.svelte";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { coarseRuntime } from "$lib/settings/state/system-facts";
  import { formatBytes } from "$lib/settings/state/format";
  import { getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
  import {
    busiestBar,
    capturedLabel,
    clockLabel,
    dayKey,
    dayWindow,
    heroHours,
    hourCells,
    indexDays,
    minutesLabel,
    storageGauge,
    weekBars,
    weekTotalMs,
  } from "$lib/overview/day-math";

  // Only the fields this surface reads; the full shape lives in
  // `crates/capture-types/src/usage_charts.rs`.
  interface UsageCharts {
    timePerApp: { app: string; activeMs: number; frameCount: number }[];
  }

  const MOMENT_LIMIT = 5;
  const CONVERSATION_ROWS = 3;
  // One row, as the mockup draws it: at 1100×720 the four tile rows have to fit
  // without the bento scrolling, and the history's job here is to prove the
  // store is being read — the full list lives behind the door.
  const ASK_ROWS = 1;
  // The Subjects tile is a door with two treads, exactly as the mockup draws
  // it: the whole list lives behind the header link.
  const SUBJECT_ROWS = 2;

  let now = $state(new Date());
  let coverage = $state<DayCoverage[]>([]);
  let moments = $state<Moment[]>([]);
  let conversations = $state<ConversationCluster[]>([]);
  let asks = $state<ConversationSummary[]>([]);
  let digest = $state<UserContextDigest | null>(null);
  let context = $state<UserContextStatus | null>(null);
  let conclusions = $state<Conclusion[]>([]);
  let authored = $state<AuthoredContext[]>([]);
  // Per-subject confidence history for the two Subjects rows, fetched lazily
  // after the tile knows which subjects it shows. G8: a subject whose fetch
  // fails or has fewer than two snapshots renders NO trace — never a
  // straight-line stand-in.
  let traces = $state(new Map<string, ConfidenceSnapshot[]>());
  let frameCount = $state<number | null>(null);
  let loaded = $state(false);

  const platform = detectKeyboardPlatform();
  const askShortcut = (() => {
    const binding = getEffectiveGlobalShortcut("toggleQuickRecall").bindings[0];
    return binding ? formatShortcut(binding, platform).join("") : "";
  })();

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    const today = new Date();
    const { startMs, endMs } = dayWindow(today);
    const [days, moment, convo, ask, dig, ctx, usage, beliefs, written] = await Promise.all([
      invoke<DayCoverage[]>("list_day_coverage").catch(() => []),
      invoke<Moment[]>("get_moments", { startMs, endMs, limit: MOMENT_LIMIT }).catch(() => []),
      invoke<ConversationCluster[]>("get_conversations", { startMs, endMs }).catch(() => []),
      invoke<ConversationSummary[]>("list_conversations", { limit: ASK_ROWS }).catch(() => []),
      invoke<UserContextDigest | null>("get_latest_user_context_digest").catch(() => null),
      invoke<UserContextStatus | null>("get_user_context_status").catch(() => null),
      invoke<UsageCharts | null>("get_usage_charts", { startMs, endMs }).catch(() => null),
      invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }).catch(
        () => [],
      ),
      // Two counts, never one: an authored statement is stored verbatim and
      // never fades; an inferred conclusion carries a confidence that rises
      // and decays. They live in different tables and survive different
      // delete paths, so the Context tile reads them separately.
      invoke<AuthoredContext[]>("list_user_context_authored").catch(() => []),
    ]);
    void systemFacts.ensureLoaded();
    now = today;
    coverage = days ?? [];
    moments = moment ?? [];
    conversations = convo ?? [];
    asks = ask ?? [];
    digest = dig?.narrative ? dig : null;
    context = ctx ?? null;
    conclusions = beliefs ?? [];
    authored = written ?? [];
    const apps = usage?.timePerApp ?? [];
    frameCount = apps.length > 0 ? apps.reduce((sum, a) => sum + (a.frameCount ?? 0), 0) : null;
    loaded = true;
  }

  // ── The day ──────────────────────────────────────────────────────────────
  const index = $derived(indexDays(coverage));
  const todayCoverage = $derived(index.get(dayKey(now)));
  const hero = $derived(heroHours(todayCoverage?.coveredMs));
  const cells = $derived(hourCells(todayCoverage?.hours));
  const bars = $derived(weekBars(index, now));
  const weekMs = $derived(weekTotalMs(bars));
  const busiest = $derived(busiestBar(bars));

  const dateLabel = $derived(
    now.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" }),
  );
  // The lede states only what was actually measured — G8 all the way down.
  const summary = $derived.by(() => {
    const parts: string[] = [];
    const captured = capturedLabel(todayCoverage?.coveredMs);
    if (captured) parts.push(`${captured} captured`);
    if (conversations.length > 0) {
      parts.push(`${conversations.length} conversation${conversations.length === 1 ? "" : "s"}`);
    }
    if (frameCount !== null && frameCount > 0) parts.push(`${frameCount.toLocaleString()} frames`);
    return parts.join(" · ");
  });

  // ── The gauge ────────────────────────────────────────────────────────────
  const facts = $derived(systemFacts.value);
  const gauge = $derived(storageGauge(facts));
  const runtime = $derived(coarseRuntime(facts?.diskFreeBytes ?? null, gauge?.perDayBytes ?? null));
  // Assembled here rather than in markup: an `{#if}` inside a sentence eats the
  // space in front of it, and the em-dash clause has to keep its breathing room.
  const gaugeReadout = $derived.by(() => {
    if (!gauge) return "";
    const week = `the notch is the last ${gauge.measuredDays} ${
      gauge.measuredDays === 1 ? "day" : "days"
    } (${formatBytes(gauge.windowBytes)})`;
    return runtime
      ? `The bar is a month at this rate, ${week} — ${runtime} left.`
      : `The bar is a month at this rate, ${week}.`;
  });

  // ── The Subjects door ────────────────────────────────────────────────────
  // Subjects are derived client-side by grouping conclusions, the same read
  // the destination makes. Each row shows the subject's HIGHEST-confidence
  // conclusion, and the tile ranks subjects by that number.
  const topSubjects = $derived.by(() => {
    const best = new Map<string, Conclusion>();
    for (const belief of conclusions) {
      const held = best.get(belief.subject);
      if (!held || belief.confidence > held.confidence) best.set(belief.subject, belief);
    }
    return [...best.values()]
      .sort((a, b) => b.confidence - a.confidence || a.subject.localeCompare(b.subject))
      .slice(0, SUBJECT_ROWS);
  });

  // Best-effort, and only for the subjects actually on screen — at most two
  // calls. Reading `traces` here would re-trigger this effect, so the map is
  // rebuilt from the rows rather than merged into.
  $effect(() => {
    const rows = topSubjects;
    if (rows.length === 0) return;
    let cancelled = false;
    void (async () => {
      const next = new Map<string, ConfidenceSnapshot[]>();
      await Promise.all(
        rows.map(async (row) => {
          try {
            const view = await invoke<SubjectView>("get_user_context_subject", {
              subject: row.subject,
            });
            const line = view.trajectories.find((t) => t.conclusionId === row.id);
            if (line && line.history.length >= 2) next.set(row.subject, line.history);
          } catch {
            // No trace rather than an invented one.
          }
        }),
      );
      if (!cancelled) traces = next;
    })();
    return () => {
      cancelled = true;
    };
  });

  const subjectHref = (subject: string): string =>
    `/overview/subjects/${encodeURIComponent(subject)}`;

  const newestAuthored = $derived.by(() => {
    if (authored.length === 0) return null;
    return [...authored].sort((a, b) => b.updatedAtMs - a.updatedAtMs)[0];
  });

  // "47 conclusions across 16 subjects, inferred separately" — both halves are
  // real reads or the line does not render at all.
  const inferredLine = $derived.by(() => {
    if (!context || context.conclusionCount <= 0) return null;
    const beliefs = `${context.conclusionCount.toLocaleString()} conclusion${
      context.conclusionCount === 1 ? "" : "s"
    }`;
    if (context.subjectCount <= 0) return `${beliefs}, inferred separately`;
    return `${beliefs} across ${context.subjectCount.toLocaleString()} subject${
      context.subjectCount === 1 ? "" : "s"
    }, inferred separately`;
  });

  const digestStamp = $derived(digest ? clockLabel(digest.generatedAtMs) : null);

  function hideBroken(event: Event): void {
    // A frame the retention pass culled leaves a dead path; an empty cell reads
    // better than a broken-image glyph.
    (event.currentTarget as HTMLImageElement).style.visibility = "hidden";
  }

  async function openQuickAccess(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // Best-effort: the global shortcut stays the summon path.
    }
  }
</script>

{#snippet chevron()}
  <svg
    class="ov__chev"
    width="8"
    height="12"
    viewBox="0 0 8 12"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path d="m1.5 1 5 5-5 5" />
  </svg>
{/snippet}

<div class="ov">
  <header class="ov__hd">
    <p class="t-title">{dateLabel}</p>
    {#if summary}<p class="t-meta is-mono is-num">{summary}</p>{/if}
  </header>

  <div class="ti-tiles">
    <!-- Media leads the grid: the moments filmstrip, edge to edge, 1px apart. -->
    <div class="ti-tile ti-tile--4 ti-tile--media">
      {#if moments.length > 0}
        <div class="ti-strip ov__strip">
          {#each moments as moment (moment.frameId)}
            <span class="ov__frame">
              <img src={convertFileSrc(moment.filePath)} alt="" onerror={hideBroken} />
              <span class="ti-strip__t">{clockLabel(moment.capturedAtMs)}</span>
            </span>
          {/each}
        </div>
      {:else}
        <div class="ov__mediaempty">
          <span class="t-label">Moments</span>
          <p class="t-meta">
            {loaded
              ? "Nothing captured today yet. Mnema picks one headline frame per activity, so the strip fills as the day does."
              : "Reading today…"}
          </p>
        </div>
      {/if}
    </div>

    <!-- The digest, as prose. G11: the "one open thread…" sentence lives INSIDE
         this narrative — v1 has no open-threads entity and no extraction. -->
    <div class="ti-tile ti-tile--2">
      <div class="ti-tile-h">
        <span class="t-label">Today</span>
        {#if digestStamp}
          <span class="ti-more t-meta is-mono is-num">updated {digestStamp}</span>
        {/if}
      </div>
      {#if digest}
        {#if digest.headline}<p class="ov__headline">{digest.headline}</p>{/if}
        <p class="t-read ov__prose">{digest.narrative}</p>
      {:else}
        <p class="t-meta ov__empty">
          No read of today yet. Mnema writes one once the Reasoning Engine has
          enough of a day to read — including whatever it finds still open.
        </p>
      {/if}
      <!-- Journal's door. The digest is the first line of the day, so "the
           whole day" is the only sentence this tile needs — and it stands
           whether or not a digest was written, because the Journal is the day
           itself, not the read of it. -->
      <a class="ti-tile-row ov__door" href="/overview/journal">
        <span class="t-meta ov__doorlbl">The whole day, hour by hour</span>
        {@render chevron()}
      </a>
    </div>

    <!-- INSTRUMENT FACE 1 — coverage. Hours are physical; this one only reads. -->
    <div class="ti-tile">
      <div class="ti-tile-h"><span class="t-label">Capture</span></div>
      {#if hero}
        <div class="ov__hero">
          <span class="t-display is-num">{hero}</span><span class="t-meta">hours today</span>
        </div>
      {:else}
        <p class="t-meta ov__empty ov__empty--tight">Nothing captured yet today.</p>
      {/if}
      <div class="ti-well ov__covwell">
        <div class="ti-cov" aria-label="Capture by hour of today">
          {#each cells as lit, hour (hour)}
            <i class:h={lit}></i>
          {/each}
        </div>
      </div>
      <div class="ti-cov__scale ov__scale" aria-hidden="true">
        <span>00</span><span>08</span><span>16</span><span>24</span>
      </div>
      <div class="ti-tile-row ov__foot">
        <span class="ov__dot" class:ov__dot--live={captureControls.isRunning}></span>
        <span class="t-meta">{captureControls.isRunning ? "Recording" : "Not recording"}</span>
      </div>
    </div>

    <!-- INSTRUMENT FACE 2 — the day-budget gauge. See `storageGauge()` for why
         the fill is the measured window rather than the mockup's "today": today
         is the one day `system_facts` deliberately does not measure (G8). -->
    <div class="ti-tile">
      <div class="ti-tile-h">
        <span class="t-label">Storage</span>
        {#if gauge}
          <span class="ti-more t-meta is-mono is-num">{gauge.measuredDays}d measured</span>
        {/if}
      </div>
      {#if gauge}
        <div class="ov__hero">
          <span class="t-ui is-mono is-num ov__value">{formatBytes(gauge.perDayBytes)}</span>
          <span class="t-meta">a day, measured</span>
        </div>
        <div class="ti-well ti-gauge ov__gauge">
          <span class="ti-gauge__track">
            <span
              class="ti-gauge__seg"
              class:ti-gauge__seg--a={!gauge.tight}
              class:ti-gauge__seg--warn={gauge.tight}
              style:width="{gauge.fillPct}%"
            ></span>
            <span class="ti-gauge__notch" style:left="{gauge.notchPct}%"></span>
          </span>
        </div>
        <div class="ti-gauge__scale ov__scale" aria-hidden="true">
          <span>0</span><span>{formatBytes(gauge.freeBytes)} free</span>
        </div>
        <div class="ti-tile-row ov__foot">
          <span class="t-meta">{gaugeReadout}</span>
        </div>
      {:else}
        <p class="t-meta ov__empty">
          No complete capture day measured yet, so Mnema will not guess what a day
          costs. The gauge appears once there is a real day to divide.
        </p>
      {/if}
    </div>

    <!-- Audio is conversations, never minutes of tape. -->
    <div class="ti-tile ti-tile--2">
      <div class="ti-tile-h">
        <span class="t-label">Conversations</span>
        {#if conversations.length > 0}
          <span class="ti-more t-meta is-num">{conversations.length} today</span>
        {/if}
      </div>
      {#if conversations.length > 0}
        {#each conversations.slice(0, CONVERSATION_ROWS) as conversation (conversation.activityId)}
          <div class="ti-grow ov__row">
            <span class="ti-grow__txt">
              <span class="ti-grow__lbl">{conversation.title}</span>
              <span class="ti-grow__sub">
                {minutesLabel(conversation.spokenMs)} spoken · {conversation.speakerCount}
                {conversation.speakerCount === 1 ? "speaker" : "speakers"}
              </span>
            </span>
            <span class="ti-grow__val t-meta is-mono is-num">
              {clockLabel(conversation.startedAtMs)}
            </span>
          </div>
        {/each}
      {:else}
        <p class="t-meta ov__empty">
          No conversations today. One appears when an activity overlaps at least
          two minutes of recorded speech.
        </p>
      {/if}
    </div>

    <!-- G11: This Week — the same per-day coverage read the jump menu uses. -->
    <div class="ti-tile">
      <div class="ti-tile-h"><span class="t-label">This week</span></div>
      <div class="ov__bars" aria-hidden="true">
        {#each bars as bar (bar.key)}
          <span class="ti-spark ov__bar">
            {#if bar.fraction > 0}
              <i
                class:is-on={bar.isToday}
                style:height="{Math.max(6, bar.fraction * 100)}%"
                style:width="100%"
              ></i>
            {/if}
          </span>
        {/each}
      </div>
      <div class="ov__barlabels" aria-hidden="true">
        {#each bars as bar (bar.key)}
          <span class="t-label" class:ov__barlabel--today={bar.isToday}>{bar.label}</span>
        {/each}
      </div>
      <div class="ti-tile-row ov__foot">
        {#if weekMs > 0}
          <span class="t-meta is-mono is-num">{capturedLabel(weekMs)}</span>
          {#if busiest}<span class="t-meta ov__push">busiest {busiest.label}</span>{/if}
        {:else}
          <span class="t-meta">Nothing captured in the last seven days.</span>
        {/if}
      </div>
    </div>

    <!-- Context. The mockup's "142 facts about you" and "3 pending" are both
         corrected here: there is no review queue and no pending state on a
         context statement, and merging what you WROTE with what the engine
         INFERRED would be one number over two stores with two lifetimes. Two
         reads, stated separately. -->
    <div class="ti-tile">
      <div class="ti-tile-h">
        <span class="t-label">Context</span>
        <a class="ti-more ov__more" href="/overview/context">Review all ›</a>
      </div>
      {#if authored.length > 0 || inferredLine}
        {#if authored.length > 0}
          <div class="ti-tile-row">
            <span class="t-ui ov__value is-num">{authored.length.toLocaleString()}</span>
            <span class="t-meta">
              statement{authored.length === 1 ? "" : "s"} you wrote
            </span>
          </div>
        {/if}
        {#if newestAuthored}
          <div class="ti-tile-row">
            <span class="t-meta ov__quote">Newest: “{newestAuthored.text}”</span>
          </div>
        {/if}
        {#if inferredLine}
          <div class="ti-tile-row ov__foot">
            <span class="t-meta is-num ov__subtle">{inferredLine}</span>
          </div>
        {/if}
      {:else}
        <p class="t-meta ov__empty">
          Nothing here yet. Write a statement to steer your dossier, or let the
          Reasoning Engine infer conclusions as it reads your activities.
        </p>
      {/if}
    </div>

    <!-- Subjects — the door to the conviction list. Two rows, each a subject,
         each with its own confidence trace: the tile and the destination are
         the same instrument at two sizes. -->
    <div class="ti-tile ti-tile--2">
      <div class="ti-tile-h">
        <span class="t-label">Subjects</span>
        <a class="ti-more ov__more" href="/overview/subjects">
          {#if context && context.subjectCount > 0}
            {context.subjectCount.toLocaleString()} views ›
          {:else}
            All subjects ›
          {/if}
        </a>
      </div>
      {#if topSubjects.length > 0}
        {#each topSubjects as row (row.subject)}
          <a class="ti-grow ov__row ov__srow" href={subjectHref(row.subject)}>
            <span class="ti-grow__txt">
              <span class="ti-grow__lbl">{row.subject}</span>
              <span class="ti-grow__sub ov__clip">{row.statement}</span>
            </span>
            <span class="ti-grow__val">
              {#if traces.get(row.subject)}
                <ConfidenceTrace
                  history={traces.get(row.subject) ?? []}
                  label={`Confidence over time for ${row.subject}`}
                />
              {/if}
              {@render chevron()}
            </span>
          </a>
        {/each}
      {:else}
        <p class="t-meta ov__empty">
          No subjects yet. As the Reasoning Engine forms views about you, each
          one appears here with its own confidence trace.
        </p>
      {/if}
    </div>

    <!-- G11: Ask history — a conversation-store read, plus the door itself. -->
    <div class="ti-tile ti-tile--2">
      <div class="ti-tile-h">
        <span class="t-label">Ask</span>
        <span class="ti-more t-meta">history</span>
      </div>
      {#if asks.length > 0}
        {#each asks.slice(0, ASK_ROWS) as ask (ask.conversationId)}
          <div class="ti-grow ov__row">
            <span class="ti-grow__txt">
              <span class="ti-grow__lbl">{ask.title || ask.preview}</span>
              <span class="ti-grow__sub">
                {ask.turnCount} {ask.turnCount === 1 ? "turn" : "turns"}
              </span>
            </span>
            <span class="ti-grow__val t-meta is-mono is-num">{clockLabel(ask.updatedAtMs)}</span>
          </div>
        {/each}
      {:else}
        <p class="t-meta ov__empty ov__empty--tight">
          Nothing asked yet. Anything you ask Quick Access lands here.
        </p>
      {/if}
      <button type="button" class="ov__ask" onclick={() => void openQuickAccess()}>
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          aria-hidden="true"
        >
          <circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" />
        </svg>
        <span class="ov__askph">Ask about your day…</span>
        {#if askShortcut}<kbd class="kbd">{askShortcut}</kbd>{/if}
      </button>
    </div>
  </div>
</div>

<style>
  .ov {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: var(--s-12);
    display: flex;
    flex-direction: column;
    gap: var(--s-12);
  }

  .ov__hd {
    display: flex;
    align-items: baseline;
    gap: var(--s-12);
    flex: 0 0 auto;
  }
  .ov__hd p {
    margin: 0;
  }

  /* ── tiles ──────────────────────────────────────────────────────────────
     Every rule below is placement inside a tile the shared skin already draws;
     no tile grows a border here — the window ring is this surface's only one. */
  .ov__hero {
    display: flex;
    align-items: baseline;
    gap: var(--gap-inline);
  }
  .ov__value {
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }
  .ov__prose,
  .ov__headline {
    margin: 0;
  }
  .ov__headline {
    font: var(--w-medium) var(--t-ui) / 1.35 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .ov__empty {
    margin: 0;
    max-width: 46ch;
    color: var(--app-text-subtle);
  }
  .ov__empty--tight {
    margin-bottom: auto;
  }
  .ov__row {
    padding: var(--s-4) 0;
    min-height: 31px;
  }
  .ov__foot {
    margin-top: auto;
    padding-top: var(--s-2);
  }
  .ov__push {
    margin-left: auto;
  }
  .ov__subtle {
    color: var(--app-text-subtle);
  }
  .ov__quote,
  .ov__clip {
    display: block;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── the three doors ──────────────────────────────────────────────────────
     Journal, Subjects and Context are destinations INSIDE Overview. A door is
     a plain link — no button face, no border: the tile it sits in is already
     the surface step. */
  .ov__more {
    font: 510 var(--t-meta) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-accent);
    text-decoration: none;
  }
  .ov__more:hover {
    text-decoration: underline;
  }
  .ov__door {
    margin-top: auto;
    padding-top: var(--s-4);
    text-decoration: none;
    color: var(--app-accent);
  }
  .ov__doorlbl {
    font-weight: 510;
    color: var(--app-accent);
  }
  .ov__door .ov__chev {
    margin-left: auto;
    color: var(--app-accent);
  }
  .ov__door:hover .ov__doorlbl {
    text-decoration: underline;
  }
  .ov__chev {
    color: var(--app-text-faint);
    flex: 0 0 auto;
  }
  /* Subject rows: the whole row is the target, so it carries the hover, and
     the row hairline `.ti-grow` already draws stays the only separator. */
  .ov__srow {
    padding-left: 0;
    padding-right: 0;
    border-radius: var(--r-md);
    text-decoration: none;
    color: inherit;
  }
  .ov__srow:hover {
    background: var(--app-surface-hover);
  }
  .ov__srow:hover .ov__chev {
    color: var(--app-text-muted);
  }
  .ov__door:focus-visible,
  .ov__srow:focus-visible,
  .ov__more:focus-visible {
    outline: none;
    box-shadow: var(--ring);
    border-radius: var(--r-md);
  }

  /* Four content rows now (Subjects joined the bento), so the grid gap and the
     filmstrip band both give back what the fourth row costs — the whole bento
     still fits 1100×720 without scrolling. */
  .ov :global(.ti-tiles) {
    gap: var(--s-12);
  }
  .ov__strip {
    height: 84px;
  }
  /* The digest is one headline and one paragraph; clamped so a long read can
     never push the fourth row off the window. The whole day is one click away
     through the door below it. */
  .ov__prose {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    overflow: hidden;
  }
  .ov__frame {
    position: relative;
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    display: block;
    background: var(--ti-empty);
  }
  .ov__frame img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .ov__mediaempty {
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: var(--s-12);
  }
  .ov__mediaempty p {
    margin: 0;
    max-width: 70ch;
    color: var(--app-text-subtle);
  }

  /* the two instrument faces */
  .ov__covwell {
    padding: 5px 6px;
    margin-top: var(--s-6);
  }
  .ov__gauge {
    margin-top: var(--s-6);
  }
  .ov__scale {
    padding: 0 1px;
  }

  .ov__dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--app-text-faint);
    flex: 0 0 auto;
  }
  /* Recording red is a STATE, never an error. */
  .ov__dot--live {
    background: var(--app-record);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-record) 18%, transparent);
  }

  /* this week */
  .ov__bars {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    height: 38px;
    padding-top: var(--s-2);
  }
  .ov__bar {
    flex: 1 1 0;
    height: 100%;
    align-items: flex-end;
  }
  .ov__barlabels {
    display: flex;
    gap: 6px;
    margin-top: 3px;
  }
  .ov__barlabels span {
    flex: 1 1 0;
    text-align: center;
    color: var(--app-text-faint);
    text-transform: none;
  }
  .ov__barlabel--today {
    color: var(--app-accent);
  }

  /* The ask door. A recessed field, not a bordered box: depth here is a surface
     step plus an INSET ring (the direction's `.ti-qfield` treatment), so the
     page's bordered-container count stays at one — the window. */
  .ov__ask {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    width: 100%;
    height: var(--h-md);
    margin-top: var(--s-4);
    padding: 0 var(--pad-control);
    border: 0;
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    color: var(--app-text-subtle);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    text-align: left;
    cursor: default;
  }
  .ov__ask:hover {
    background: var(--app-surface-hover);
  }
  .ov__ask:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }
  .ov__askph {
    flex: 1 1 auto;
  }

  /* ── the 800×600 floor ────────────────────────────────────────────────────
     Two columns. The hero and the gauge bar stay; the tick labels are what
     drop, because a gauge without a number is decoration but a gauge without
     tick labels is still a gauge. */
  @media (max-width: 900px) {
    .ov :global(.ti-tiles) {
      grid-template-columns: repeat(2, 1fr);
    }
    .ov :global(.ti-tile--3),
    .ov :global(.ti-tile--4) {
      grid-column: span 2;
    }
    .ov__scale {
      display: none;
    }
  }
</style>
