<!-- Quick Recall detail pane (right side of the search-mode split, mockup
     `.detail`, `#sel=N` states). Renders the SELECTED result as a preview:
     screen → hero frame (1280px preview), id row, badges, clickable captured
     URL, times, scrollable OCR context with residual-term highlights;
     audio → source identity, badges, times, waveform with the REAL
     match-position marker, scrollable transcript anchored to the match turn.
     Data comes lazily from the detail store (cached per result); identity/
     badges/times render straight off the search result, so a missing context
     never blanks the pane. -->
<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { formatRelativeTime, formatTimestampCompact } from "$lib/format-time";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { quickRecallDetail as detail, type DetailData } from "./detailStore.svelte";
  import { appIcons } from "./app-icons.svelte";
  import { highlightSegments, residualTerms } from "./context-highlight";
  import {
    appIconColor,
    appIconLabel,
    capturedRangeLabel,
    detailCacheKey,
    formatDuration,
    formatTurnClock,
    matchFraction,
    matchTurnIndex,
    segmentDurationMs,
    waveBars,
    waveMarkerX,
  } from "./detail-view";
  import type { HighlightSegment } from "./context-highlight";

  let { dim = false }: { dim?: boolean } = $props();

  const sel = $derived(search.selectedResult);

  // Fetch on selection change only; the store early-returns on a same-key
  // call and serves cache hits synchronously.
  $effect(() => {
    const selected = search.selectedResult;
    if (selected === null) {
      detail.clear();
    } else {
      void detail.load(selected);
    }
  });

  // Only render fetched data that belongs to the CURRENT selection — during
  // the one render before the load effect fires, detail.key still names the
  // previous result, and stale context under a new identity would mislead.
  const wantKey = $derived(sel !== null ? detailCacheKey(sel) : null);
  const current = $derived<DetailData | null>(
    wantKey !== null && detail.key === wantKey ? detail.data : null,
  );
  const contextLoading = $derived(sel !== null && current === null);

  const frame = $derived(sel?.kind === "frame" ? sel.frame : null);
  const audio = $derived(sel?.kind === "audio" ? sel.audio : null);
  const frameData = $derived(current?.kind === "frame" ? current : null);
  const audioData = $derived(current?.kind === "audio" ? current : null);

  const terms = $derived(residualTerms(search.residualQuery));

  // Fade the hero in once decoded (same pattern as the row thumbnails);
  // reset whenever the source changes so a new hero re-fades.
  let heroLoaded = $state(false);
  $effect(() => {
    frameData?.heroUrl;
    heroLoaded = false;
  });

  const ocrSegments = $derived<HighlightSegment[] | null>(
    frameData?.ocrText != null
      ? highlightSegments(frameData.ocrText, terms)
      : null,
  );

  const frameRange = $derived(
    frame !== null
      ? capturedRangeLabel(frame.groupStartAt, frame.groupEndAt)
      : null,
  );

  type TurnView = {
    id: number;
    clock: string;
    label: string;
    segments: HighlightSegment[];
    match: boolean;
  };
  const turnViews = $derived.by<TurnView[]>(() => {
    if (audio === null || audioData === null) return [];
    const spoken = audioData.turns.filter(
      (turn) => (turn.transcriptText ?? "").trim().length > 0,
    );
    const matchIndex = matchTurnIndex(spoken, audio.spanStartMs);
    return spoken.map((turn, index) => ({
      id: turn.id,
      clock: formatTurnClock(audio.audioSegment.startedAt, turn.startMs),
      label: turn.speakerLabel,
      segments: highlightSegments(turn.transcriptText ?? "", terms),
      match: index === matchIndex,
    }));
  });

  const audioDurationMs = $derived(
    audio !== null ? segmentDurationMs(audio.audioSegment) : 0,
  );
  const audioMatchFrac = $derived(
    audio !== null ? matchFraction(audio.spanStartMs, audioDurationMs) : 0,
  );
  const audioWave = $derived(
    audio !== null ? waveBars(audio.groupKey, audioMatchFrac) : [],
  );

  // Scroll the context to the match: the match turn (audio) or the first
  // highlighted term (OCR), parked a third down so surrounding text shows.
  let ctxEl = $state<HTMLElement | null>(null);
  $effect(() => {
    current;
    terms;
    const el = ctxEl;
    if (el === null) return;
    const target =
      el.querySelector<HTMLElement>('[data-scroll-target="true"]') ??
      el.querySelector<HTMLElement>("mark");
    el.scrollTop =
      target !== null ? Math.max(0, target.offsetTop - el.clientHeight / 3) : 0;
  });
