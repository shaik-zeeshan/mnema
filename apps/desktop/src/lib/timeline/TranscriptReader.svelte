<script lang="ts">
  // The reading surface: one padded row per speaker turn — a left speaker column
  // (mark · name, the turn's clock time under it) beside a ~78ch prose measure. Slices 1, 3, 4, 7 and 8 of the
  // redesign live here — typography, the dual-encoded marker, the timestamps
  // toggle, follow mode, and word-level karaoke.
  //
  // Karaoke is load-bearing: once the highlight tracks playback a timestamp
  // column has nothing left to do, which is what pays for timestamps being off
  // by default. When the provider emits no `words[]` the whole paragraph becomes
  // the seek target instead (the pre-existing behaviour) — the mechanic survives
  // at a coarser unit rather than the feature quietly disappearing.
  import {
    activeKaraokeIndex,
    karaokeForGroup,
    formatTranscriptSegmentTitle,
    segmentSeekMs,
    wordSeekMs,
    type KaraokeWord,
    type SpeakerMark,
    type SpeakerTranscriptGroup,
  } from "./audio-drawer-view";
  import SpeakerMarkGlyph from "./SpeakerMark.svelte";
  import IconArrowDown from "~icons/lucide/arrow-down";
  import IconCheck from "~icons/lucide/check";
  import IconMore from "~icons/lucide/ellipsis";
  import IconX from "~icons/lucide/x";
  import { tip } from "$lib/components/tooltip";
  import type { TranscriptionSegment, TranscriptionWord } from "$lib/types/app-infra";

  interface SuggestionChip {
    name: string;
    /** `maybe · 0.88`-style confidence line, when there is one. */
    meta: string | null;
  }

  interface Props {
    groups: SpeakerTranscriptGroup[];
    marks: Map<number, SpeakerMark>;
    /** `TranscriptionStructuredPayload.words[]`; empty = degraded karaoke. */
    words: TranscriptionWord[];
    /** The transcription's own timed runs — the seek boundary the degraded
     *  branch snaps to instead of the whole speaker group's start. */
    segments: TranscriptionSegment[];
    currentMs: number;
    activeGroupIndex: number | null;
    showTimestamps: boolean;
    /** Wall-clock start of the segment; a turn's time = this + its offset. */
    segmentStartMs: number;
    /** The turn the repair slide-over is aimed at, drawn selected. */
    fixingIndex: number | null;
    /** Speaker-strip selection: every other voice recedes (0.38). */
    selectedClusterId: number | null;
    /** "only this voice": the other voices' turns are hidden, not dimmed. */
    onlySelected: boolean;
    speakerName: (group: SpeakerTranscriptGroup) => string;
    isUnnamed: (group: SpeakerTranscriptGroup) => boolean;
    /** Unnamed, or carrying an unconfirmed suggestion: show the repair door at rest. */
    needsAttention: (group: SpeakerTranscriptGroup) => boolean;
    /** False for a paragraph with no diarized owner (the transcription fallback):
     *  there is no cluster to repair, so no door and no "name this voice" nudge. */
    repairable: (group: SpeakerTranscriptGroup) => boolean;
    suggestionFor: (group: SpeakerTranscriptGroup, index: number) => SuggestionChip | null;
    suggestionBusy: (group: SpeakerTranscriptGroup) => boolean;
    onConfirmSuggestion: (group: SpeakerTranscriptGroup) => void;
    onSeekMs: (ms: number) => void;
    onOpenRepair: (index: number) => void;
    /** Detached follow mode: the parent's Esc handler re-attaches instead of closing. */
    followDetached?: boolean;
    containerEl?: HTMLDivElement | null;
  }

  let {
    groups,
    marks,
    words,
    segments,
    currentMs,
    activeGroupIndex,
    showTimestamps,
    segmentStartMs,
    fixingIndex,
    selectedClusterId,
    onlySelected,
    speakerName,
    isUnnamed,
    needsAttention,
    repairable,
    suggestionFor,
    suggestionBusy,
    onConfirmSuggestion,
    onSeekMs,
    onOpenRepair,
    followDetached = $bindable(false),
    containerEl = $bindable(null),
  }: Props = $props();

  // A soft dismiss: hides a suggestion chip the user doesn't want to answer right
  // now and writes NOTHING. The destructive twin ("Not this person") lives in the
  // repair slide-over with danger styling and its consequence spelled out.
  let softDismissed = $state<number[]>([]);
  // Reset when the transcript's SPEAKER SET changes — never on `groups`' array
  // identity. `groups` is rebuilt by every `refreshCurrentSpeakerTurns()` (which
  // every speaker write ends in) and by every transcript-poll tick while a job is
  // pending, so keying the reset on the array threw the user's dismissals away
  // seconds after they pressed the hide button: answering one voice resurrected the
  // chip they had just hidden on another. A `$derived` only propagates when its
  // VALUE changes, so this survives an equal-content reload and still resets on a
  // real change (different segment, a merge, a cluster appearing).
  const speakerSetKey = $derived(groups.map((group) => group.clusterId).join(","));
  $effect(() => {
    // Reset when the transcript itself changes.
    void speakerSetKey;
    softDismissed = [];
  });

  /**
   * Per-group karaoke words, or null when the provider gave us nothing usable.
   * The coverage guard matters: a partial `words[]` would silently drop most of a
   * paragraph's text, so anything under 80% coverage degrades to the paragraph.
   */
  const karaoke = $derived.by<(KaraokeWord[] | null)[]>(() =>
    groups.map((group) => karaokeForGroup(words, group)),
  );

  // ── Follow mode ───────────────────────────────────────────────────────────
  // `wheel`/`touchmove` are the manual-scroll signal precisely because they are
  // user-only: `scrollIntoView` moves the container without ever firing them, so
  // no "was that us?" timing guard is needed (an earlier guard here refreshed on
  // every follow scroll and therefore made detaching during playback impossible).
  // The element the reader last centred on. NOT `$state`: it is written from inside
  // the follow effect, and a reactive write there would re-trigger it.
  let centredOn: HTMLElement | null = null;

  function detach(): void {
    followDetached = true;
    // The user moved the container out from under the last centred element, so
    // re-attaching has to scroll again even if the same word still holds the floor.
    centredOn = null;
  }

  // `wheel` is attached by hand because Svelte auto-passives only `touchstart`
  // and `touchmove` (its PASSIVE_EVENTS list). A non-passive wheel listener on
  // the scroll container makes WebKit dispatch every wheel tick to the main
  // thread BEFORE it is allowed to scroll — and `detach` never calls
  // preventDefault, so there is nothing to wait for.
  $effect(() => {
    const container = containerEl;
    if (!container) return;
    container.addEventListener("wheel", detach, { passive: true });
    return () => container.removeEventListener("wheel", detach);
  });

  function scrollActiveIntoView(): void {
    const container = containerEl;
    if (!container) return;
    const target =
      container.querySelector<HTMLElement>(".para .w.is-now") ??
      container.querySelector<HTMLElement>('[data-speaker-group-index].is-active');
    if (!target) return;
    // `currentMs` ticks ~4x/s but the highlighted word only moves ~2.5x/s, so a
    // third of the ticks re-centre on the element already centred. That matters more
    // than the wasted call: `behavior: "smooth"` restarts a scroll animation WebKit
    // has not finished, so the container never settles and its ~800-word subtree
    // keeps compositing for the whole of playback.
    if (target === centredOn) return;
    centredOn = target;
    target.scrollIntoView({ block: "center", behavior: "smooth" });
  }

  $effect(() => {
    // Re-run when the highlight moves or follow is re-attached.
    void currentMs;
    void activeGroupIndex;
    if (followDetached) return;
    scrollActiveIntoView();
  });

  function jumpToPlayhead(): void {
    followDetached = false;
    scrollActiveIntoView();
  }
