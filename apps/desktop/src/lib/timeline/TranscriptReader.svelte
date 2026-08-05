<script lang="ts">
  // The reading surface: speaker-labelled prose blocks with a quiet, right-
  // aligned identity gutter and a ~58ch measure. Slices 1, 3, 4, 7 and 8 of the
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
    expanded: boolean;
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
    expanded,
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
  class:reader--expanded={expanded}
  data-ts={showTimestamps ? "on" : "off"}
>
  <div
    class="reader__scroll"
    role="list"
    bind:this={containerEl}
    ontouchmove={detach}
  >
    <article class="doc">
      {#each groups as group, index (index)}
        {@const chip = softDismissed.includes(group.clusterId)
          ? null
          : suggestionFor(group, index)}
        {@const unnamed = isUnnamed(group)}
        {@const timeLabel = formatTranscriptSegmentTitle(group)}
        <div
          class="turn"
          class:turn--overlap={group.overlaps}
          class:is-active={activeGroupIndex === index}
          data-speaker-group-index={index}
          role="listitem"
        >
          <div class="gutter">
            {#if repairable(group)}
              <button
                type="button"
                class="who"
                class:who--unknown={unnamed}
                class:who--needs={needsAttention(group)}
                aria-haspopup="dialog"
                aria-label={`Repair speaker ${speakerName(group)}`}
                onclick={() => onOpenRepair(index)}
              >
                <SpeakerMarkGlyph mark={marks.get(group.clusterId)} ghosted={unnamed} />
                <span class="who__nm">{speakerName(group)}</span>
                <span class="who__edit" aria-hidden="true">⋯</span>
              </button>
            {:else}
              <span class="who who--static" class:who--unknown={unnamed}>
                <SpeakerMarkGlyph mark={marks.get(group.clusterId)} ghosted={unnamed} />
                <span class="who__nm">{speakerName(group)}</span>
              </span>
            {/if}
            {#if unnamed && repairable(group) && !chip}
              <span class="gnote">name this voice</span>
            {:else if group.overlaps}
              <span class="gnote">overlapping speech</span>
            {/if}
            {#if chip}
              <span class="gsuggest">
                <span class="gsuggest__label"
                  >maybe {chip.name}{chip.meta ? ` · ${chip.meta}` : ""}</span
                >
                <button
                  type="button"
                  class="gsuggest__yes"
                  disabled={suggestionBusy(group)}
                  aria-label={`Confirm ${chip.name} — links this voice and saves a sample`}
                  onclick={() => onConfirmSuggestion(group)}>✓</button
                >
                <button
                  type="button"
                  class="gsuggest__no"
                  aria-label="Hide this suggestion — changes nothing, ask me later"
                  title={'Hide — writes nothing. "Not this person" lives in the repair panel.'}
                  onclick={() => (softDismissed = [...softDismissed, group.clusterId])}>✕</button
                >
              </span>
            {/if}
            <span class="ts">{timeLabel}</span>
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
              title={`Jump to ${timeLabel}`}
              onclick={() => onSeekMs(segmentSeekMs(segments, group))}>{group.text}</button
            >
          {/if}
        </div>
      {/each}
    </article>
  </div>

  <div class="jump" data-show={followDetached ? "1" : "0"} aria-hidden={!followDetached}>
    <button type="button" tabindex={followDetached ? 0 : -1} onclick={jumpToPlayhead}
      >↓ jump to playhead</button
    >
    <kbd>esc</kbd>
  </div>
</div>

<style>
  /* Direction 03: prose NEVER sits on material. The reader is an opaque plate
     inside the drawer's glass shell, so a transcript's contrast never depends
     on whatever is behind the window. */
  .reader {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    margin: 0 10px;
    border-radius: var(--r-lg);
    background: var(--app-surface);
    overflow: hidden;
  }

  .reader__scroll {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    scroll-behavior: smooth;
    scrollbar-width: thin;
    scrollbar-color: var(--app-border-strong) transparent;
    padding: 10px 0;
  }

  .reader--expanded .reader__scroll {
    padding: 28px 0;
  }

  .reader__scroll::-webkit-scrollbar {
    width: 8px;
  }

  .reader__scroll::-webkit-scrollbar-thumb {
    background: var(--app-border-strong);
    border-radius: 4px;
    border: 2px solid transparent;
    background-clip: padding-box;
  }

  /* A readable measure, not a percentage of however wide the window got. */
  .doc {
    max-width: min(97%, 70ch);
    margin: 0 auto;
    padding: 0 16px;
  }

  .reader--expanded .doc {
    padding: 0 24px;
  }

  /* The prose column fills the doc rather than sitting at a fixed ch measure:
     `.doc` at 97% of the scroll area puts the prose at ~75% of the window,
     which is what was asked for. ponytail: percentages have no upper bound, so
     on a very wide display the line runs well past the ~58ch comfortable for
     monospace (already ~96ch at 1280) — add a px ceiling to `.doc` (e.g.
     `min(97%, 1400px)`) if long lines start to hurt. */
  .turn {
    display: grid;
    grid-template-columns: 150px minmax(0, 1fr);
    gap: 16px;
    margin-bottom: 16px;
    align-items: start;
  }

  .reader--expanded .turn {
    grid-template-columns: 180px minmax(0, 1fr);
    gap: 24px;
    margin-bottom: 24px;
  }

  @media (max-width: 820px) {
    /* No side margin to spare down here — the 3% cost the wide measure pays
       measured narrower than the old fixed one at an 800px window. */
    .doc {
      max-width: 100%;
    }

    .turn,
    .reader--expanded .turn {
      grid-template-columns: minmax(0, 1fr);
      gap: 4px;
    }
  }

  /* ── the quiet identity gutter ───────────────────────────────────────────── */
  .gutter {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 5px;
    text-align: right;
    padding-top: 5px;
    min-width: 0;
  }

  @media (max-width: 820px) {
    .gutter {
      align-items: flex-start;
      text-align: left;
    }
  }

  .who {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    max-width: 100%;
    padding: 2px 5px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    font: inherit;
    font-size: 11px;
    letter-spacing: 0.01em;
    text-align: right;
    cursor: pointer;
  }

  .who:hover,
  .who:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-strong);
    color: var(--app-text-strong);
    outline: none;
  }

  .who:focus-visible {
    box-shadow: var(--app-ring);
  }

  .who--static {
    cursor: default;
  }

  .who__nm {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .who--unknown .who__nm {
    color: var(--app-text-subtle);
  }

  /* AUDIT 5 — the repair door is not hover-only. A cluster that still needs the
     user shows its marker at rest; a settled one reveals it on hover. */
  .who__edit {
    font-size: 10px;
    color: var(--app-text-subtle);
    opacity: 0;
    transition: opacity 120ms ease;
  }

  .who:hover .who__edit,
  .who:focus-visible .who__edit {
    opacity: 1;
  }

  .who--needs .who__edit {
    opacity: 0.75;
  }

  .gnote {
    padding-right: 5px;
    font-size: 10px;
    letter-spacing: 0.04em;
    color: var(--app-text-subtle);
  }

  .gsuggest {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    max-width: 100%;
    padding: 2px 4px 2px 8px;
    border: 1px solid var(--app-border-strong);
    border-radius: 999px;
    background: var(--app-surface-subtle, var(--app-surface));
    font-size: 10px;
    color: var(--app-text-muted);
  }

  .gsuggest__label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .gsuggest__yes,
  .gsuggest__no {
    flex: none;
    padding: 1px 5px;
    border-radius: 999px;
    background: transparent;
    font: inherit;
    font-size: 10px;
    cursor: pointer;
  }

  .gsuggest__yes {
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent);
  }

  .gsuggest__yes:hover:not(:disabled) {
    background: var(--app-accent-bg);
  }

  .gsuggest__yes:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: progress;
  }

  /* AUDIT 3 — a SOFT DISMISS. It writes nothing. */
  .gsuggest__no {
    border: 1px solid var(--app-border-strong);
    color: var(--app-text-subtle);
  }

  .gsuggest__no:hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
  }

  /* Timestamps: off by default, one toggle away, and on a SECOND GUTTER LINE —
     never a third column, so turning them on never reflows the measure. */
  .ts {
    display: none;
    font-family: var(--app-font-mono);
    font-size: 10px;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.06em;
    color: var(--app-text-subtle);
  }

  [data-ts="on"] .ts {
    display: block;
  }

  /* ── the prose: the one house-style break, taken deliberately ───────────── */
  /* The `.t-read` role: 14 / 1.55 / -.008em — prose, and only prose. */
  .para {
    margin: 0;
    font-size: var(--t-read);
    line-height: 1.55;
    color: var(--app-text);
    letter-spacing: -0.008em;
    text-wrap: pretty;
  }

  .reader--expanded .para {
    font-size: 15px;
    line-height: 1.62;
  }

  .turn--overlap .para {
    box-shadow: inset 2px 0 0 var(--app-warn-border);
    padding-left: 14px;
    margin-left: -16px;
  }

  /* ── word states: underline-led, never a filled box ──────────────────────
     Every state changes COLOUR and text-decoration only. No background, no
     transform, no padding/weight change: these fire while the text is
     reflowing under the playhead, and `text-decoration` doesn't grow the line
     box the way `border-bottom` would. */
  .w {
    padding: 0 0.5px;
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-decoration: underline 1px transparent;
    text-underline-offset: 3px;
    text-decoration-skip-ink: none;
    cursor: pointer;
    transition:
      color 80ms ease,
      text-decoration-color 80ms ease;
  }

  .w:hover {
    color: var(--app-text-strong);
    text-decoration-color: var(--app-text-subtle);
  }

  .w.is-done {
    color: var(--app-text-muted);
  }

  .w.is-now {
    color: var(--app-accent);
    text-decoration-color: var(--app-accent);
  }

  /* Pressed: the text dims for the duration of the press. Nothing moves. */
  .w:active,
  .para--button:active {
    color: var(--app-text-subtle);
  }

  /* Quieter states must not cost keyboard users the focus indicator. */
  .w:focus-visible {
    outline: none;
    border-radius: 3px;
    box-shadow: var(--app-ring);
  }

  /* AUDIT 7 — with no words[] the paragraph itself is the seek target. */
  .para--button {
    display: block;
    width: 100%;
    padding: 0;
    border: 0;
    /* A <button> brings a UA background + sans font with it; the degraded branch
       must read as the same prose the karaoke branch does. */
    appearance: none;
    -webkit-appearance: none;
    background: transparent;
    color: var(--app-text);
    font-family: inherit;
    font-weight: inherit;
    letter-spacing: inherit;
    text-align: left;
    cursor: pointer;
    /* No underline anywhere on this branch — see the :hover rule below. */
    transition: color 80ms ease;
  }

  /* Colour only, deliberately NOT the word branch's underline: a rule under
     every line of a multi-line paragraph reads as a hyperlink block rather than
     as prose. Brightening the whole block is affordance enough at this size. */
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

  /* ── jump-to-playhead ────────────────────────────────────────────────────
     AUDIT 4 — neutral chrome, not accent: full-strength accent is reserved for
     the playhead and the current word. AUDIT 10 — it genuinely fades, which
     display:none could never do. */
  .jump {
    position: absolute;
    left: 50%;
    bottom: 10px;
    z-index: 5;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 12px;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--glass-pop);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    box-shadow: var(--sh-float), inset 0 0 0 var(--hairline) var(--glass-line);
    color: var(--app-text-strong);
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    opacity: 0;
    visibility: hidden;
    transform: translate(-50%, 6px);
    transition:
      opacity 180ms ease,
      transform 180ms cubic-bezier(0.2, 0.7, 0.2, 1),
      visibility 180ms;
  }

  .jump[data-show="1"] {
    opacity: 1;
    visibility: visible;
    transform: translate(-50%, 0);
  }

  .jump button {
    border: 0;
    background: transparent;
    color: inherit;
    font: inherit;
    text-transform: inherit;
    letter-spacing: inherit;
    cursor: pointer;
  }

  .jump kbd {
    padding: 0 4px;
    border: 0;
    border-radius: var(--r-sm);
    background: var(--glass-tint);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
    color: var(--app-text-subtle);
    font-family: var(--app-font-mono);
    text-transform: none;
    letter-spacing: 0;
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
