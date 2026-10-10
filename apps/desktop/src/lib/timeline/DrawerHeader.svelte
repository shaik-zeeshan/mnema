<script lang="ts">
  // The drawer's identity strip: source, index, clock, duration, model label —
  // then the live status pill and the four chrome actions (rerun, timestamps,
  // expand/collapse, close). The status pill is a `role="status"` live region
  // because every frame-6 state changes without the user doing anything.
  // Under it, the speaker strip: one toggle chip per voice (mark · name · state ·
  // talk share); a selected chip dims every other voice, "only this voice" hides them.
  import IconX from "~icons/lucide/x";
  import { tip } from "$lib/components/tooltip";
  import SpeakerMarkGlyph from "./SpeakerMark.svelte";
  import {
    formatCompactDuration,
    type AudioSegmentRecord,
    type SpeakerMark,
    type StatusPill,
    type StatusTone,
    type StripSpeaker,
  } from "./audio-drawer-view";

  // Neutral unless something needs the user: the accent stays the playhead's.
  const TONE: Record<StatusTone, string> = { ok: "", work: "", idle: "", warn: "mx-chip--warn", bad: "mx-chip--danger" };

  interface Props {
    segment: AudioSegmentRecord;
    sourceLabel: string;
    timeRangeLabel: string;
    timeRangeTip: string;
    durationLabel: string;
    modelLabel: string | null;
    status: StatusPill;
    actionLabel: string;
    actionDisabled: boolean;
    actionTitle: string;
    rerunLoading: boolean;
    onRerun: () => void;
    onClose: () => void;
    speakers: StripSpeaker[];
    marks: Map<number, SpeakerMark>;
    selectedClusterId?: number | null;
    onlySelected?: boolean;
    showTimestamps?: boolean;
    expanded?: boolean;
  }

  let {
    segment,
    sourceLabel,
    timeRangeLabel,
    timeRangeTip,
    durationLabel,
    modelLabel,
    status,
    actionLabel,
    actionDisabled,
    actionTitle,
    rerunLoading,
    onRerun,
    onClose,
    speakers,
    marks,
    selectedClusterId = $bindable(null),
    onlySelected = $bindable(false),
    showTimestamps = $bindable(false),
    expanded = $bindable(false),
  }: Props = $props();

  const hasSelection = $derived(speakers.some((s) => s.clusterId === selectedClusterId));

  function toggleSpeaker(clusterId: number): void {
    selectedClusterId = selectedClusterId === clusterId ? null : clusterId;
    if (selectedClusterId == null) onlySelected = false;
  }
</script>

<header class="rhead">
  <span class="mx-chip mx-chip--{segment.source === 'systemAudio' ? 'sysaudio' : 'mic'}"
    >{sourceLabel.toLowerCase()}</span
  >
  <span class="num">#{segment.segmentIndex}</span>
  <span class="rhead__sep" aria-hidden="true">·</span>
  <span class="num" use:tip={timeRangeTip}>{timeRangeLabel}</span>
  <span class="rhead__sep" aria-hidden="true">·</span>
  <span class="num">{durationLabel}</span>
  {#if modelLabel}
    <span class="rhead__sep" aria-hidden="true">·</span>
    <span use:tip={modelLabel}>{modelLabel}</span>
  {/if}
  <span class="rhead__file" use:tip={segment.filePath}>{segment.fileName}</span>
  <span class="rhead__grow"></span>
  <span class="mx-chip {TONE[status.tone]}" role="status" aria-live="polite" aria-busy={status.busy}>
    {#if status.busy}<span class="mx-spin mx-spin--sm" aria-hidden="true"></span>{/if}
    {status.label}
  </span>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--sm"
    onclick={onRerun}
    disabled={actionDisabled}
    use:tip={actionTitle}
  >
    {rerunLoading ? "starting…" : actionLabel.toLowerCase()}
  </button>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--sm"
    aria-pressed={showTimestamps}
    onclick={() => (showTimestamps = !showTimestamps)}
  >
    timestamps
  </button>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--sm"
    aria-pressed={expanded} onclick={() => (expanded = !expanded)}>
    {expanded ? "collapse" : "expand"}
  </button>
  <button
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
    onclick={onClose}
    aria-label="Close audio player"
    use:tip={"Close  esc"}
  >
    <IconX width="15" height="15" aria-hidden="true" />
  </button>
</header>

{#if speakers.length > 0}
  <div class="spk" role="toolbar" aria-label="Speakers in this segment">
    <span class="spk__lbl">Speakers</span>
    <div class="spk__list">
      {#each speakers as speaker (speaker.clusterId)}
        <button
          type="button"
          class="mx-btn mx-btn--ghost mx-btn--sm sp"
          class:sp--unnamed={speaker.unnamed}
          aria-pressed={selectedClusterId === speaker.clusterId}
          aria-label={`${speaker.name}${speaker.state ? `, ${speaker.state}` : ""}, ${formatCompactDuration(speaker.talkMs)} talk time`}
          onclick={() => toggleSpeaker(speaker.clusterId)}
        >
          <SpeakerMarkGlyph mark={marks.get(speaker.clusterId)} ghosted={speaker.unnamed} />
          <span>{speaker.name}</span>
          {#if speaker.state}<span class="sp__state">{speaker.state}</span>{/if}
          <span class="sp__share">{speaker.sharePct}%</span>
        </button>
      {/each}
    </div>
    <span class="rhead__grow"></span>
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--sm"
      aria-pressed={onlySelected && hasSelection}
      disabled={!hasSelection}
      onclick={() => (onlySelected = !onlySelected)}
    >
      only this voice
    </button>
  </div>
{/if}

<style>
  .rhead {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 10px 10px 10px 14px;
    border-bottom: 1px solid var(--mx-hairline);
    font: 500 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-muted);
  }

  .rhead__sep {
    color: var(--app-text-faint);
  }

  .rhead__file {
    max-width: 18ch;
    margin-left: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }

  .rhead__grow {
    flex: 1 1 auto;
  }

  /* ── speaker strip ─────────────────────────────────────────────────────── */
  .spk {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    padding: 5px 10px 5px 14px;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .spk__lbl {
    margin-right: 8px;
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
    white-space: nowrap;
  }

  .spk__list {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }

  .sp {
    --_gap: 7px;
  }

  .sp--unnamed {
    --_fg: var(--app-text-muted);
  }

  .sp__state {
    font-weight: 400;
    color: var(--app-text-subtle);
  }

  .sp__share {
    font: 400 var(--text-sm) / 1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-subtle);
  }
</style>