</script>

<div
  class="reader"
  class:reader--sel={selectedClusterId != null}
  class:reader--only={onlySelected}
  data-ts={showTimestamps ? "on" : "off"}
>
  <div
    class="reader__scroll"
    role="list"
    bind:this={containerEl}
    ontouchmove={detach}
  >
    {#each groups as group, index (index)}
      {@const chip = softDismissed.includes(group.clusterId)
        ? null
        : suggestionFor(group, index)}
      {@const unnamed = isUnnamed(group)}
      {@const mark = marks.get(group.clusterId)}
      <div
        class="turn"
        class:turn--overlap={group.overlaps}
        class:is-active={activeGroupIndex === index}
        class:is-sel={group.clusterId === selectedClusterId}
        class:is-fixing={fixingIndex === index}
        style={mark?.colorVar ? `--sp: var(${mark.colorVar})` : undefined}
        data-speaker-group-index={index}
        role="listitem"
      >
        <div class="gutter">
          {#if repairable(group)}
            <button
              type="button"
              class="mx-btn mx-btn--ghost mx-btn--sm who"
              class:who--unknown={unnamed}
              class:who--needs={needsAttention(group)}
              aria-haspopup="dialog"
              aria-label={`Repair speaker ${speakerName(group)}`}
              onclick={() => onOpenRepair(index)}
            >
              <SpeakerMarkGlyph {mark} ghosted={unnamed} />
              <span class="who__nm">{speakerName(group)}</span>
              <IconMore class="who__edit" width="13" height="13" aria-hidden="true" />
            </button>
          {:else}
            <span class="who who--static" class:who--unknown={unnamed}>
              <SpeakerMarkGlyph {mark} ghosted={unnamed} />
              <span class="who__nm">{speakerName(group)}</span>
            </span>
          {/if}
          {#if unnamed && repairable(group) && !chip}
            <span class="gnote">name this voice</span>
          {:else if group.overlaps}
            <span class="gnote">overlapping speech</span>
          {/if}
          {#if chip}
            <span class="suggest">
              maybe <b>{chip.name.split(" ")[0]}</b>{#if chip.meta}<span class="num"
                  >· {chip.meta}</span
                >{/if}
              <button
                type="button"
                class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
                disabled={suggestionBusy(group)}
                aria-label={`Confirm ${chip.name} — links this voice and saves a sample`}
                use:tip={"Confirm — links + saves a sample"}
                onclick={() => onConfirmSuggestion(group)}
                ><IconCheck width="13" height="13" aria-hidden="true" /></button
              >
              <button
                type="button"
                class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
                aria-label="Hide this suggestion — changes nothing, ask me later"
                use:tip={"Hide — writes nothing"}
                onclick={() => (softDismissed = [...softDismissed, group.clusterId])}
                ><IconX width="13" height="13" aria-hidden="true" /></button
              >
            </span>
          {/if}
          <span class="ts">{new Date(segmentStartMs + group.startMs).toLocaleTimeString()}</span>
        </div>

        {#if karaoke[index]}
          {@const wordList = karaoke[index] ?? []}
          {@const nowIndex =
            activeGroupIndex === index ? activeKaraokeIndex(wordList, currentMs) : -1}
          <p class="para">
            {#each wordList as word, wi (wi)}<button
                type="button"
                class="w"
                class:is-done={word.endMs <= currentMs && wi !== nowIndex}
                class:is-now={wi === nowIndex}
                tabindex="-1"
                onclick={() => onSeekMs(wordSeekMs(word, group))}>{word.text}</button
              >{" "}{/each}
          </p>
        {:else}
          <!-- Degraded: no word timings, so the transcription run is the seek
               unit (not the whole speaker group's start). -->
          <button
            type="button"
            class="para para--button"
            class:is-seg-now={activeGroupIndex === index}
            title={`Jump to ${formatTranscriptSegmentTitle(group)}`}
            onclick={() => onSeekMs(segmentSeekMs(segments, group))}>{group.text}</button
          >
        {/if}
      </div>
    {/each}
  </div>

  <button
    type="button"
    class="mx-btn mx-btn--sm jump"
    data-show={followDetached ? "1" : "0"}
    aria-hidden={!followDetached}
    tabindex={followDetached ? 0 : -1}
    onclick={jumpToPlayhead}
  >
    <IconArrowDown width="13" height="13" aria-hidden="true" />jump to playhead<kbd>esc</kbd>
  </button>
</div>

<style>
  .reader {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .reader__scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    scroll-behavior: smooth;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
    padding: 4px 0;
  }

  /* One padded row per turn, so the active / selected wash has room to read. */
  .turn {
    display: grid;
    grid-template-columns: 176px minmax(0, 1fr);
    gap: var(--s-5);
    padding: 12px 22px 12px 18px;
    transition: background-color var(--t-med) var(--ease-quart);
  }

  .turn + .turn {
    border-top: 1px solid var(--mx-hairline);
  }

  .turn.is-active {
    background: color-mix(in srgb, var(--sp, var(--app-text-subtle)) 8%, transparent);
  }

  /* Speaker selection: everyone else recedes; "only" filters to them. */
  .reader--sel .turn:not(.is-sel) {
    opacity: 0.38;
  }

  .reader--sel .turn.is-sel:not(.is-active) {
    background: var(--mx-wash);
  }

  .reader--only .turn:not(.is-sel) {
    display: none;
  }

  .turn.is-fixing {
    background: var(--mx-selected);
  }

  @media (max-width: 820px) {
    .turn {
      grid-template-columns: minmax(0, 1fr);
      gap: 4px;
    }
  }

  /* ── the speaker column ─────────────────────────────────────────────────── */
  .gutter {
    display: grid;
    align-content: start;
    justify-items: start;
    gap: 6px;
    min-width: 0;
  }

  .who {
    max-width: 100%;
    /* Align the ghost button's label with the column edge. */
    margin-left: -9px;
    --_fg: var(--app-text-strong);
  }

  .who--static {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    margin-left: 0;
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--_fg);
  }

  .who--unknown {
    --_fg: var(--app-text-muted);
  }

  .who__nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* AUDIT 5 — the repair door is not hover-only. A cluster that still needs the
     user shows its marker at rest; a settled one reveals it on hover. */
  .who :global(.who__edit) {
    flex: none;
    color: var(--app-text-subtle);
    opacity: 0;
    transition: opacity var(--t-fast) var(--ease-quart);
  }

  .who:hover :global(.who__edit),
  .who:focus-visible :global(.who__edit),
  .who--needs :global(.who__edit) {
    opacity: 1;
  }

  .gnote {
    font: 400 var(--text-sm) / 1.2 var(--font-sans);
    color: var(--app-text-muted);
  }

  .suggest {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    font: 400 var(--text-sm) / 1.3 var(--font-sans);
    color: var(--app-text-muted);
  }

  .suggest b {
    font-weight: 500;
    color: var(--app-text);
  }

  .suggest .num {
    margin-right: 2px;
    color: var(--app-text-subtle);
  }

  /* Timestamps: off by default, one toggle away, on a second gutter line —
     never a third column, so turning them on never reflows the measure. */
  .ts {
    font: 400 var(--text-xs) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }

  [data-ts="off"] .ts {
    display: none;
  }

  /* ── the prose ──────────────────────────────────────────────────────────── */
  .para {
    margin: 0;
    max-width: 78ch;
    font: 400 var(--text-base) / 1.65 var(--font-sans);
    color: var(--app-text-subtle);
    text-wrap: pretty;
  }

  .turn--overlap .para {
    border-left: 2px solid var(--app-warn-border);
    padding-left: 14px;
    margin-left: -16px;
  }

  /* ── word states: colour + a wash on the current word. Nothing here changes
     the box (no padding / weight), because these fire while the text reflows
     under the playhead. */
  .w {
    padding: 0;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
    transition: color var(--t-fast) var(--ease-quart);
  }

  .w:hover {
    color: var(--app-text-strong);
  }

  .w.is-done {
    color: var(--app-text);
  }

  .w.is-now {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    box-shadow: 0 0 0 2px var(--app-accent-bg);
  }

  /* Quieter states must not cost keyboard users the focus indicator. */
  .w:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* AUDIT 7 — with no words[] the paragraph itself is the seek target. */
  .para--button {
    display: block;
    width: 100%;
    padding: 0;
    border: 0;
    appearance: none;
    -webkit-appearance: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    transition: color var(--t-fast) var(--ease-quart);
  }

  .para--button:hover {
    color: var(--app-text-strong);
  }

  .para--button:focus-visible {
    outline: none;
    border-radius: 4px;
    box-shadow: var(--app-ring);
  }

  .para--button.is-seg-now {
    color: var(--app-accent);
  }

  /* ── jump-to-playhead: it genuinely fades, which display:none never could. */
  .jump {
    position: absolute;
    left: 50%;
    bottom: 10px;
    z-index: 5;
    box-shadow: var(--app-shadow-popover);
    opacity: 0;
    visibility: hidden;
    transform: translate(-50%, 6px);
    transition:
      opacity var(--t-med) var(--ease-quart),
      transform var(--t-med) var(--ease-expo),
      visibility var(--t-med);
  }

  .jump[data-show="1"] {
    opacity: 1;
    visibility: visible;
    transform: translate(-50%, 0);
  }

  @media (prefers-reduced-motion: reduce) {
    .reader__scroll {
      scroll-behavior: auto;
    }

    .jump {
      transition: none;
    }
  }
</style>
