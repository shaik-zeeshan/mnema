<script lang="ts">
  // ── Overview (⌘2) — the bento as a widget board ────────────────────────────
  // Direction 03 (Layered Glass) reads the bento as Sonoma widgets: ONE cell
  // unit, ONE 16px gutter, and only four legal footprints (4×1, 2×2, 2×1, 1×1).
  // Every tile is the same opaque `.plate` with the same 14px radius and the
  // same `--sh-tile`; only the TINT changes, pulled from the tile's subject as a
  // soft glow off its top edge. Text always lands on `--app-surface`.
  //
  // The moments strip is the one padding-zero payload zone: its frames run off
  // the tile edge and are clipped by its radius — the move that stops a bento
  // from reading as a form.
  //
  // Data: every tile is a real read (`lib/overview/overview-data.ts`) or an
  // honest empty state. G8 — no invented number, no denominator that isn't real
  // on this machine. G11 — This Week and Ask history are in; Open Threads is the
  // digest's own prose sentence, nothing extracted.
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { captureControls } from "$lib/capture-controls.svelte";
  import {
    EMPTY_SNAPSHOT,
    loadOverview,
    type OverviewSnapshot,
  } from "$lib/overview/overview-data";
  import { clockAt, heroHours, spokenLabel } from "$lib/overview/overview-shape";
  import { formatCapturedHours } from "$lib/timeline/jumper-coverage";

  let data = $state<OverviewSnapshot>(EMPTY_SNAPSHOT);
  let loaded = $state(false);

  const today = new Date();
  const dayLabel = today.toLocaleDateString(undefined, {
    weekday: "long",
    month: "long",
    day: "numeric",
  });

  const hero = $derived(heroHours(data.capturedTodayMs));
  const weekTotalMs = $derived(data.week.reduce((sum, d) => sum + d.coveredMs, 0));
  const weekPeakMs = $derived(Math.max(1, ...data.week.map((d) => d.coveredMs)));
  const settings = $derived(captureControls.recordingSettings);
  const sources = $derived([
    { key: "screen", on: settings?.captureScreen === true, label: "Screen" },
    { key: "mic", on: settings?.captureMicrophone === true, label: "Microphone" },
    { key: "sys", on: settings?.captureSystemAudio === true, label: "System audio" },
  ]);
  const activeSourceCount = $derived(sources.filter((s) => s.on).length);

  onMount(() => {
    void (async () => {
      data = await loadOverview();
      loaded = true;
    })();
  });

  async function openFrame(frameId: number): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: "frame",
        frameId,
        audioSegmentId: null,
      });
    } catch {
      // The timeline hand-off is a convenience; a failure leaves the board as is.
    }
  }

  async function openAsk(conversationId: string): Promise<void> {
    try {
      await invoke("open_conversation_in_chat", { conversationId });
    } catch {
      // Same: the tile is a read surface, not a workflow.
    }
  }

  async function openQuickAccess(): Promise<void> {
    try {
      await invoke("summon_quick_recall_window_command");
    } catch {
      // Quick Access owns its own failure reporting.
    }
  }
</script>

