<!-- Quick Look (recall.html `.rc-look`): opens under the bar, same surface,
     a fixed 476px tall so scrubbing never resizes the window (the page passes
     BarShell cap 570). Screen: the frame with its matches outlined (solid =
     keyword, dashed = by meaning), a 7-frame strip 15 s apart, the frame's
     text, Open this frame / Copy text / Page. Audio: the line set large over
     a zoomed waveform, the transcript turns as the strip, the transcript,
     "On screen then". State + keys: detailStore.svelte.ts / search-keys.ts. -->
<script module lang="ts">
  export { quickLook } from "./detailStore.svelte";
</script>

<script lang="ts">
  import IconMissing from "~icons/lucide/image-off";
  import { FramePreviewUrlHolder } from "$lib/frame-preview";
  import { invoke } from "@tauri-apps/api/core";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import { quickLook as look } from "./detailStore.svelte";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import QuickLookSide, { marked } from "./QuickLookSide.svelte";
  import { highlightSegments, residualTerms } from "./context-highlight";
  import { matchTurnIndex, segmentDurationMs } from "./detail-view";
  import { clock, missingCopy, RANGE, signedClock, STEP_MS, stripWindow } from "./quick-look";

  const r = $derived(look.result!);
  const frame = $derived(r.kind === "frame" ? r.frame : null);
  const audio = $derived(r.kind === "audio" ? r.audio : null);
  const terms = $derived(residualTerms(search.residualQuery));
  const meaning = $derived(frame ? frame.foundByMeaning : audio!.foundByMeaning);

  // Close when the query moves on (typing, a cleared window).
  const openedFor = search.resultsQuery;
  $effect(() => {
    if (search.resultsQuery !== openedFor) look.close();
  });

  const hms = (d: Date) =>
    d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });

  // ── Screen stage: one full-size preview alive at a time ─────────────────
  const holder = new FramePreviewUrlHolder();
  let stageUrl = $state<string | null>(null);
  let natural = $state<{ w: number; h: number } | null>(null);
  let stageW = $state(0);
  let stageH = $state(0);
  $effect(() => {
    const id = look.previewId;
    if (id === null || !frame) return;
    void invoke<FrameScrubPreviewsDto>("get_frame_scrub_previews", {
      request: { frameIds: [id], maxPixelSize: 1280 },
    })
      .then(async ({ previews }) => {
        const p = previews.find((x) => x.frameId === id);
        if (p?.missingReason) look.gone[id] = p.missingReason;
        if (!p?.preview || look.previewId !== id) return;
        const url = await holder.swap(p.preview.filePath, p.preview.mimeType);
        if (url) stageUrl = url;
      })
      .catch(() => {});
  });
  $effect(() => () => holder.clear());

  const gone = $derived(look.previewId === null ? undefined : look.gone[look.previewId]);
  const fit = $derived.by(() => {
    const nw = natural?.w ?? frame?.representativeFrame.width ?? 16;
    const nh = natural?.h ?? frame?.representativeFrame.height ?? 10;
    const top = 40;
    const W = stageW - 28;
    const H = stageH - top - 12;
    const z = Math.max(0, Math.min(W / nw, H / nh));
    return { w: nw * z, h: nh * z, left: (stageW - nw * z) / 2, top: top + (H - nh * z) / 2 };
  });
  const frameMs = $derived(look.frame?.ms ?? 0);
  const offsetMs = $derived(look.k === 0 ? 0 : frameMs - (look.strip[RANGE]?.ms ?? frameMs));
  const boxes = $derived(look.frame ? (look.boxes[look.frame.id] ?? []) : []);
  const screenCells = $derived(stripWindow(look.k, -RANGE, RANGE));

  // ── Audio ───────────────────────────────────────────────────────────────
  const turns = $derived(look.turns ?? []);
  const matchK = $derived(audio ? Math.max(0, matchTurnIndex(turns, audio.spanStartMs)) : 0);
  const segmentMs = $derived(audio ? segmentDurationMs(audio.audioSegment) : 0);
  const turnCells = $derived(turns.length ? stripWindow(look.k, 0, turns.length - 1, 4) : []);
  const audioTitle = $derived(
    audio?.alignedFrame?.windowTitle ??
      audio?.alignedFrame?.appName ??
      (audio?.sourceKind === "microphone" ? "Microphone" : "System audio"),
  );
  // ponytail: bar heights are seeded from the group key, not real peaks (as
  // BestMoment's crop); `get_audio_segment_waveform_peaks` exists if it must be true.
  const wave = $derived.by(() => {
    const cur = look.turn;
    if (!audio || !cur || !turnCells.length) return null;
    const from = Math.max(0, turns[turnCells[0]].startMs - 20_000);
    const end = turns[turnCells[turnCells.length - 1]].endMs + 20_000;
    // Never run the axis past the recording's own end ("1:20" of a 1:00 segment).
    const to = Math.max(from + 1, segmentMs > 0 ? Math.min(segmentMs, end) : end);
    let s = 0;
    for (const ch of audio.groupKey) s = (s * 31 + ch.charCodeAt(0)) >>> 0;
    s = (s % 2147483646) + 1;
    const bars = Array.from({ length: 120 }, (_, i) => {
      s = (s * 16807) % 2147483647;
      const at = from + ((i + 0.5) / 120) * (to - from);
      return { v: 14 + (s % 80), on: at >= cur.startMs && at <= cur.endMs };
    });
    const x = (ms: number) => `${(((ms - from) / (to - from)) * 100).toFixed(2)}%`;
    return { from, to, bars, match: x(turns[matchK].startMs), cur: x(cur.startMs) };
  });
