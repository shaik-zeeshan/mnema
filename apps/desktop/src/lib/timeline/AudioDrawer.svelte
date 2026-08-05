<script lang="ts">
  // The Timeline audio drawer, redesigned as a reading document
  // (docs/transcription/mockups/transcription-reader.html).
  //
  // Two states of ONE drawer: `peek` is the 50vh bottom sheet the timeline has
  // always had — an entry point that answers "what is this segment" in two
  // paragraphs — and `expanded` lifts the ceiling into a full reader. The
  // expanded state covers the timeline and so loses the non-modal
  // click-another-bar behaviour; that tradeoff is accepted (mockup annotation
  // (f)) and peek is the mitigation.
  //
  // This component owns the drawer's own view state: the audio element and
  // transport, expanded/peek, the timestamps toggle, follow mode, the waveform
  // peaks, and the repair slide-over. Data loading and every DB write stay in
  // routes/+page.svelte and arrive here as props/callbacks.
  import { tick } from "svelte";
  import { waveformPeaks } from "./waveform-peaks.svelte";
  import { AudioTransport } from "./audio-transport.svelte";
  import { getFocusableElements, trapTabKey } from "$lib/keyboard";
  import { tip } from "$lib/components/tooltip";
  import DrawerHeader from "./DrawerHeader.svelte";
  import DrawerStatePanels from "./DrawerStatePanels.svelte";
  import DrawerTransport from "./DrawerTransport.svelte";
  import SpeakerRepairPanel from "./SpeakerRepairPanel.svelte";
  import TranscriptReader from "./TranscriptReader.svelte";
  import {
    activeGroupIndex as resolveActiveGroupIndex,
    assignSpeakerMarks,
    buildSpeakerGroups,
    drawerPanelKind,
    drawerStatusPill,
    transcriptFallbackGroups,
    processingStageLabel,
    provenanceFootnote,
    queuedAgoLabel,
    speakerClusterOptionLabel,
    speakerIsUnnamed,
    speakerPersistedName,
    speakerProfileName,
    suggestedMergeTargetLabel,
    samplePreviewHeadsMs,
    suggestionChipFor,
    formatScore,
    waveformBars,
    type AudioSegmentRecord,
    type AudioTranscriptStatus,
    type DrawerShortcut,
    type SpeakerInlineAction,
    type SpeakerTranscriptGroup,
  } from "./audio-drawer-view";
  import type {
    PersonProfileDto,
    SpeakerAnalysisProvenance,
    SpeakerClusterDto,
    SpeakerTurnDto,
    TranscriptionSegment,
    TranscriptionWord,
  } from "$lib/types/app-infra";

  interface Props {
    segment: AudioSegmentRecord;
    sourceLabel: string;
    timeRangeLabel: string;
    timeRangeTip: string;
    durationLabel: string;

    audioSrc: string | null;
    mediaLoading: boolean;
    mediaError: string | null;
    loadError: string | null;
    onMediaError: () => void;

    transcriptStatus: AudioTranscriptStatus;
    transcriptText: string | null;
    transcriptSegments: TranscriptionSegment[];
    /** `TranscriptionStructuredPayload.words[]` — the karaoke source. */
    transcriptWords: TranscriptionWord[];
    transcriptModelLabel: string | null;
    transcriptError: string | null;
    transcriptRerunLoading: boolean;
    transcriptRerunError: string | null;
    transcriptActionLabel: string;
    transcriptActionDisabled: boolean;
    transcriptActionTitle: string;
    onRerunTranscript: () => void;

    turns: SpeakerTurnDto[];
    clusters: SpeakerClusterDto[];
    profiles: PersonProfileDto[];
    speakerTurnsError: string | null;
    speakerTurnsNotice: string | null;
    speakerAnalysisRunning: boolean;
    speakerAnalysisFailed: boolean;
    speakerRetryDisabled: boolean;
    speakerRetryLoading: boolean;
    speakerProvenance: SpeakerAnalysisProvenance | null;
    /** The queued/running job the "still working" panel reports on. */
    pendingJob: { processor: string; queuedAt: string } | null;
    onRetrySpeakerAnalysis: () => void;

    correctionError: string | null;
    /**
     * The cluster a speaker write is in flight for, or null. The route serialises
     * these writes, so a non-null value disables *every* speaker control — one
     * left enabled would swallow the click instead of acting on it.
     */
    busyClusterId: number | null;
    inlineBusy: { clusterId: number; action: SpeakerInlineAction } | null;
    developerMode: boolean;

    onClose: () => void;
    onApplyName: (group: SpeakerTranscriptGroup, name: string) => void;
    onLinkPerson: (clusterId: number, personId: number) => void;
    onConfirmSuggestion: (group: SpeakerTranscriptGroup) => void;
    onNotThisPerson: (group: SpeakerTranscriptGroup) => void;
    onMergeClusters: (sourceClusterId: number, targetClusterId: number) => void;
    onMoveGroup: (group: SpeakerTranscriptGroup, targetClusterId: number) => void;
    /** Matched drawer shortcut, or null. Suppression inside inputs is the page's call. */
    shortcutFor: (event: KeyboardEvent) => DrawerShortcut | null;

    /** A one-shot in-segment seek requested from the timeline. */
    pendingSeekMs?: number | null;
    /** Which reader turn the repair slide-over is pointed at. Bound out so the
     *  page's outside-click / Escape policy can collapse it before it closes the
     *  whole drawer. */
    repairIndex?: number | null;
    /** The drawer element, for the page's inside/outside dismissal test. */
    drawerEl?: HTMLDivElement | null;
    /** The scroll container, so the page can preserve position across a refresh. */
    transcriptContainerEl?: HTMLDivElement | null;
  }

  let {
    segment,
    sourceLabel,
    timeRangeLabel,
    timeRangeTip,
    durationLabel,
    audioSrc,
    mediaLoading,
    mediaError,
    loadError,
    onMediaError,
    transcriptStatus,
    transcriptText,
    transcriptSegments,
    transcriptWords,
    transcriptModelLabel,
    transcriptError,
    transcriptRerunLoading,
    transcriptRerunError,
    transcriptActionLabel,
    transcriptActionDisabled,
    transcriptActionTitle,
    onRerunTranscript,
    turns,
    clusters,
    profiles,
    speakerTurnsError,
    speakerTurnsNotice,
    speakerAnalysisRunning,
    speakerAnalysisFailed,
    speakerRetryDisabled,
    speakerRetryLoading,
    speakerProvenance,
    pendingJob,
    onRetrySpeakerAnalysis,
    correctionError,
    busyClusterId,
    inlineBusy,
    developerMode,
    onClose,
    onApplyName,
    onLinkPerson,
    onConfirmSuggestion,
    onNotThisPerson,
    onMergeClusters,
    onMoveGroup,
    shortcutFor,
    pendingSeekMs = $bindable(null),
    repairIndex = $bindable(null),
    drawerEl = $bindable(null),
    transcriptContainerEl = $bindable(null),
  }: Props = $props();

  // The per-segment reset key. It MUST be the id, not `segment` itself: the page
  // resolves `segment` out of `audioSegments`, and `refreshAudioSegments()` (the
  // ~1.5s head poll while capture runs) reassigns that array with freshly mapped
  // DTO objects, so the same segment arrives as a new object identity. Reading
  // `segment.id` inside an effect subscribes to that identity; reading a
  // `$derived` number does not, because a derived only propagates when its value
  // actually changes. Without this every poll wipes `duration` (which only
  // `loadedmetadata` ever restores, so the scrubber dies for good), resets the
  // transport mid-playback, closes the repair panel mid-edit, and steals focus.
  const segmentId = $derived(segment.id);

  // Every OTHER read off the row object needs the same treatment, for the same
  // reason. `transcriptFallbackGroups(..., segment.durationSeconds)` returns a
  // fresh array, and it IS the reader's `groups` whenever diarization has no turns
  // yet — the routine state while capture runs (transcription lands first), and the
  // permanent state when speakrs yields zero turns. A new `groups` identity then
  // re-derives every paragraph's karaoke word list (each a full scan of `words[]`
  // plus a sort, a join and two regex passes), rebuilds the speaker-mark map, and
  // rebuilds all 150 waveform bars: measured at ~20ms per poll for a 5-minute
  // 800-word segment, four times a minute for as long as the drawer stays open.
  // A `$derived` scalar only propagates when its VALUE changes.
  const segmentSource = $derived(segment.source);
  const segmentDurationSeconds = $derived(segment.durationSeconds);

  // ── drawer view state ─────────────────────────────────────────────────────
  let expanded = $state(false);
  let showTimestamps = $state(false);
  let followDetached = $state(false);
  /** "Read without speakers": ignore a failed speaker pass and read the words. */
  let ignoreSpeakerFailure = $state(false);
  let closeEl = $state<HTMLButtonElement | null>(null);
  let repairEl = $state<HTMLElement | null>(null);
  let returnFocusEl: HTMLElement | null = null;

  $effect(() => {
    // Reset per-segment view state, but keep the user's density preferences
    // (expanded / timestamps) — those are about how they read, not what.
    void segmentId;
    followDetached = false;
    repairIndex = null;
    ignoreSpeakerFailure = false;
  });

  // ── audio element + transport ─────────────────────────────────────────────
  const transport = new AudioTransport(() => onMediaError());

  $effect(() => {
    void segmentId;
    transport.reset();
  });

  const currentMs = $derived(transport.currentMs);

  /** A seek the user asked for by pointing at a word re-attaches the reader:
   *  they just told us where they want to be. `seekBySeconds` and the scrubber
   *  deliberately do not, so scrubbing while reading elsewhere stays possible. */
  function seekToMs(startMs: number): void {
    transport.seekToMs(startMs);
    followDetached = false;
  }

  // A one-shot seek handed down from the timeline (play-this-moment).
  $effect(() => {
    const seekMs = pendingSeekMs;
    const el = transport.element;
    if (seekMs == null || !el || audioSrc == null) return;
    const applySeek = () => {
      seekToMs(seekMs);
      pendingSeekMs = null;
    };
    if (el.readyState >= 1) {
      applySeek();
      return;
    }
    el.addEventListener("loadedmetadata", applySeek, { once: true });
    return () => el.removeEventListener("loadedmetadata", applySeek);
  });

  const peaks = waveformPeaks(() => segmentId);

  // ── the reading model ─────────────────────────────────────────────────────
  const speakerGroups = $derived(buildSpeakerGroups(turns, clusters));

  const fallbackGroups = $derived(
    transcriptFallbackGroups(transcriptSegments, transcriptText, segmentDurationSeconds),
  );

  const groups = $derived(speakerGroups.length > 0 ? speakerGroups : fallbackGroups);
  const marks = $derived(assignSpeakerMarks(groups.map((g) => g.clusterId)));
  const distinctSpeakers = $derived(new Set(speakerGroups.map((g) => g.clusterId)).size);

  const activeGroupIndex = $derived(
    resolveActiveGroupIndex(groups, currentMs, transport.engaged),
  );

  const repairGroup = $derived(repairIndex == null ? null : groups[repairIndex] ?? null);

  // The slide-over is aimed by INDEX, but `groups` is rebuilt from `turns` by every
  // speaker write, and `buildSpeakerGroups` collapses consecutive same-cluster turns
  // — so a merge deletes a paragraph and every index below it shifts up by one. The
  // merge button lives INSIDE this panel, which makes that the common path: left
  // alone the panel stays open silently aimed at the next voice down, and the user's
  // next name / unlink / move lands on a speaker they never opened — writes with no
  // backend inverse. Follow the cluster instead, and close if it is gone.
  let repairPin: { index: number; clusterId: number } | null = null;
  $effect(() => {
    const index = repairIndex;
    const group = repairGroup;
    const pin = repairPin;
    if (index == null || group == null) {
      repairPin = null;
      return;
    }
    if (pin == null || pin.index !== index) {
      repairPin = { index, clusterId: group.clusterId };
      return;
    }
    if (pin.clusterId === group.clusterId) return;
    const next = groups.findIndex((candidate) => candidate.clusterId === pin.clusterId);
    repairPin = next < 0 ? null : { index: next, clusterId: pin.clusterId };
    repairIndex = next < 0 ? null : next;
  });
  const unnamedRemaining = $derived(
    new Set(
      speakerGroups.filter((g) => g.personId == null).map((g) => g.clusterId),
    ).size,
  );

  const status = $derived(
    drawerStatusPill({
      source: segmentSource,
      transcriptStatus,
      speakerAnalysisRunning,
      speakerAnalysisFailed,
      distinctSpeakers,
    }),
  );

  const panel = $derived(
    drawerPanelKind({
      transcriptStatus,
      speakerAnalysisFailed,
      ignoreSpeakerFailure,
      groupCount: groups.length,
    }),
  );

  const processingFootnote = $derived.by(() => {
    if (!pendingJob) return "";
    const stage = processingStageLabel(
      segmentSource,
      pendingJob.processor as "audio_transcription",
    );
    const ago = queuedAgoLabel(pendingJob.queuedAt);
    return [stage, ago].filter(Boolean).join(" · ");
  });

  // ── a11y: focus in on open, restore on close, trap Tab while open ─────────
  $effect(() => {
    void segmentId;
    returnFocusEl ??= document.activeElement as HTMLElement | null;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled) return;
      (closeEl ?? drawerEl)?.focus();
    });
    return () => {
      cancelled = true;
    };
  });

  $effect(() => () => {
    const active = document.activeElement as HTMLElement | null;
    if (!active || active === document.body || drawerEl?.contains(active)) {
      returnFocusEl?.focus({ preventScroll: true });
    }
  });

  // Move focus into the repair panel when it opens, back to the gutter on close.
  $effect(() => {
    if (repairIndex == null) return;
    let cancelled = false;
    void tick().then(() => {
      if (cancelled || repairIndex == null) return;
      (getFocusableElements(repairEl)[0] ?? repairEl)?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      // Focus was moved INTO a panel that is now being removed. Without this it
      // falls to <body>, and every drawer key (Escape, the transport shortcuts,
      // the Tab trap) is handled on the drawer element — so they all go dead
      // until the user happens to click back inside.
      const active = document.activeElement as HTMLElement | null;
      if (!active || active === document.body || repairEl?.contains(active)) {
        (closeEl ?? drawerEl)?.focus({ preventScroll: true });
      }
    };
  });

  function closeRepair(): void {
    repairIndex = null;
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      // Layered: the repair panel first, then follow mode, then the drawer.
      if (repairIndex != null) closeRepair();
      else if (followDetached) followDetached = false;
      else onClose();
      return;
    }
    const shortcut = shortcutFor(event);
    if (shortcut === "playPause") {
      event.preventDefault();
      transport.togglePlayPause();
      return;
    }
    if (shortcut) {
      event.preventDefault();
      transport.seekBySeconds(
        shortcut === "seekBackFast"
          ? -30
          : shortcut === "seekForwardFast"
            ? 30
            : shortcut === "seekBack"
              ? -5
              : 5,
      );
      return;
    }
    trapTabKey(event, drawerEl);
  }

  const waveBars = $derived(
    waveformBars(peaks.value, transport.duration * 1000, turns, marks),
  );
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="audio-drawer"
  class:audio-drawer--expanded={expanded}
  role="dialog"
  aria-modal="false"
  aria-label={`Audio segment player — ${sourceLabel} #${segment.segmentIndex}`}
  tabindex="-1"
  bind:this={drawerEl}
  onkeydown={onKeydown}