<div class="ov">
  <div class="ov__scroll">
    <header class="ov__head">
      <h1 class="ov__title">{dayLabel}</h1>
      <p class="ov__sub">
        {#if data.capturedTodayMs > 0}
          {formatCapturedHours(data.capturedTodayMs)} captured
        {:else}
          Nothing captured yet today
        {/if}
        {#if data.conversations.length > 0}
          · {data.conversations.length}
          {data.conversations.length === 1 ? "conversation" : "conversations"}
        {/if}
      </p>
    </header>

    <div class="bento">
      <!-- 4×1 — moments. Padding-zero payload zone: the frames run off the edge. -->
      <section class="tile tile--media tile--w4">
        <div class="strip-h">
          <svg viewBox="0 0 24 24" aria-hidden="true"
            ><rect x="3" y="3.5" width="18" height="17" rx="2.4" /><path
              d="m3.4 16.4 4.7-4.7 4 4 3-3 5.5 5.5"
            /><circle cx="15.6" cy="8.4" r="1.5" /></svg
          >
          <span class="label">Moments · today</span>
        </div>
        {#if data.moments.length > 0}
          <div class="strip">
            {#each data.moments as card (card.moment.frameId)}
              <button
                class="strip__frame"
                type="button"
                title={card.moment.title}
                onclick={() => openFrame(card.moment.frameId)}
              >
                {#if card.previewUrl}
                  <img src={card.previewUrl} alt="" />
                {/if}
                <span class="strip__cap"
                  >{clockAt(card.moment.capturedAtMs)} · {card.moment.title}</span
                >
              </button>
            {/each}
          </div>
        {:else}
          <p class="tile__empty tile__empty--media">
            {loaded ? "No moments yet today — they appear once an activity has a headline frame." : "Loading…"}
          </p>
        {/if}
      </section>

      <!-- 2×2 — the day's digest prose + G11's one open thread. Its header is
           the Journal door (pages 08–10: destinations open from their tile). -->
      <section class="tile tile--w2 tile--h2 tile--prose">
        <div class="tile__h">
          <span class="label">Today</span>
          {#if data.digest && clockAt(data.digest.generatedAtMs)}
            <span class="meta mono">updated {clockAt(data.digest.generatedAtMs)}</span>
          {/if}
          <button type="button" class="tile__door" onclick={() => void goto("/journal")}>
            Open Journal <span aria-hidden="true">›</span>
          </button>
        </div>
        {#if data.digest}
          {#if data.digest.headline}
            <p class="digest__headline">{data.digest.headline}</p>
          {/if}
          <p class="digest__prose">{data.digest.narrative}</p>
          <p class="label label--section">Open threads</p>
          <p class="digest__thread">
            {data.openThread ?? "The digest didn't name an open thread today."}
          </p>
        {:else}
          <p class="tile__empty">
            No daily digest yet. One is written after User Context has distilled a
            day — nothing is generated by opening this tile.
          </p>
        {/if}
      </section>

      <!-- 1×1 — capture. The page's single --t-display. -->
      <section class="tile tile--cap">
        <div class="tile__h">
          <span class="label">Capture</span>
          <span
            class="dot"
            class:dot--off={!captureControls.isCapturing || captureControls.paused}
          ></span>
        </div>
        {#if hero}
          <div class="hero">
            <span class="hero__n">{hero}</span>
            <span class="meta">hours today</span>
          </div>
        {:else}
          <p class="tile__empty">Nothing captured yet today.</p>
        {/if}
        <div class="tile__row tile__row--foot">
          <span class="srcs">
            {#each sources as source (source.key)}
              <span class="srcs__i srcs__i--{source.key}" class:srcs__i--off={!source.on}
                title={source.label}
              >
                {#if source.key === "screen"}
                  <svg viewBox="0 0 24 24" aria-hidden="true"
                    ><rect x="2.5" y="3.5" width="19" height="13.5" rx="2.2" /><path
                      d="M8.5 21h7M12 17v4"
                    /></svg
                  >
                {:else if source.key === "mic"}
                  <svg viewBox="0 0 24 24" aria-hidden="true"
                    ><rect x="9" y="2.5" width="6" height="11" rx="3" /><path
                      d="M5.5 10.5v1.5a6.5 6.5 0 0 0 13 0v-1.5M12 19v2.5"
                    /></svg
                  >
                {:else}
                  <svg viewBox="0 0 24 24" aria-hidden="true"
                    ><path d="M11 5.2 6.4 9H2.8v6h3.6L11 18.8z" /><path
                      d="M15.4 9a4.4 4.4 0 0 1 0 6M18.6 5.6a8.6 8.6 0 0 1 0 12.8"
                    /></svg
                  >
                {/if}
              </span>
            {/each}
          </span>
          <span class="meta">
            {#if activeSourceCount === 3}
              all three sources
            {:else if activeSourceCount === 0}
              no sources on
            {:else}
              {activeSourceCount} of 3 sources
            {/if}
          </span>
        </div>
      </section>

      <!-- 1×1 — this week (G11). Seven local days of `list_day_coverage`. -->
      <section class="tile tile--week">
        <div class="tile__h">
          <span class="label">This week</span>
          {#if weekTotalMs > 0}
            <span class="meta mono">{formatCapturedHours(weekTotalMs)}</span>
          {/if}
        </div>
        {#if weekTotalMs > 0}
          <div class="spark" aria-hidden="true">
            {#each data.week as day (day.key)}
              <span
                class="spark__b"
                class:spark__b--on={day.isToday}
                style="height:{Math.max(3, Math.round((day.coveredMs / weekPeakMs) * 100))}%"
              ></span>
            {/each}
          </div>
          <div class="spark__l">
            {#each data.week as day (day.key)}
              <span class="label" class:label--now={day.isToday}>{day.label}</span>
            {/each}
          </div>
        {:else}
          <p class="tile__empty">No capture in the last seven days.</p>
        {/if}
      </section>

      <!-- 2×1 — conversations. -->
      <section class="tile tile--conv tile--w2">
        <div class="tile__h">
          <span class="label">Conversations</span>
          {#if data.conversations.length > 0}
            <span class="meta">{data.conversations.length} today</span>
          {/if}
        </div>
        {#if data.conversations.length > 0}
          {#each data.conversations.slice(0, 2) as cluster (cluster.activityId)}
            <div class="row">
              <span class="row__txt">
                <span class="row__lbl">{cluster.title}</span>
                <span class="row__sub"
                  >{spokenLabel(cluster.spokenMs)} · {cluster.speakerCount}
                  {cluster.speakerCount === 1 ? "speaker" : "speakers"}</span
                >
              </span>
              <span class="meta mono">{clockAt(cluster.startedAtMs)}</span>
            </div>
          {/each}
        {:else}
          <p class="tile__empty">
            No speech yet today. A conversation appears once two minutes of speech
            land inside one activity.
          </p>
        {/if}
      </section>

      <!-- 2×1 — ask history (G11). Straight off the conversation store. -->
      <section class="tile tile--ask tile--w2">
        <div class="tile__h">
          <span class="label">Recent asks</span>
        </div>
        {#if data.asks.length > 0}
          {#each data.asks.slice(0, 2) as ask (ask.conversationId)}
            <button class="row row--btn" type="button" onclick={() => openAsk(ask.conversationId)}>
              <span class="row__txt">
                <span class="row__lbl">{ask.title || ask.preview || "Untitled"}</span>
                <span class="row__sub"
                  >{ask.turnCount}
                  {ask.turnCount === 1 ? "turn" : "turns"} · {ask.origin === "quick_recall"
                    ? "Quick Access"
                    : "Chat"}</span
                >
              </span>
              <span class="meta mono">{clockAt(ask.updatedAtMs)}</span>
            </button>
          {/each}
        {:else}
          <p class="tile__empty">You haven't asked Mnema anything yet.</p>
        {/if}
      </section>

      <!-- 2×1 — subjects (page 09's door). A client-side group-by over the
           dossier's conclusions: name + its top conclusion's confidence, and
           the active/fading split. Never an evidence count — the index never
           loads one. -->
      <section class="tile tile--subj tile--w2">
        <div class="tile__h">
          <span class="label">Subjects</span>
          {#if data.subjects.activeCount > 0}
            <span class="meta">
              {`${data.subjects.activeCount} active${data.subjects.fadingCount > 0 ? ` · ${data.subjects.fadingCount} fading` : ""}`}
            </span>
          {/if}
          <button type="button" class="tile__door" onclick={() => void goto("/subjects")}>
            Open Subjects <span aria-hidden="true">›</span>
          </button>
        </div>
        {#if data.subjects.rows.length > 0}
          {#each data.subjects.rows.slice(0, 2) as subject (subject.name)}
            <div class="row">
              <span class="row__txt">
                <span class="row__lbl">{subject.name}</span>
              </span>
              <span class="meta mono">{subject.topConfidence.toFixed(2)}</span>
            </div>
          {/each}
        {:else}
          <p class="tile__empty">
            No views formed yet. Subjects appear once conclusions have been
            distilled from your days.
          </p>
        {/if}
      </section>

      <!-- 4×1 — what Mnema concluded today (page 10's door). -->
      <section class="tile tile--ctx tile--w4">
        <div class="tile__h">
          <span class="label">Context</span>
          {#if data.conclusions.length > 0}
            <span class="meta">{data.conclusions.length} moved today</span>
          {/if}
          <button type="button" class="tile__door" onclick={() => void goto("/context")}>
            Open Context <span aria-hidden="true">›</span>
          </button>
        </div>
        {#if data.conclusions.length > 0}
          {#each data.conclusions.slice(0, 1) as conclusion (conclusion.id)}
            <div class="row">
              <span class="row__txt">
                <span class="row__lbl">{conclusion.subject}</span>
                <span class="row__sub">{conclusion.statement}</span>
              </span>
            </div>
          {/each}
        {:else}
          <p class="tile__empty">
            Nothing new about you today. Conclusions land after a distillation pass.
          </p>
        {/if}
        {#if data.dossierCount > 0}
          <div class="tile__row tile__row--foot">
            <span class="meta">
              {data.dossierCount}
              {data.dossierCount === 1 ? "conclusion" : "conclusions"} in your dossier
            </span>
          </div>
        {/if}
      </section>

      <!-- 4×1 short — a launcher, never an answer surface. -->
      <button class="tile tile--launch tile--w4" type="button" onclick={openQuickAccess}>
        <svg viewBox="0 0 24 24" aria-hidden="true"
          ><circle cx="11" cy="11" r="7.5" /><path d="m21 21-4.4-4.4" /></svg
        >
        <span class="launch__t">Ask about your day, or search what you saw…</span>
        <span class="meta launch__hint">opens Quick Access</span>
      </button>
    </div>
  </div>
</div>

<style>
  /* The page owns its own scroll region and pulls up under the sticky title bar,
     so the bento genuinely scrolls UNDER the chrome material rather than
     starting below it. No layout change — the negative margin is entirely this
     surface's, the bar keeps its own z-index and backdrop blur. */
  .ov {
    flex: 1 1 auto;
    min-height: 0;
    margin-top: calc(var(--h-titlebar) * -1);
    position: relative;
  }

  .ov__scroll {
    position: absolute;
    inset: 0;
    overflow-y: auto;
    padding: calc(var(--h-titlebar) + 14px) var(--gap-group) var(--gap-group);
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-hover) transparent;
  }

  .ov__head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    margin-bottom: var(--gap-group);
    flex-wrap: wrap;
  }

  .ov__title {
    margin: 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-title);
    line-height: var(--lh-title);
    letter-spacing: var(--ls-title);
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }

  .ov__sub {
    margin: 0;
    font-family: var(--app-font-mono);
    font-size: var(--t-meta);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }

  /* ── the widget grid: one unit, one gutter, four footprints ──────────────── */
  /* Rows are explicit because the tile set is: media 120, three widget rows of
     124, and the 44px launcher. Auto-flow places every tile from its footprint
     class alone — nothing here hard-codes a column. */
  .bento {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    grid-template-rows: 120px repeat(4, 124px) 44px;
    gap: var(--gap-group);
  }

  .tile--w2 {
    grid-column: span 2;
  }
  .tile--w4 {
    grid-column: 1 / -1;
  }
  .tile--h2 {
    grid-row: span 2;
  }

  /* ── the tile: opaque plate, floating, tinted by its subject ─────────────── */
  .tile {
    --tint: transparent;
    position: relative;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    padding: 14px;
    border: none;
    border-radius: var(--r-panel);
    background: var(--app-surface);
    box-shadow: var(--sh-tile);
    text-align: left;
    min-width: 0;
  }

  /* The subject glow: off the TOP edge, ≤26% of the source colour, sitting on
     an opaque plate so no text's contrast depends on it. */
  .tile::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 60%;
    pointer-events: none;
    background: radial-gradient(120% 100% at 50% 0%, var(--tint), transparent 72%);
    opacity: 0.55;
  }

  .tile > * {
    position: relative;
  }

  .tile--cap {
    --tint: color-mix(in srgb, var(--app-record) 26%, transparent);
  }
  .tile--conv {
    --tint: color-mix(in srgb, var(--app-source-mic) 26%, transparent);
  }
  .tile--ask {
    --tint: color-mix(in srgb, var(--app-accent) 22%, transparent);
  }
  .tile--ctx {
    --tint: color-mix(in srgb, var(--app-source-screen) 26%, transparent);
  }
  .tile--week {
    --tint: color-mix(in srgb, var(--app-info) 20%, transparent);
  }
  .tile--subj {
    --tint: color-mix(in srgb, var(--app-accent) 22%, transparent);
  }

  /* The destination door: an accent link in the tile header, pinned right.
     Pages 08–10 open from here; the tile itself keeps showing its read. */
  .tile__door {
    margin-left: auto;
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    padding: 2px 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    font-family: var(--app-font-sans);
    font-size: var(--t-meta);
    line-height: 1;
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .tile__door:hover {
    background: var(--app-accent-bg);
  }
  .tile__door:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .tile__h .meta + .tile__door {
    margin-left: 0;
  }

  .tile__h {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 2px;
  }
  .tile__h .meta {
    margin-left: auto;
  }
  .tile__h .dot {
    margin-left: auto;
  }

  .tile__row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 18px;
  }
  .tile__row--foot {
    margin-top: auto;
  }

  .tile__empty {
    margin: 2px 0 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    color: var(--app-text-subtle);
  }

  /* ── type roles, straight off the ramp ───────────────────────────────────── */
  .label {
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    line-height: var(--lh-label);
    letter-spacing: var(--ls-label);
    font-weight: var(--w-medium);
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .label--section {
    margin: 10px 0 2px;
  }
  .label--now {
    color: var(--app-accent);
  }

  .meta {
    font-family: var(--app-font-sans);
    font-size: var(--t-meta);
    line-height: var(--lh-meta);
    letter-spacing: var(--ls-meta);
    color: var(--app-text-muted);
  }
  .mono {
    font-family: var(--app-font-mono);
    font-variant-numeric: tabular-nums;
  }

  /* ── moments strip: padding zero, frames clipped by the tile radius ──────── */
  .tile--media {
    padding: 0;
  }

  .strip-h {
    position: absolute;
    left: 14px;
    top: 14px;
    z-index: 3;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 20px;
    padding: 0 8px;
    border-radius: var(--r-pill);
    background: rgba(12, 12, 16, 0.6);
    -webkit-backdrop-filter: blur(10px);
    backdrop-filter: blur(10px);
  }
  .strip-h .label,
  .strip-h svg {
    color: #fff;
  }
  .strip-h svg {
    width: 11px;
    height: 11px;
  }

  .strip {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: stretch;
    gap: 10px;
    padding: 10px 0 10px 10px;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
  }
  .strip::-webkit-scrollbar {
    display: none;
  }

  .strip__frame {
    position: relative;
    flex: 0 0 auto;
    width: 214px;
    padding: 0;
    border: none;
    border-radius: 9px;
    overflow: hidden;
    background: var(--app-surface-subtle);
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.22);
    cursor: default;
  }
  .strip__frame img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .strip__cap {
    position: absolute;
    left: 6px;
    bottom: 6px;
    max-width: calc(100% - 12px);
    padding: 2px 6px;
    border-radius: var(--r-sm);
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    font-variant-numeric: tabular-nums;
    color: #fff;
    background: rgba(12, 12, 16, 0.62);
    -webkit-backdrop-filter: blur(8px);
    backdrop-filter: blur(8px);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
  }
  .tile__empty--media {
    margin: 40px 14px 0;
  }

  /* ── digest ──────────────────────────────────────────────────────────────── */
  .digest__headline {
    margin: 2px 0 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-ui);
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
  }
  .digest__prose,
  .digest__thread {
    margin: 2px 0 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-read);
    line-height: var(--lh-read);
    letter-spacing: var(--ls-read);
    color: var(--app-text);
  }
  /* The prose is the tile's only elastic part: it takes the slack and clips
     itself, so the Open Threads block below it can never be squeezed away. */
  .digest__prose {
    flex: 1 1 auto;
    min-height: 0;
    overflow: hidden;
  }
  .digest__thread {
    flex: 0 0 auto;
    font-size: var(--t-ui);
    color: var(--app-text-muted);
  }

  /* ── capture ─────────────────────────────────────────────────────────────── */
  .hero {
    display: flex;
    align-items: baseline;
    gap: 6px;
  }
  .hero__n {
    font-family: var(--app-font-sans);
    font-size: var(--t-display);
    line-height: var(--lh-display);
    letter-spacing: var(--ls-display);
    font-weight: var(--w-semi);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-strong);
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    display: block;
    background: var(--app-record);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-record) 18%, transparent);
  }
  .dot--off {
    background: var(--app-text-subtle);
    box-shadow: none;
  }

  .srcs {
    display: inline-flex;
    gap: 5px;
  }
  .srcs__i {
    width: var(--o-badge);
    height: var(--o-badge);
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .srcs__i svg {
    width: 11px;
    height: 11px;
  }
  .srcs__i--screen {
    background: var(--app-source-screen-bg);
    color: var(--app-source-screen);
  }
  .srcs__i--mic {
    background: var(--app-source-mic-bg);
    color: var(--app-source-mic);
  }
  .srcs__i--sys {
    background: var(--app-source-sysaudio-bg);
    color: var(--app-source-sysaudio);
  }
  .srcs__i--off {
    background: var(--glass-tint);
    color: var(--app-text-faint);
  }

  /* ── this week ───────────────────────────────────────────────────────────── */
  .spark {
    display: flex;
    align-items: flex-end;
    gap: 5px;
    height: 38px;
    margin-top: auto;
  }
  .spark__b {
    flex: 1;
    border-radius: 2px 2px 0 0;
    background: var(--app-text-faint);
    opacity: 0.55;
    display: block;
  }
  .spark__b--on {
    background: var(--app-accent);
    opacity: 1;
  }
  .spark__l {
    display: flex;
    gap: 5px;
    margin-top: 4px;
  }
  .spark__l .label {
    flex: 1;
    text-align: center;
    color: var(--app-text-faint);
  }

  /* ── list rows (conversations / asks / context) ──────────────────────────── */
  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 34px;
    padding: 5px 2px;
    min-width: 0;
    background: none;
    border: none;
    width: 100%;
    text-align: left;
  }
  .row + .row {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .row--btn {
    cursor: default;
    border-radius: var(--r-sm);
  }
  .row--btn:hover {
    background: var(--app-surface-hover);
  }
  .row__txt {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }
  .row__lbl {
    font-family: var(--app-font-sans);
    font-size: var(--t-ui);
    line-height: 1.3;
    font-weight: var(--w-medium);
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row__sub {
    font-family: var(--app-font-sans);
    font-size: var(--t-meta);
    line-height: 1.35;
    color: var(--app-text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row .meta {
    flex: 0 0 auto;
  }

  /* ── ask launcher ────────────────────────────────────────────────────────── */
  .tile--launch {
    flex-direction: row;
    align-items: center;
    gap: 10px;
    padding: 0 12px;
    cursor: default;
  }
  .tile--launch:hover {
    background: var(--app-surface-hover);
  }
  .tile--launch svg {
    width: 15px;
    height: 15px;
    flex: 0 0 auto;
    color: var(--app-text-subtle);
  }
  .launch__t {
    font-family: var(--app-font-sans);
    font-size: var(--t-ui);
    color: var(--app-text-subtle);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .launch__hint {
    margin-left: auto;
    flex: 0 0 auto;
  }

  svg {
    display: block;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  /* ── the 800×600 floor: two columns; This week drops, the 6:42 hero stays ── */
  @media (max-width: 900px) {
    .bento {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-template-rows: 106px repeat(4, 118px) 44px;
      gap: 12px;
    }
    /* A 2×1 list collapses to a 1×1 — only the media strip and the prose tile
       still need the full width. */
    .tile--w2 {
      grid-column: span 1;
    }
    .tile--w4,
    .tile--prose {
      grid-column: 1 / -1;
    }
    .tile--h2 {
      grid-row: span 1;
    }
    .tile--week {
      display: none;
    }
    .digest__headline,
    .digest__thread,
    .tile--prose .label--section {
      display: none;
    }
    .ov__scroll {
      padding: calc(var(--h-titlebar) + 8px) 12px 12px;
    }
  }
</style>
