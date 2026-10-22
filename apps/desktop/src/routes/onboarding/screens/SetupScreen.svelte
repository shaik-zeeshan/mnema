<!--
  Screen 6 / 8 — Setup (issue #195, slice 9). Downloads, and the gate that no
  longer exists.

  CONTRACT (unchanged from the shell's placeholder)
    props       flow · onContinue · onBack
    owns        Non-blocking downloads, per-item state, the real error text AT
                the failed item, cancel semantics, the free-disk preflight.
    must not    Disable Continue. EVER. Not while downloading, not on a failure,
                not on a cancel, not on an empty work-list. A download in flight
                used to block the whole flow; that is the bug this issue is for.
    gates       NONE. `flow.canContinue` is true here by construction, and the
                Continue button below never reads it — it has no `disabled`
                binding at all, which is the strongest form of "never disables".

  WHY THE PLUMBING LOOKS LIKE THIS
    · The four per-subsystem download commands already exist; this screen only
      drives them. No new Rust.
    · It subscribes to the same four progress events `onboarding-listeners.ts`
      uses (same constants, same payload shape) rather than reading them off the
      controller: `semanticSearchDownloadProgress` is not re-exported there, and
      a second Tauri listener is free.
    · The fold is `$lib/onboarding/model-readiness` verbatim — `startProgress`
      then `applyProgressEvent`. Monotonic bytes, out-of-order tolerance and the
      four-state classification all live in that (tested) reducer.
    · Items run SEQUENTIALLY in the resolver's fixed order (speakrs → Whisper
      base → nomic). Speakrs first is deliberate: the Voice step next door cannot
      run its embedder without it.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { describeError, formatBytes } from "$lib/settings/state/format";
  import type { FeatureId } from "$lib/onboarding/feature-rules";
  import {
    applyProgressEvent,
    reseedProgress,
    startProgress,
    type DownloadProgressEvent,
    type DownloadProgressState,
  } from "$lib/onboarding/model-readiness";
  import type { DownloadSubsystem, DownloadWorkItem } from "$lib/onboarding/resolve-setup";
  import {
    AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
    OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
    SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
    SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
  } from "../onboarding-mapping";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconCheck from "~icons/lucide/check";
  import IconDisk from "~icons/lucide/hard-drive";
  import IconRetry from "~icons/lucide/rotate-ccw";

  let {
    flow,
    onContinue,
    onBack,
  }: { flow: OnboardingFlow; onContinue: () => void; onBack: () => void } = $props();

  // ── The four existing per-subsystem commands, keyed by work-item subsystem ──
  const START: Record<DownloadSubsystem, string> = {
    ocr: "start_ocr_model_download",
    audioTranscription: "start_audio_transcription_model_download",
    speakerAnalysis: "start_speaker_analysis_model_download",
    semanticSearch: "start_semantic_search_model_download",
  };
  const CANCEL: Record<DownloadSubsystem, string> = {
    ocr: "cancel_ocr_model_download",
    audioTranscription: "cancel_audio_transcription_model_download",
    speakerAnalysis: "cancel_speaker_analysis_model_download",
    semanticSearch: "cancel_semantic_search_model_download",
  };
  const PROGRESS_EVENTS = [
    OCR_MODEL_DOWNLOAD_PROGRESS_EVENT,
    AUDIO_TRANSCRIPTION_MODEL_DOWNLOAD_PROGRESS_EVENT,
    SPEAKER_ANALYSIS_MODEL_DOWNLOAD_PROGRESS_EVENT,
    SEMANTIC_SEARCH_MODEL_DOWNLOAD_PROGRESS_EVENT,
  ];
  const TERMINAL = ["completed", "failed", "cancelled"];

  // Rows are named by the FEATURE they serve, not the model. Model identifiers
  // live only on *Change settings* (density rule) — "pyannote community-1 +
  // WeSpeaker (CoreML)" is exactly the string that rule exists to keep out.
  const FEATURE_NAME: Record<FeatureId, string> = {
    screen: "Screen capture",
    microphone: "Microphone",
    systemAudio: "System audio",
    ocr: "Text in screenshots",
    transcription: "Transcription",
    speakerSeparation: "Who's speaking",
    semanticSearch: "Semantic Search",
    aiFeatures: "AI features",
    privacy: "Privacy",
  };

  // ── State ────────────────────────────────────────────────────────────────
  let progress = $state<DownloadProgressState>(startProgress([]));
  /** Items already started once. A retry / "turn it back on" removes the id. */
  let attempted = $state<string[]>([]);
  /** The one item whose download command has been issued and not yet settled. */
  let inFlight = $state<string | null>(null);
  /** The user chose to download anyway after a measured disk shortfall. */
  let diskOverride = $state(false);

  const workList = $derived(flow.workList);
  /** Cancelling turns the dependent feature off, which drops its item out of
   *  the running total — the "total drops to …" behaviour from the mockup. */
  const active = $derived(workList.filter((item) => flow.features[item.feature]));
  const activeBytes = $derived(active.reduce((sum, item) => sum + item.bytes, 0));
  const activeReceived = $derived(
    active.reduce((sum, item) => sum + (progress.received[item.id] ?? 0), 0),
  );
  // The reducer's own `percent` is byte-weighted over the WHOLE work-list and is
  // monotonic, so it cannot shrink when a cancel removes an item from the total.
  // Same arithmetic over the active subset, off the reducer's monotonic
  // `received`, so it stays monotonic for any fixed set of items.
  const percent = $derived(
    activeBytes > 0 ? Math.min(100, Math.floor((activeReceived / activeBytes) * 100)) : 100,
  );
  // Any total containing nomic is approximate — the Rust figure it mirrors is
  // `approx_download_bytes`, not a per-file sum.
  const approx = $derived(active.some((item) => item.subsystem === "semanticSearch"));
  const totalLabel = $derived(`${approx ? "~" : ""}${formatBytes(activeBytes)}`);
  const currentItem = $derived(
    active.find((item) => item.label === progress.currentLabel) ?? null,
  );
  const downloadingNow = $derived(
    currentItem && progress.states[currentItem.id] === "downloading" ? currentItem : null,
  );
  const failedCount = $derived(
    active.filter((item) => progress.states[item.id] === "failed").length,
  );

  // ── Free-disk preflight — before the work-list starts, not per item ───────
  // Measured by *Capture & Storage* (its probe is the only writer). An inability
  // to measure never blocks: only a measured shortfall acts (ADR 0040).
  const shortfall = $derived.by(() => {
    const free = flow.storageProbe?.freeBytes;
    if (free === undefined || free === null || free >= activeBytes) return null;
    return `${formatBytes(free)} free · ${totalLabel} needed. Free some space, or start them anyway.`;
  });
  const mayStart = $derived(shortfall === null || diskOverride);

  // ── Wiring ───────────────────────────────────────────────────────────────
  // The work-list is LIVE — the user can back out to *Change settings*, pick a
  // different model, and return to a different agenda. `reseedProgress` carries
  // the surviving items' progress across and is identity when nothing moved, so
  // this never wipes a download in flight and never loops.
  $effect(() => {
    const items = workList;
    untrack(() => {
      progress = reseedProgress(progress, items);
      // An item that left and came back is a fresh attempt.
      attempted = attempted.filter((id) => items.some((item) => item.id === id));
    });
  });

  $effect(() => {
    let dead = false;
    const unlisten: Array<() => void> = [];
    for (const name of PROGRESS_EVENTS) {
      void listen<DownloadProgressEvent>(name, (event) => {
        progress = applyProgressEvent(progress, event.payload);
        if (!TERMINAL.includes(event.payload.status)) return;
        const item = matchItem(event.payload);
        if (item && inFlight === item.id) inFlight = null;
      }).then((fn) => {
        if (dead) fn();
        else unlisten.push(fn);
      });
    }
    return () => {
      dead = true;
      for (const fn of unlisten) fn();
    };
  });

  // The sequential driver: one download at a time, in the resolver's order.
  $effect(() => {
    if (!mayStart) return;
    // A cascade may have switched the in-flight item's feature off; don't stall.
    if (inFlight && active.some((item) => item.id === inFlight)) return;
    const next = active.find(
      (item) => progress.states[item.id] !== "ready" && !attempted.includes(item.id),
    );
    if (!next) return;
    attempted = [...attempted, next.id];
    inFlight = next.id;
    void startItem(next);
  });

  function matchItem(event: DownloadProgressEvent): DownloadWorkItem | null {
    return (
      workList.find(
        (item) => item.provider === event.provider && item.modelId === event.modelId,
      ) ?? null
    );
  }

  async function startItem(item: DownloadWorkItem): Promise<void> {
    try {
      await invoke(START[item.subsystem], {
        request: { provider: item.provider, modelId: item.modelId },
      });
    } catch (err) {
      // A synchronous rejection never reaches the progress stream, so fold it in
      // as a failure — the row shows the REAL reason, which is the one thing
      // this screen exists for.
      progress = applyProgressEvent(progress, {
        provider: item.provider,
        modelId: item.modelId,
        status: "failed",
        downloadedBytes: 0,
        totalBytes: item.bytes,
        message: describeError(err),
      });
      if (inFlight === item.id) inFlight = null;
    }
  }

  /**
   * Cancel disables the dependent PROCESSING feature and leaves the capture
   * source alone: cancelling Whisper turns transcription off, never the
   * microphone — that audio is still worth keeping and becomes transcribable
   * later.
   */
  async function cancelItem(item: DownloadWorkItem): Promise<void> {
    if (flow.features[item.feature]) flow.toggleFeature(item.feature);
    await cancelSubsystem(item);
    // `applyToggle` cascades (turning transcription off also drops speaker
    // separation), so a sibling download may have just lost its reason to run.
    for (const other of workList) {
      if (other.id === item.id || flow.features[other.feature]) continue;
      if (progress.states[other.id] === "downloading") await cancelSubsystem(other);
    }
  }

  async function cancelSubsystem(item: DownloadWorkItem): Promise<void> {
    try {
      await invoke(CANCEL[item.subsystem]);
    } catch {
      // Cancelling a download that already settled is not worth a message; the
      // feature is off either way, which is what the user asked for.
    }
    if (inFlight === item.id) inFlight = null;
  }

  function retryItem(item: DownloadWorkItem): void {
    attempted = attempted.filter((id) => id !== item.id);
  }

  function restoreItem(item: DownloadWorkItem): void {
    // Speaker separation is locked while transcription is off (`featureLockReason`),
    // so restoring it alone would be a silent no-op — and cancelling Whisper is
    // exactly what cascaded it off in the first place.
    if (item.feature === "speakerSeparation" && !flow.features.transcription) {
      flow.toggleFeature("transcription");
    }
    if (!flow.features[item.feature]) flow.toggleFeature(item.feature);
    retryItem(item);
  }

  function itemPercent(item: DownloadWorkItem): number {
    // A catalog model with no declared size carries 0 bytes — show it as pending
    // until its terminal event lands rather than dividing by zero.
    if (item.bytes <= 0) return 0;
    return Math.min(100, Math.round(((progress.received[item.id] ?? 0) / item.bytes) * 100));
  }
</script>

<div class="ob-main">
  <div class="ob-col">
    <h1 class="mx-kicker">Setup</h1>
    <div class="ob-head">
      <div class="ob-pct" aria-hidden="true">{percent}<small>%</small></div>
      <!-- Deliberately NOT a live region: the byte figure changes on every chunk
           event, so `role="status"` would announce it dozens of times a second. -->
      <p class="ob-line">
        {#if workList.length === 0}
          Nothing to download — everything you need is already here.
        {:else if !mayStart}
          Downloads are held until there is room · <span class="num">{totalLabel}</span> to fetch.
        {:else if downloadingNow}
          Downloading <b>{FEATURE_NAME[downloadingNow.feature]}</b> ·
          <span class="num">{formatBytes(activeReceived)} of {totalLabel}</span>
        {:else if active.length === 0}
          Every download was cancelled.
        {:else if progress.done}
          All set · <span class="num">{totalLabel}</span> downloaded.
        {:else if failedCount > 0}
          {failedCount === 1 ? "One download" : `${failedCount} downloads`} didn't finish · the rest carry on.
        {:else}
          Starting <b>{FEATURE_NAME[active[0].feature]}</b> · <span class="num">{totalLabel}</span> to fetch.
        {/if}
      </p>
    </div>

    {#if shortfall && !diskOverride}
      <div class="mx-notice" data-tone="warn">
        <span class="mx-notice__icon"><IconDisk width="15" height="15" /></span>
        <b class="mx-notice__title">Not enough room for the downloads</b>
        <p class="mx-notice__text">{shortfall}</p>
        <div class="mx-notice__acts">
          <button class="mx-btn mx-btn--sm" onclick={() => (diskOverride = true)}>Download anyway</button>
        </div>
      </div>
    {/if}

    <div class="ob-work">
      {#each workList as item (item.id)}
        {@const off = !flow.features[item.feature]}
        {@const state = progress.states[item.id] ?? "missing"}
        <div class="ob-item" class:off={off && state !== "ready"}>
          <span class="ob-item__name">
            {FEATURE_NAME[item.feature]}<small>{item.subsystem === "semanticSearch" ? "about " : ""}{formatBytes(item.bytes)}</small>
          </span>
          {#if state === "ready"}
            <!-- Ready wins over a switched-off feature: the bytes ARE on disk. -->
            <span class="ob-item__st ready"><IconCheck width="14" height="14" />Ready</span>
          {:else if off}
            <span class="ob-item__st warn">
              Turned off
              <button class="mx-btn mx-btn--sm" onclick={() => restoreItem(item)}>Turn it back on</button>
            </span>
          {:else if state === "downloading"}
            <span class="ob-item__st">
              Downloading
              <button class="mx-btn mx-btn--sm" onclick={() => void cancelItem(item)}>Cancel</button>
            </span>
          {:else if state === "failed"}
            <span class="mx-inline" data-tone="danger">
              <IconAlert width="13" height="13" />{progress.errors[item.id] ?? "Download failed."}
              <button class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => retryItem(item)}>
                <IconRetry width="12" height="12" />Retry
              </button>
            </span>
          {:else}
            <span class="ob-item__st">{mayStart ? "Queued" : "Held"}</span>
          {/if}
          {#if !off && (state === "downloading" || (state === "failed" && itemPercent(item) > 0))}
            <div class="ob-track" class:stalled={state === "failed"}>
              <b style:width="{itemPercent(item)}%"></b>
            </div>
          {/if}
          {#if !off && state === "failed"}
            <p class="ob-item__why">
              {FEATURE_NAME[item.feature]} stays off until this arrives. Continue still works.
            </p>
          {/if}
        </div>
      {:else}
        <p class="ob-fine">Every model your settings need is already on this machine.</p>
      {/each}
    </div>
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note">Live since you arrived, and it never disables. Downloads carry on behind you.</p>
  <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
  <button class="mx-btn mx-btn--primary" onclick={onContinue}>
    Continue<IconArrow class="ob-btn-arrow" width="14" height="14" />
  </button>
</footer>

<style>
  .ob-pct {
    font: 300 72px / 0.9 var(--font-mono);
    letter-spacing: -0.06em;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .ob-pct small {
    font-size: 28px;
    color: var(--app-text-subtle);
    letter-spacing: -0.02em;
  }
  .ob-line {
    margin: 0;
    font: 400 15px / 1.4 var(--font-sans);
    color: var(--app-text-muted);
  }
  .ob-line b {
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .ob-line .num {
    color: var(--app-text-strong);
  }
  .mx-notice {
    max-width: 620px;
  }
  .ob-work {
    display: grid;
    border-top: 1px solid var(--mx-hairline);
    max-width: 620px;
  }
  .ob-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    column-gap: var(--s-3);
    row-gap: 8px;
    padding: 12px 0;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .ob-item__name {
    font: 500 var(--text-md) / 1 var(--font-sans);
    color: var(--app-text-strong);
  }
  .ob-item__name small {
    margin-left: var(--s-2);
    font: 400 var(--text-base) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .ob-item.off .ob-item__name {
    color: var(--app-text-subtle);
  }
  .ob-item__st {
    display: inline-flex;
    align-items: center;
    gap: var(--s-2);
    font: 400 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ob-item__st.ready {
    color: var(--app-accent);
  }
  .ob-item__st.warn {
    color: var(--app-warn);
  }
  .ob-item .mx-inline {
    justify-self: end;
  }
  .ob-item__why {
    grid-column: 1 / -1;
    margin: -2px 0 0;
    font: 400 var(--text-base) / 1.4 var(--font-sans);
    color: var(--app-text-subtle);
  }
  /* Real progress, not ambient motion: no sheen, no loop. */
  .ob-track {
    grid-column: 1 / -1;
    height: 3px;
    border-radius: 2px;
    background: var(--app-surface-hover);
    overflow: hidden;
  }
  .ob-track b {
    display: block;
    height: 100%;
    background: var(--app-accent);
    transition: width 120ms linear;
  }
  .ob-track.stalled b {
    background: var(--chart-grey-4);
  }
  @media (prefers-reduced-motion: reduce) {
    .ob-track b {
      transition: none;
    }
  }
</style>
