<script lang="ts">
  // The 256px inspector — the selected activity's record.
  //
  // This is what lets a river card stay four lines: no card grows a second
  // column, because the evidence behind it lives here. Every row is counted from
  // data the surface already holds; a figure that cannot be counted is absent
  // (**G8**), which is why "speakers" and "subjects" simply disappear on an
  // activity that has neither.
  import IconPanel from "~icons/lucide/panel-right";
  import IconPlay from "~icons/lucide/play";
  import IconClock from "~icons/lucide/clock";
  import type { Activity } from "$lib/types/recording";
  import type { TurnView } from "$lib/insights/receipt-audio";
  import { categoryLabel, focusHint, humanizeMs } from "$lib/insights/activity-helpers";
  import { speakerRoster, type ActivityRecord } from "./journal-record";

  interface Props {
    activity: Activity | null;
    record: ActivityRecord | null;
    /** 1-based position of the selection among the day's activities. */
    ordinal: number;
    total: number;
    subjects: string[];
    /** Spoken turns over the span; empty until (or unless) any exist. */
    turns: TurnView[];
    turnsLoading: boolean;
    onopen: () => void;
    ontimeline: () => void;
  }

  let {
    activity,
    record,
    ordinal,
    total,
    subjects,
    turns,
    turnsLoading,
    onopen,
    ontimeline,
  }: Props = $props();

  function hhmm(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });
  }
  function hhmmss(ms: number): string {
    return new Date(ms).toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
      hour12: false,
    });
  }

  const speakers = $derived(speakerRoster(turns.map((t) => t.speaker)));

  /** Where each spoken turn sits over the span — real timings, not a drawn
   *  waveform: the strip says WHEN there was speech and by whom. */
  const lane = $derived.by(() => {
    if (!activity || turns.length === 0) return [];
    const span = activity.endedAtMs - activity.startedAtMs;
    if (span <= 0) return [];
    return turns.map((t) => {
      const left = Math.max(0, (t.startMs - activity.startedAtMs) / span);
      const right = Math.min(1, (t.endMs - activity.startedAtMs) / span);
      return { key: t.key, left: left * 100, width: Math.max(0.8, (right - left) * 100), colorVar: t.colorVar };
    });
  });
</script>

<aside class="ss-insp" aria-label="Inspector">
  <div class="ss-insp__h">
    <span class="ic" aria-hidden="true"><IconPanel /></span>
    <span>Inspector</span>
    <span class="spacer"></span>
    <span class="kbd">⌥⌘I</span>
  </div>

  <div class="ss-insp__b">
    {#if activity && record}
      <div class="ss-insp__sec">
        <span>Activity</span>
        <span class="pos">{ordinal} of {total}</span>
      </div>
      <div class="ss-kv ss-kv--stack">
        <span class="ss-kv__k">Title</span>
        <span class="ss-kv__v title">{activity.title}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Category</span>
        <span class="ss-kv__v">
          {activity.category ? categoryLabel(activity.category) : "Uncategorized"}
        </span>
      </div>
      {#if activity.focus}
        <div class="ss-kv">
          <span class="ss-kv__k">Focus</span>
          <span class="ss-kv__v">{focusHint(activity.focus)}</span>
        </div>
      {/if}
      <div class="ss-kv">
        <span class="ss-kv__k">When</span>
        <span class="ss-kv__v is-mono">{hhmm(activity.startedAtMs)} – {hhmm(activity.endedAtMs)}</span>
      </div>
      <div class="ss-kv">
        <span class="ss-kv__k">Duration</span>
        <span class="ss-kv__v is-mono">{humanizeMs(activity.endedAtMs - activity.startedAtMs)}</span>
      </div>

      <div class="ss-insp__sec"><span>Evidence</span></div>
      <div class="ss-kv">
        <span class="ss-kv__k">Frames</span>
        <span class="ss-kv__v is-mono">
          {#if record.frameCount > 0}
            {record.frameCount} · {record.segmentCount}
            {record.segmentCount === 1 ? "segment" : "segments"}
          {:else}
            none on disk
          {/if}
        </span>
      </div>
      {#if record.citedFrames > 0 || record.citedSpoken > 0}
        <div class="ss-kv">
          <span class="ss-kv__k">Cited</span>
          <span class="ss-kv__v is-mono">
            {record.citedFrames} frames · {record.citedSpoken} spoken
          </span>
        </div>
      {/if}
      {#if record.headlineMs !== null}
        <div class="ss-kv">
          <span class="ss-kv__k">Headline</span>
          <span class="ss-kv__v is-mono">frame · {hhmmss(record.headlineMs)}</span>
        </div>
      {/if}
      {#if speakers}
        <div class="ss-kv">
          <span class="ss-kv__k">Speakers</span>
          <span class="ss-kv__v">{speakers}</span>
        </div>
        <!-- Spoken coverage over the span, one block per turn, speaker-coloured.
             Not a waveform: nothing here is drawn from samples we do not read. -->
        <div class="lane" aria-hidden="true">
          {#each lane as block (block.key)}
            <i style="left:{block.left}%;width:{block.width}%;background:var({block.colorVar});"></i>
          {/each}
        </div>
      {:else if turnsLoading}
        <div class="ss-kv">
          <span class="ss-kv__k">Speakers</span>
          <span class="ss-kv__v quiet">reading…</span>
        </div>
      {/if}

      {#if subjects.length > 0}
        <div class="ss-insp__sec"><span>Derived from this</span></div>
        <div class="ss-kv">
          <span class="ss-kv__k">Subjects</span>
          <span class="ss-kv__v">{subjects.join(" · ")}</span>
        </div>
      {/if}

      <div class="ss-insp__sec"><span>Actions</span></div>
      <div class="acts">
        <button
          type="button"
          class="btn btn--sm btn--primary act"
          disabled={record.frameCount === 0 && record.citedSpoken === 0}
          onclick={onopen}
        >
          <IconPlay />Open receipt<span class="kbd kbd--end">⏎</span>
        </button>
        <button
          type="button"
          class="btn btn--sm act"
          disabled={record.firstFrameId === null}
          onclick={ontimeline}
        >
          <IconClock />Show in Timeline<span class="kbd kbd--end">⌘1</span>
        </button>
      </div>
    {:else}
      <p class="ss-insp__empty">Select a card in the river to see its record here.</p>
    {/if}
  </div>
</aside>

<style>
  .ic {
    display: flex;
    font-size: 11px;
  }
  .spacer {
    margin-left: auto;
  }
  .pos {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    text-transform: none;
    letter-spacing: 0;
  }
  .title {
    font: var(--w-semi) 15px / 1.3 var(--app-font-sans);
    letter-spacing: var(--ls-title);
  }
  .quiet {
    color: var(--app-text-subtle);
  }

  .lane {
    position: relative;
    height: 22px;
    margin: 2px var(--s-10) 6px;
    border-radius: var(--r-sm);
    background: var(--app-surface-subtle);
    overflow: hidden;
  }
  .lane i {
    position: absolute;
    top: 4px;
    bottom: 4px;
    /* A 12-second turn inside a 55-minute span is 0.4% wide; without a floor it
       would be invisible, and a turn that happened must be visible. */
    min-width: 2px;
    border-radius: 1px;
    opacity: 0.9;
  }

  .acts {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding: 6px var(--s-10) 0;
  }
  .act {
    justify-content: flex-start;
    width: 100%;
  }
  .kbd--end {
    margin-left: auto;
    background: transparent;
    color: inherit;
    opacity: 0.75;
  }
</style>