</script>

<div class="rc-look">
  <div class="rc-look__main">
    <div class="rc-look__stage" bind:clientWidth={stageW} bind:clientHeight={stageH}>
      {#if frame}
        <span class="rc-look__tag"
          >{#if look.k === 0}<i></i><b>matched frame</b>{:else}<b>{signedClock(offsetMs)}</b
            >{offsetMs >= 0 ? "after" : "before"} the match{/if}<span>· {hms(new Date(frameMs))}</span></span
        >
        <span class="rc-look__legend"
          >{#if boxes.some((b) => b.kind === "keyword")}<span class="kw">keyword</span>{/if}{#if meaning}<span
              class="mn">meaning</span
            >{/if}</span
        >
        {#if gone}
          {@const copy = missingCopy(gone)}
          <div class="mx-empty mx-empty--center" data-tone="warn">
            <span class="mx-empty__glyph"><IconMissing width="18" height="18" aria-hidden="true" /></span>
            <h4 class="mx-empty__title">{copy.title}</h4>
            <p class="mx-empty__text">{copy.text}</p>
          </div>
        {:else if stageUrl}
          <div
            class="rc-look__frame"
            style:width="{fit.w}px"
            style:height="{fit.h}px"
            style:left="{fit.left}px"
            style:top="{fit.top}px"
          >
            <img
              src={stageUrl}
              alt=""
              onload={(event) => {
                const img = event.currentTarget as HTMLImageElement;
                natural = { w: img.naturalWidth, h: img.naturalHeight };
                holder.settle();
              }}
            />
            {#each boxes as box, i (i)}
              <span
                class="rc-box"
                data-kind={box.kind}
                style:left="{box.x * 100}%"
                style:top="{box.y * 100}%"
                style:width="{box.w * 100}%"
                style:height="{box.h * 100}%"
              ></span>
            {/each}
          </div>
        {/if}
      {:else if audio && look.turn}
        {@const t = look.turn}
        <span class="rc-look__tag"
          >{#if look.k === matchK}<i></i><b>matched line</b>{:else}<b>{look.k > matchK ? "after" : "before"}</b
            >the match{/if}<span>· {clock(t.startMs)} into the recording</span></span
        >
        <div class="rc-qa" data-src={audio.sourceKind === "microphone" ? "mic" : "sys"}>
          {#if t.speakerLabel}
            <div class="rc-qa__who">
              <i class="rc-spk">{t.speakerLabel.charAt(0).toUpperCase()}</i><b>{t.speakerLabel}</b><span
                >in {audioTitle}</span
              >
            </div>
          {/if}
          <blockquote>{@render marked(highlightSegments(t.text, terms))}</blockquote>
          {#if wave}
            <div class="rc-qa__wave" aria-hidden="true">
              {#each wave.bars as bar, i (i)}<i class:on={bar.on} style:--v={bar.v}></i>{/each}
              <b data-match style:--x={wave.match}></b>
              {#if look.k !== matchK}<b style:--x={wave.cur}></b>{/if}
            </div>
            <div class="rc-qa__axis">
              <span>{clock(wave.from)}</span><span>of {clock(segmentMs)}</span><span>{clock(wave.to)}</span>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="rc-strip" aria-label={frame ? "Frames 15 s apart" : "Transcript lines"}>
      {#if frame}
        {#each screenCells as j (j)}
          {@const f = look.strip[j + RANGE]}
          {@const px = f ? look.pixelId(f) : null}
          {@const off = f ? f.ms - (look.strip[RANGE]?.ms ?? f.ms) : j * STEP_MS}
          <button
            type="button"
            class="rc-film"
            tabindex="-1"
            aria-current={j === look.k}
            data-match={j === 0 || undefined}
            aria-label={j === 0 ? "Matched frame" : `${signedClock(off)} from the match`}
            onclick={() => {
              look.goTo(j);
              search.inputEl?.focus();
            }}
          >
            {#if px !== null && look.gone[px]}
              <span class="rc-film__img rc-film__img--gone"><IconMissing width="13" height="13" aria-hidden="true" /></span>
            {:else}
              <span class="rc-film__img">{#if px !== null && look.thumbs[px]}<img src={look.thumbs[px]} alt="" />{/if}</span>
            {/if}
            <small>{j === 0 ? "match" : signedClock(off)}</small>
          </button>
        {/each}
      {:else}
        {#each turnCells as j (j)}
          <button
            type="button"
            class="rc-film rc-film--turn"
            tabindex="-1"
            aria-current={j === look.k}
            data-match={j === matchK || undefined}
            onclick={() => {
              look.goTo(j);
              search.inputEl?.focus();
            }}
          >
            <span class="rc-film__txt"
              >{#if turns[j].speakerLabel}<b>{turns[j].speakerLabel}</b>{/if} {turns[j].text}</span
            >
            <small>{clock(turns[j].startMs)}</small>
          </button>
        {/each}
      {/if}
    </div>
  </div>

  <QuickLookSide {gone} {audioTitle} />
</div>

<style>
  /* Fills BarShell's body edge to edge (it pads 8 8 10). Fixed height:
     scrubbing never resizes the window. */
  .rc-look {
    flex: none;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 256px;
    height: 476px;
    margin: -8px -8px -10px;
  }
  .rc-look__main {
    min-width: 0;
    min-height: 0;
    display: grid;
    grid-template-rows: minmax(0, 1fr) auto;
  }
  .rc-look__stage {
    position: relative;
    overflow: hidden;
    background: color-mix(in srgb, var(--app-surface-subtle) 60%, transparent);
  }
  .rc-look__stage > .mx-empty {
    position: absolute;
    inset: 40px 0 0;
    align-content: center;
  }
  .rc-look__frame {
    position: absolute;
    overflow: hidden;
    border-radius: var(--r-md);
    box-shadow: 0 0 0 1px var(--app-border-strong);
    animation: mx-rise var(--t-slow) var(--ease-expo) both;
  }
  .rc-look__frame img {
    display: block;
    width: 100%;
    height: 100%;
  }
  .rc-box {
    position: absolute;
    border-radius: 3px;
    background: color-mix(in srgb, var(--app-accent) 12%, transparent);
    box-shadow: inset 0 0 0 1.5px var(--app-accent);
  }
  .rc-box[data-kind="meaning"] {
    background: color-mix(in srgb, var(--app-fg) 7%, transparent);
    box-shadow: none;
    border: 1px dashed var(--app-text-subtle);
  }
  .rc-look__tag,
  .rc-look__legend {
    position: absolute;
    z-index: 3;
    top: 10px;
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 22px;
    padding: 0 8px;
    border-radius: var(--r-sm);
    background: var(--app-overlay-bg-strong);
    border: 1px solid var(--app-overlay-border);
    white-space: nowrap;
  }
  .rc-look__tag {
    left: 12px;
    font: 450 var(--text-sm) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-muted);
  }
  .rc-look__tag b {
    font-weight: 500;
    color: var(--app-text-strong);
  }
  .rc-look__tag i {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  .rc-look__legend {
    right: 12px;
    gap: 12px;
    font: 400 var(--text-xs) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .rc-look__legend:empty {
    display: none;
  }
  .rc-look__legend span {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .rc-look__legend span::before {
    content: "";
    width: 14px;
    height: 8px;
    border-radius: 2px;
  }
  .rc-look__legend .kw::before {
    background: var(--app-accent-bg);
    outline: 1.5px solid var(--app-accent);
  }
  .rc-look__legend .mn::before {
    outline: 1px dashed var(--app-text-subtle);
  }

  .rc-strip {
    display: flex;
    gap: 6px;
    min-width: 0;
    padding: 8px 10px 9px;
    border-top: 1px solid var(--mx-hairline);
  }
  .rc-film {
    flex: 1 1 0;
    min-width: 0;
    display: grid;
    grid-template-rows: auto 12px;
    gap: 5px;
    padding: 4px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    cursor: pointer;
    font: 450 var(--text-xs) / 12px var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
    text-align: center;
    transition:
      background-color var(--t-fast) var(--ease-quart),
      color var(--t-fast) var(--ease-quart);
  }
  .rc-film:hover {
    background: var(--mx-wash);
    color: var(--app-text);
  }
  .rc-film[aria-current="true"] {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .rc-film__img {
    position: relative;
    aspect-ratio: 8 / 5;
    overflow: hidden;
    border-radius: 4px;
    box-shadow: 0 0 0 1px var(--app-border);
    background: var(--app-surface-subtle);
    opacity: 0.6;
    transition: opacity var(--t-fast) var(--ease-quart);
  }
  .rc-film__img img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
    object-position: top;
  }
  .rc-film:hover .rc-film__img,
  .rc-film[aria-current="true"] .rc-film__img {
    opacity: 1;
  }
  .rc-film__img--gone {
    display: grid;
    place-items: center;
    background: var(--mx-wash);
    color: var(--app-text-faint);
    opacity: 1;
  }
  .rc-film small {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    font: inherit;
  }
  .rc-film[data-match] small::before {
    content: "";
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  .rc-film--turn {
    grid-template-rows: 52px 12px;
    text-align: left;
  }
  .rc-film__txt {
    padding: 6px 8px;
    border-radius: 4px;
    box-shadow: 0 0 0 1px var(--app-border);
    overflow: hidden;
    font: 400 var(--text-sm) / 1.35 var(--font-sans);
    color: var(--app-text-muted);
    display: -webkit-box;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    -webkit-box-orient: vertical;
  }
  .rc-film__txt b {
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .rc-film--turn small {
    justify-content: flex-start;
    padding-left: 2px;
  }

  .rc-strip,
  .rc-look__tag,
  .rc-look__legend {
    animation: mx-rise var(--t-slow) var(--ease-expo) 120ms both;
  }
  /* Audio stage: the line, large, over a zoomed waveform. */
  .rc-qa {
    --s: var(--app-source-sysaudio);
    position: absolute;
    inset: 0;
    display: grid;
    align-content: center;
    gap: 18px;
    padding: 44px 36px 20px;
  }
  .rc-qa[data-src="mic"] {
    --s: var(--app-source-mic);
  }
  .rc-qa__who {
    display: flex;
    align-items: center;
    gap: 9px;
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-muted);
  }
  .rc-qa__who b {
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .rc-spk {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    font: 600 11px / 1 var(--font-sans);
    font-style: normal;
    color: var(--app-text-strong);
    background: color-mix(in srgb, var(--s) 26%, var(--app-surface));
  }
  .rc-qa blockquote {
    position: relative;
    margin: 0;
    padding-left: 38px;
    font: 500 24px / 1.34 var(--font-sans);
    letter-spacing: -0.018em;
    color: var(--app-text-strong);
    text-wrap: pretty;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 5;
    line-clamp: 5;
    -webkit-box-orient: vertical;
  }
  .rc-qa blockquote::before {
    content: "\201C";
    position: absolute;
    left: 0;
    top: -6px;
    font: 700 56px / 1 var(--font-sans);
    color: color-mix(in srgb, var(--s) 72%, transparent);
  }
  .rc-qa__wave {
    position: relative;
    display: flex;
    align-items: center;
    gap: 2px;
    height: 64px;
  }
  .rc-qa__wave i {
    flex: 1;
    height: calc(var(--v) * 1%);
    min-height: 2px;
    border-radius: 1px;
    background: color-mix(in srgb, var(--app-fg) 18%, transparent);
  }
  .rc-qa__wave i.on {
    background: color-mix(in srgb, var(--s) 70%, transparent);
  }
  .rc-qa__wave b {
    position: absolute;
    top: -6px;
    bottom: -6px;
    left: var(--x);
    width: 1.5px;
    border-radius: 1px;
    background: var(--app-text-strong);
    transition: left var(--t-slow) var(--ease-expo);
  }
  .rc-qa__wave b[data-match] {
    background: var(--app-accent);
  }
  .rc-qa__axis {
    display: flex;
    justify-content: space-between;
    font: 450 var(--text-xs) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
  @media (prefers-reduced-motion: reduce) {
    .rc-look * {
      animation: none !important;
    }
  }
</style>