>
  <DrawerHeader
    {segment}
    {sourceLabel}
    {timeRangeLabel}
    {timeRangeTip}
    {durationLabel}
    modelLabel={transcriptModelLabel}
    {status}
    actionLabel={transcriptActionLabel}
    actionDisabled={transcriptActionDisabled}
    actionTitle={transcriptActionTitle}
    rerunLoading={transcriptRerunLoading}
    onRerun={onRerunTranscript}
    {onClose}
    bind:showTimestamps
    bind:expanded
    bind:closeEl
  />

  {#if mediaError}
    <div class="drawer-error" role="alert">
      <span class="drawer-error__label">playback unavailable</span>
      <span class="drawer-error__msg">{mediaError}</span>
    </div>
  {/if}
  {#if loadError}
    <div class="drawer-error" role="alert">
      <span class="drawer-error__label">playback error</span>
      <span class="drawer-error__msg">{loadError}</span>
    </div>
  {/if}
  {#if transcriptRerunError}
    <div class="drawer-error" role="alert">
      <span class="drawer-error__label">rerun failed</span>
      <span class="drawer-error__msg">{transcriptRerunError}</span>
    </div>
  {/if}

  {#if audioSrc}
    {#key segmentId}
      <audio
        class="audio-drawer__native"
        preload="metadata"
        src={audioSrc}
        bind:this={transport.element}
        onerror={onMediaError}
        ontimeupdate={transport.onTimeUpdate}
        onloadedmetadata={transport.onLoadedMetadata}
        ondurationchange={transport.onLoadedMetadata}
        onplay={() => (transport.isPlaying = true)}
        onpause={() => (transport.isPlaying = false)}
        onended={transport.onEnded}
        aria-hidden="true"
      ></audio>
    {/key}
  {/if}

  <div class="stage" class:stage--repair={repairGroup}>
    {#if panel === "reader"}
      <TranscriptReader
        {groups}
        {marks}
        words={transcriptWords}
        segments={transcriptSegments}
        {currentMs}
        {activeGroupIndex}
        {showTimestamps}
        {expanded}
        speakerName={(group) =>
          group.clusterId < 0 ? group.speakerLabel : speakerPersistedName(group, profiles)}
        isUnnamed={(group) =>
          group.clusterId < 0 || speakerIsUnnamed(group, profiles)}
        needsAttention={(group) =>
          group.clusterId >= 0 &&
          (group.personId == null || group.suggestedPersonId != null)}
        repairable={(group) => group.clusterId >= 0}
        suggestionFor={(group, index) =>
          suggestionChipFor(groups, group, index, profiles, developerMode)}
        suggestionBusy={(group) =>
          busyClusterId !== null ||
          (inlineBusy?.clusterId === group.clusterId && inlineBusy.action === "confirm")}
        {onConfirmSuggestion}
        onSeekMs={seekToMs}
        onOpenRepair={(index) => {
          if ((groups[index]?.clusterId ?? -1) < 0) return;
          repairIndex = index;
        }}
        bind:followDetached
        bind:containerEl={transcriptContainerEl}
      />
      <!-- The reader IS reachable with `speakerTurnsError` set: the page's
           `loadSelectedAudioSpeakerTurns` catch clears the failed-job id and keeps
           the error, so `groups` falls back to the transcript's own runs, the panel
           is "reader", and DrawerStatePanels — the only other place this renders —
           never mounts. Without this a SQLITE_BUSY `list_speaker_turns` reads as
           "nobody spoke". Error outranks notice, as the old guard implied. -->
      {#if speakerTurnsError}
        <p class="stage__error" role="alert">{speakerTurnsError}</p>
      {:else if speakerTurnsNotice}
        <p class="stage__note">{speakerTurnsNotice}</p>
      {/if}
      {#if correctionError}
        <p class="stage__error" role="alert">{correctionError}</p>
      {/if}
    {:else}
      <DrawerStatePanels
        {panel}
        {durationLabel}
        {processingFootnote}
        provenanceFootnote={provenanceFootnote(speakerProvenance)}
        noSpeechNotice={speakerTurnsNotice ?? ""}
        {transcriptError}
        {speakerTurnsError}
        rerunDisabled={transcriptActionDisabled}
        rerunLoading={transcriptRerunLoading}
        {speakerRetryDisabled}
        {speakerRetryLoading}
        onPlayAnyway={transport.togglePlayPause}
        onRerun={onRerunTranscript}
        onRetrySpeakers={onRetrySpeakerAnalysis}
        onReadWithoutSpeakers={() => (ignoreSpeakerFailure = true)}
      />
    {/if}

    {#if repairGroup}
      <div bind:this={repairEl}>
        <SpeakerRepairPanel
          group={repairGroup}
          mark={marks.get(repairGroup.clusterId)}
          {turns}
          {clusters}
          {profiles}
          persistedName={speakerPersistedName(repairGroup, profiles)}
          unnamed={speakerIsUnnamed(repairGroup, profiles)}
          busy={busyClusterId !== null}
          error={correctionError}
          unnamedRemaining={Math.max(
            0,
            unnamedRemaining - (repairGroup.personId == null ? 1 : 0),
          )}
          linkedPersonName={speakerProfileName(profiles, repairGroup.personId)}
          suggestionPending={repairGroup.personId == null &&
            repairGroup.suggestedPersonId != null}
          mergeTargetLabel={suggestedMergeTargetLabel(repairGroup, clusters, profiles)}
          mergeScoreLabel={developerMode ? formatScore(repairGroup.suggestedMergeScore) : null}
          clusterOptionLabel={(cluster) => speakerClusterOptionLabel(cluster, profiles)}
          onClose={closeRepair}
          onApplyName={(name) => onApplyName(repairGroup, name)}
          onLink={(personId) => onLinkPerson(repairGroup.clusterId, personId)}
          onMerge={() => {
            const target = repairGroup.suggestedMergeTargetClusterId;
            if (target != null) onMergeClusters(repairGroup.clusterId, target);
          }}
          onNotThisPerson={() => onNotThisPerson(repairGroup)}
          onMoveGroupTo={(target) => onMoveGroup(repairGroup, target)}
          onPlaySamples={() => transport.playSamples(samplePreviewHeadsMs(repairGroup, turns))}
        />
      </div>
    {/if}
  </div>

  <DrawerTransport
    isPlaying={transport.isPlaying}
    currentTime={transport.currentTime}
    duration={transport.duration}
    playable={audioSrc != null}
    {mediaLoading}
    bars={waveBars}
    compact={!expanded}
    onToggle={transport.togglePlayPause}
    onScrubInput={(event) => {
      transport.scrubbing = true;
      transport.currentTime = Number((event.currentTarget as HTMLInputElement).value);
    }}
    onScrubChange={(event) => {
      transport.scrubbing = false;
      transport.seekToSeconds(Number((event.currentTarget as HTMLInputElement).value));
    }}
  />

  {#if !expanded && panel === "reader"}
    <button type="button" class="expand-cta" onclick={() => (expanded = true)}>
      open reader ⤢
    </button>
  {/if}
</div>

<style>
  /* A panel that slides up out of the window's bottom edge: inset 12px on the
     sides, flush to the bottom, so only its top corners are round. */
  .audio-drawer {
    position: fixed;
    left: 12px;
    right: 12px;
    bottom: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    max-height: 50vh;
    overflow: hidden;
    background: var(--app-surface);
    border: 0;
    border-radius: var(--r-xl) var(--r-xl) 0 0;
    box-shadow:
      var(--app-shadow-popover),
      0 0 0 var(--hairline) var(--app-border-strong);
    animation: audio-drawer-rise 180ms cubic-bezier(0.2, 0.7, 0.2, 1);
    outline: none;
  }

  /* Fifty vertical percent cannot host a 4-minute conversation and a generous
     measure at the same time; the expanded reader lifts the ceiling. It covers
     the timeline — accepted tradeoff, peek is the mitigation. */
  .audio-drawer--expanded {
    /* Below the app titlebar, which is fixed and would otherwise cover the
       drawer's own header row (rerun / timestamps / collapse / close). */
    top: calc(var(--app-titlebar-height) + 8px);
    max-height: none;
  }

  .audio-drawer:focus-visible {
    box-shadow:
      var(--app-shadow-popover),
      0 0 0 var(--hairline) var(--app-accent),
      var(--app-ring);
  }

  @keyframes audio-drawer-rise {
    from {
      transform: translateY(12px);
      opacity: 0;
    }
    to {
      transform: translateY(0);
      opacity: 1;
    }
  }

  .audio-drawer__native {
    display: none;
  }

  /* ── stage: the reader or a state panel, plus the repair slide-over ─────── */
  .stage {
    position: relative;
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  /* The slide-over is absolute over the stage, so reserve its width (330px + a
     12px gutter) while it is open or it covers the right edge of every line. */
  .stage--repair {
    padding-right: 342px;
  }

  .stage__note,
  .stage__error {
    margin: 0;
    padding: 4px 14px 8px;
    font-size: 11px;
    line-height: 1.5;
  }

  .stage__note {
    color: var(--app-text-muted);
    font-style: italic;
  }

  .stage__error {
    color: var(--app-danger-text, var(--app-danger));
    font-family: var(--app-font-mono);
    word-break: break-word;
  }

  /* AUDIT 4 — neutral, so the accent stays reserved for the playhead + word. */
  .expand-cta {
    position: absolute;
    right: 14px;
    bottom: 52px;
    padding: 4px 10px;
    border: 1px solid var(--app-border-hover, var(--app-border-strong));
    border-radius: 999px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover);
    color: var(--app-text-strong);
    font: inherit;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    cursor: pointer;
  }

  .expand-cta:hover,
  .expand-cta:focus-visible {
    background: var(--app-surface-hover);
    outline: none;
  }

  .drawer-error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin: 8px 12px 0;
    padding: 8px 10px;
    border: 1px solid var(--app-danger-border);
    border-radius: 4px;
    background: var(--app-danger-bg-soft, transparent);
    font-size: 11px;
    color: var(--app-danger-text, var(--app-danger));
  }

  .drawer-error__label {
    flex: none;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-danger);
  }

  .drawer-error__msg {
    flex: 1 1 auto;
    font-family: var(--app-font-mono);
    line-height: 1.4;
    word-break: break-word;
  }

  @media (prefers-reduced-motion: reduce) {
    .audio-drawer {
      animation: none;
    }
  }
</style>