</script>

<section
  class="quick-recall__detail"
  class:quick-recall__detail--dim={dim}
  aria-label="Result detail"
>
  {#if dim}
    <!-- First-search skeleton: mirrors the mockup's dimmed detail pane
         (hero block + two lines) so loading reads as one surface. -->
    <div aria-hidden="true">
      <div class="quick-recall__detail-sk quick-recall__detail-sk--hero"></div>
      <div class="quick-recall__detail-sk" style="width: 60%"></div>
      <div class="quick-recall__detail-sk" style="width: 40%"></div>
    </div>
  {:else if sel === null}
    <p class="quick-recall__detail-placeholder">Select a result to preview</p>
  {:else if frame !== null}
    <!-- ── Screen result ─────────────────────────────────────────────── -->
    <div class="qr-detail__hero">
      <svg
        class="qr-detail__hero-glyph"
        width="34"
        height="34"
        viewBox="0 0 14 14"
        fill="none"
        stroke="currentColor"
        stroke-width="1.4"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <rect x="1.5" y="2" width="11" height="8" rx="1.5" />
        <path d="M4 12h6" />
        <path d="M7 10v2" />
      </svg>
      {#if frameData?.heroUrl}
        <img
          class="qr-detail__hero-img"
          class:qr-detail__hero-img--loaded={heroLoaded}
          src={frameData.heroUrl}
          alt=""
          onload={() => (heroLoaded = true)}
        />
      {/if}
    </div>
    <div class="qr-detail__id-row">
      {#if appIcons.src(frame.appBundleId ?? frame.appName) !== null}
        <img
          class="qr-detail__appicon qr-detail__appicon--img"
          src={appIcons.src(frame.appBundleId ?? frame.appName)}
          alt=""
          aria-hidden="true"
        />
      {:else}
        <span
          class="qr-detail__appicon"
          style:background={appIconColor(frame.appName)}
          aria-hidden="true">{appIconLabel(frame.appName)}</span
        >
      {/if}
      <span class="qr-detail__app-name">{frame.appName ?? "Unknown app"}</span>
      {#if frame.windowTitle}
        <span class="qr-detail__win-title" use:tip={frame.windowTitle}
          >{frame.windowTitle}</span
        >
      {/if}
    </div>
    <div class="qr-detail__badge-row">
      <span class="qr-detail__mchip"
        >{frame.matchCount} {frame.matchCount === 1 ? "match" : "matches"}</span
      >
      {#if frame.foundByMeaning}
        <span class="qr-detail__pill qr-detail__pill--meaning"
          >found by meaning</span
        >
      {/if}
      {#if frame.hasSecretRedactions}
        <span class="qr-detail__pill qr-detail__pill--redacted">redacted</span>
      {/if}
    </div>
    {#if frame.url !== null}
      <button
        type="button"
        class="qr-detail__url-row"
        disabled={search.openingCapturedUrl}
        onclick={() => void search.openCapturedFrameUrl(frame.thumbnailFrameId)}
        use:tip={"Open captured page (⌘O)"}
      >
        <span class="qr-detail__url-proto">https://</span>{frame.url}
      </button>
    {/if}
    <div class="qr-detail__time-row">
      <span class="qr-detail__time-rel"
        >{formatRelativeTime(frame.groupEndAt)}</span
      >
      <span>{formatTimestampCompact(frame.representativeFrame.capturedAt)}</span>
      {#if frameRange !== null}
        <span>{frameRange}</span>
      {/if}
    </div>
    <div class="qr-detail__ctx-label">
      <span>OCR context</span>
      <span class="qr-detail__scroll-hint">scrolls · matches highlighted</span>
    </div>
    <div class="qr-detail__ctx" bind:this={ctxEl}>
      {#if ocrSegments !== null}
        {#each ocrSegments as segment, i (i)}{#if segment.marked}<mark
              >{segment.text}</mark
            >{:else}{segment.text}{/if}{/each}
      {:else if contextLoading}
        <span class="qr-detail__ctx-note">Loading context…</span>
      {:else}
        <span class="qr-detail__ctx-note"
          >No OCR text captured for this moment.</span
        >
      {/if}
    </div>
  {:else if audio !== null}
    <!-- ── Audio result ──────────────────────────────────────────────── -->
    <div class="qr-detail__id-row">
      <span
        class="qr-detail__srcicon"
        class:qr-detail__srcicon--mic={audio.sourceKind === "microphone"}
        class:qr-detail__srcicon--sys={audio.sourceKind !== "microphone"}
        aria-hidden="true"
      >
        <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
          <rect x="1" y="4" width="2" height="4" rx="1" />
          <rect x="5" y="2" width="2" height="8" rx="1" />
          <rect x="9" y="4.5" width="2" height="3" rx="1" />
        </svg>
      </span>
      <span class="qr-detail__app-name"
        >{audio.sourceKind === "microphone" ? "Microphone" : "System audio"}</span
      >
    </div>
    <div class="qr-detail__badge-row">
      <span class="qr-detail__mchip">{audio.matchCount} adjacent</span>
      <span
        class="qr-detail__pill"
        class:qr-detail__pill--mic={audio.sourceKind === "microphone"}
        class:qr-detail__pill--sys={audio.sourceKind !== "microphone"}
        >{audio.sourceKind === "microphone" ? "microphone" : "system audio"}</span
      >
      {#if audio.foundByMeaning}
        <span class="qr-detail__pill qr-detail__pill--meaning"
          >found by meaning</span
        >
      {/if}
      {#if audio.hasSecretRedactions}
        <span class="qr-detail__pill qr-detail__pill--redacted">redacted</span>
      {/if}
    </div>
    <div class="qr-detail__time-row">
      <span class="qr-detail__time-rel"
        >{formatRelativeTime(audio.absoluteStartAt)}</span
      >
      <span>{formatTimestampCompact(audio.absoluteStartAt)}</span>
      <span>duration {formatDuration(audioDurationMs / 1000)}</span>
    </div>
    <div class="qr-detail__wavebox">
      <svg
        class="qr-detail__wave"
        viewBox="0 0 448 20"
        preserveAspectRatio="none"
        aria-hidden="true"
      >
        {#each audioWave as bar (bar.x)}
          <rect
            class={bar.on ? "wb-on" : "wb"}
            x={bar.x}
            y={bar.y}
            width="4"
            height={bar.h}
            rx="1"
          />
        {/each}
        <line
          class="wmark"
          x1={waveMarkerX(audioMatchFrac)}
          y1="0"
          x2={waveMarkerX(audioMatchFrac)}
          y2="20"
        />
      </svg>
      <div class="qr-detail__wavemeta">
        <span>0:00</span>
        <span class="qr-detail__wave-at"
          >match at {formatDuration(audio.spanStartMs / 1000)}</span
        >
        <span>{formatDuration(audioDurationMs / 1000)}</span>
      </div>
    </div>
    <div class="qr-detail__ctx-label">
      <span>Transcript</span>
      <span class="qr-detail__scroll-hint">scrolls · matches highlighted</span>
    </div>
    <div class="qr-detail__ctx qr-detail__ctx--turns" bind:this={ctxEl}>
      {#if turnViews.length > 0}
        {#each turnViews as turn (turn.id)}
          <p
            class="qr-detail__turn"
            class:qr-detail__turn--match={turn.match}
            data-scroll-target={turn.match ? "true" : undefined}
          >
            <span class="qr-detail__turn-clock">[{turn.clock}]</span>
            <span class="qr-detail__turn-label">{turn.label}:</span>
            {#each turn.segments as segment, i (i)}{#if segment.marked}<mark
                  >{segment.text}</mark
                >{:else}{segment.text}{/if}{/each}
          </p>
        {/each}
      {:else if contextLoading}
        <span class="qr-detail__ctx-note">Loading transcript…</span>
      {:else}
        <span class="qr-detail__ctx-note"
          >No transcript available for this segment.</span
        >
      {/if}
    </div>
  {/if}
</section>

<style>
  .quick-recall__detail {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    padding: 16px 20px;
    overflow: hidden;
    background: var(--app-surface);
  }

  /* Dimmed while the first search is in flight (mockup `.sp-detail.dim`). */
  .quick-recall__detail--dim {
    opacity: 0.35;
  }

  .quick-recall__detail-sk {
    height: 9px;
    border-radius: 5px;
    background: var(--app-surface-raised);
    margin-top: 8px;
  }

  .quick-recall__detail-sk--hero {
    height: 96px;
    border-radius: 7px;
    margin-top: 0;
    border: 1px solid var(--app-border);
  }

  .quick-recall__detail-placeholder {
    margin: auto;
    font-size: var(--text-sm);
    color: var(--app-text-faint, var(--app-text-subtle));
    text-align: center;
  }

  /* ── Hero frame (mockup `.detail .hero`). A fixed 16:10 box (the mockup's
     320×200 hero ratio) keeps the pane stable across loads; the backing stays
     dark in both themes (it holds a screenshot) with a centered glyph while
     loading or when the preview is missing (`missingReason` fallback). */
  .qr-detail__hero {
    position: relative;
    flex: none;
    aspect-ratio: 16 / 10;
    display: grid;
    place-items: center;
    border: 1px solid var(--app-border-strong);
    border-radius: 7px;
    overflow: hidden;
    background: #101014;
    color: #6a6a74;
  }

  .qr-detail__hero-img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
  }

  .qr-detail__hero-img--loaded {
    opacity: 1;
  }

  @media (prefers-reduced-motion: no-preference) {
    .qr-detail__hero-img {
      transition: opacity 0.18s ease;
    }
  }

  /* ── Identity row (mockup `.detail .id-row`). */
  .qr-detail__id-row {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 12px;
    min-width: 0;
  }

  .qr-detail__appicon {
    flex: none;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 9px;
    font-weight: 700;
    color: #fff;
  }

  /* Real app icon (resolved from the OS); macOS icons carry their own shape,
     so no background/radius box. */
  .qr-detail__appicon--img {
    object-fit: contain;
    border-radius: 0;
  }

  .qr-detail__srcicon {
    flex: none;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 1px solid;
  }

  .qr-detail__srcicon--mic {
    color: var(--app-source-mic);
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
  }

  .qr-detail__srcicon--sys {
    color: var(--app-source-sysaudio);
    background: var(--app-source-sysaudio-bg);
    border-color: var(--app-source-sysaudio-border);
  }

  .qr-detail__app-name {
    flex: none;
    font-size: 13px;
    color: var(--app-text-strong);
    font-weight: 600;
    white-space: nowrap;
  }

  .qr-detail__win-title {
    font-size: 12px;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  /* ── Badges (mockup `.detail .badge-row`). */
  .qr-detail__badge-row {
    flex: none;
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-top: 8px;
    align-items: center;
  }

  .qr-detail__mchip {
    font-size: 10px;
    line-height: 1;
    color: var(--app-accent);
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    border-radius: 4px;
    padding: 3px 6px;
  }

  .qr-detail__pill {
    display: inline-flex;
    align-items: center;
    font-size: 10px;
    line-height: 1;
    padding: 3px 6px;
    border-radius: 4px;
    border: 1px solid;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }

  .qr-detail__pill--meaning {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .qr-detail__pill--redacted {
    color: var(--app-warn);
    background: var(--app-warn-bg);
    border-color: var(--app-warn-border);
  }

  .qr-detail__pill--mic {
    color: var(--app-source-mic);
    background: var(--app-source-mic-bg);
    border-color: var(--app-source-mic-border);
  }

  .qr-detail__pill--sys {
    color: var(--app-source-sysaudio);
    background: var(--app-source-sysaudio-bg);
    border-color: var(--app-source-sysaudio-border);
  }

  /* ── Captured URL (mockup `.detail .url-row`) — the one URL surface;
     clickable via the brokered open-captured-url path (⌘O twin). */
  .qr-detail__url-row {
    flex: none;
    display: block;
    width: 100%;
    min-width: 0;
    margin-top: 8px;
    padding: 0;
    font: inherit;
    font-size: 12px;
    text-align: left;
    color: var(--app-accent);
    background: none;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .qr-detail__url-row:hover {
    text-decoration: underline;
  }

  .qr-detail__url-row:disabled {
    opacity: 0.6;
    cursor: default;
    text-decoration: none;
  }

  .qr-detail__url-proto {
    color: var(--app-text-subtle);
  }

  /* ── Times (mockup `.detail .time-row`). */
  .qr-detail__time-row {
    flex: none;
    margin-top: 6px;
    font-size: 11px;
    color: var(--app-text-subtle);
    display: flex;
    gap: 12px;
    flex-wrap: wrap;
  }

  .qr-detail__time-rel {
    color: var(--app-text-muted);
  }

  /* ── Waveform (mockup `.wavebox`): real match-position marker. */
  .qr-detail__wavebox {
    flex: none;
    margin-top: 12px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-raised);
    padding: 8px 12px 5px;
  }

  .qr-detail__wave {
    width: 100%;
    height: 26px;
    display: block;
  }

  .qr-detail__wave .wb {
    fill: var(--app-text-faint);
  }

  .qr-detail__wave .wb-on {
    fill: var(--app-accent);
  }

  .qr-detail__wave .wmark {
    stroke: var(--app-accent);
    stroke-width: 1.5;
  }

  .qr-detail__wavemeta {
    display: flex;
    justify-content: space-between;
    font-size: 10px;
    color: var(--app-text-subtle);
    margin-top: 4px;
  }

  .qr-detail__wave-at {
    color: var(--app-accent);
  }

  /* ── Context (mockup `.detail .ctx-label` / `.ctx`). The scroll area takes
     the remaining height via flex (WKWebView collapses height:100% against
     flex-stretched parents — flex: 1 1 auto + min-height: 0 instead). */
  .qr-detail__ctx-label {
    flex: none;
    margin: 12px 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--app-text-subtle);
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .qr-detail__scroll-hint {
    font-size: 10px;
    color: var(--app-text-subtle);
    text-transform: none;
    letter-spacing: 0;
  }

  .qr-detail__ctx {
    position: relative; /* offsetTop anchor for the scroll-to-match effect */
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-raised);
    padding: 10px 12px;
    font-size: 11px;
    line-height: 1.65;
    color: var(--app-text);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .qr-detail__ctx--turns {
    white-space: normal;
  }

  .qr-detail__ctx-note {
    color: var(--app-text-subtle);
  }

  .qr-detail__ctx mark {
    border-radius: 2px;
    background: color-mix(in srgb, var(--app-accent) 26%, transparent);
    color: var(--app-text-strong);
    padding: 0 1px;
  }

  .qr-detail__turn {
    margin: 0 0 10px;
  }

  .qr-detail__turn--match {
    border-left: 2px solid var(--app-accent);
    padding-left: 8px;
    margin-left: -10px;
  }

  .qr-detail__turn-clock {
    color: var(--app-text-subtle);
  }

  .qr-detail__turn-label {
    color: var(--app-text-strong);
    font-weight: 600;
  }
</style>
