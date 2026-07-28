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
    return `Not enough room for the downloads. ${formatBytes(free)} free · ${totalLabel} needed.`;
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

<h1 class="ob-sr-only">Setup</h1>

<div class="split">
  <div class="col">
    <div class="pct" aria-hidden="true">{percent}%</div>

    <!-- Deliberately NOT a live region: the byte figure changes on every chunk
         event, so `role="status"` would announce it dozens of times a second. -->
    <p class="line">
      {#if workList.length === 0}
        Nothing to download — everything you need is already here.
      {:else if !mayStart}
        Downloads are held until there is room.
      {:else if downloadingNow}
        <span class="nw">
          Downloading <span class="ob-strong">{FEATURE_NAME[downloadingNow.feature]}</span>
          · <span class="ob-num">{formatBytes(activeReceived)} of {totalLabel}</span>
        </span>
      {:else if active.length === 0}
        Every download was cancelled.
      {:else if progress.done}
        All set · <span class="ob-num">{totalLabel}</span> downloaded.
      {:else if failedCount > 0}
        {failedCount === 1 ? "One download" : `${failedCount} downloads`} didn't finish.
      {:else}
        Starting <span class="ob-strong">{FEATURE_NAME[active[0].feature]}</span>
        · <span class="ob-num">{totalLabel}</span> to fetch.
      {/if}
    </p>

    {#if shortfall}
      <div class="preflight">
        <p class="ob-blocked">{shortfall}</p>
        {#if !diskOverride}
          <button class="ob-btn sm" onclick={() => (diskOverride = true)}>
            Download anyway
          </button>
        {/if}
      </div>
    {/if}

    <div class="foot">
      <hr class="ob-rule" />
      <div class="ob-acts">
        <button class="ob-btn ghost spacer" onclick={onBack}>← Back</button>
        <button class="ob-btn primary" onclick={onContinue}>Continue&nbsp; →</button>
      </div>
      <p class="ob-fine">
        Live since you arrived, and it never disables. Downloads carry on behind you.
      </p>
    </div>
  </div>

  <div class="col mid">
    <div class="work">
      {#each workList as item (item.id)}
        {@const off = !flow.features[item.feature]}
        {@const state = progress.states[item.id] ?? "missing"}
        <div class="item">
          <div class="head">
            <span class="t" class:dim={off && state !== "ready"}>{FEATURE_NAME[item.feature]}</span>
            {#if state === "ready"}
              <!-- Ready wins over a switched-off feature: the bytes ARE on disk,
                   and this screen reports downloads. Whether the feature is on
                   is *Your settings*' business. -->
              <span class="st ready">✓ Ready</span>
            {:else if off}
              <span class="st warn">
                Turned off
                <button class="ob-btn sm" onclick={() => restoreItem(item)}>
                  Turn it back on
                </button>
              </span>
            {:else if state === "downloading"}
              <span class="st busy">
                Downloading
                <button class="ob-btn sm" onclick={() => void cancelItem(item)}>Cancel</button>
              </span>
            {:else if state === "failed"}
              <span class="st fail">
                {progress.errors[item.id] ?? "Download failed."}
                <button class="ob-btn sm" onclick={() => retryItem(item)}>Retry</button>
              </span>
            {:else}
              <span class="st wait">{mayStart ? "Queued" : "Held"}</span>
            {/if}
          </div>
          {#if !off && (state === "downloading" || state === "failed")}
            <div class="track">
              <i
                class:sheen={state === "downloading"}
                style="width:{state === 'failed' ? 0 : itemPercent(item)}%"
              ></i>
            </div>
          {/if}
        </div>
      {:else}
        <p class="ob-fine">
          Every model your settings need is already on this machine.
        </p>
      {/each}
    </div>
  </div>
</div>

<style>
  /* Mockup frame 06 (`chosen-cinematic-rewind.html`): a large aggregate percent
     plus one current-item line on the left, per-item rows on the right. Every
     colour is an `--app-*` token — the mockup's hexes are dark-theme only. */
  .split {
    display: grid;
    grid-template-columns: 360px 1fr;
    gap: 44px;
    flex: 1;
    min-height: 0;
  }
  .col {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .col.mid {
    justify-content: center;
  }

  .pct {
    font-size: 72px;
    line-height: 1;
    letter-spacing: -0.045em;
    color: var(--app-text-strong);
    font-variant-numeric: tabular-nums;
  }
  .line {
    margin: 14px 0 0;
    font-size: var(--text-md);
    line-height: 1.7;
    color: var(--app-text);
    max-width: 40ch;
  }
  /* The current-item line is one line, like the mockup's: it overflows the
     360px column into the gutter rather than wrapping (density rule 1). */
  .line .nw {
    white-space: nowrap;
  }

  .preflight {
    margin-top: 18px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
  }

  .foot {
    margin-top: auto;
    padding-top: 24px;
  }
  .foot .ob-acts {
    margin-top: 22px;
  }
  .foot .ob-fine {
    margin-top: 16px;
  }

  /* ---- the work-list: name + state only. The bar carries the progress. ---- */
  .work {
    display: flex;
    flex-direction: column;
  }
  .item {
    padding: 20px 0;
    border-top: 1px solid var(--app-border);
  }
  .item:first-child {
    border-top: 0;
    padding-top: 2px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 16px;
  }
  .t {
    font-size: var(--text-md);
    color: var(--app-text-strong);
  }
  .t.dim {
    color: var(--app-text-muted);
  }
  .st {
    margin-left: auto;
    font-size: var(--text-sm);
    display: flex;
    align-items: baseline;
    gap: 12px;
    text-align: right;
  }
  .st.ready {
    color: var(--app-accent-strong);
  }
  .st.busy {
    color: var(--app-text);
  }
  .st.wait {
    color: var(--app-text-subtle);
  }
  .st.warn {
    color: var(--app-warn);
  }
  .st.fail {
    color: var(--app-danger);
  }

  .track {
    height: 3px;
    background: var(--app-border-strong);
    margin-top: 13px;
    position: relative;
    overflow: hidden;
    border-radius: 2px;
  }
  .track i {
    position: absolute;
    inset: 0 auto 0 0;
    background: var(--app-text-strong);
    display: block;
    border-radius: 2px;
    transition: width 240ms linear;
  }
  /* Functional feedback, not ambient motion: it exists only while bytes are
     actually moving, which is the one exception the density rule allows. */
  .track i.sheen::after {
    content: "";
    position: absolute;
    inset: 0;
    width: 60px;
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--app-text-strong) 60%, transparent),
      transparent
    );
    animation: sheen 3.4s ease-in-out infinite;
  }
  @keyframes sheen {
    0% {
      transform: translateX(-70px);
    }
    100% {
      transform: translateX(430px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .track i.sheen::after {
      animation: none;
    }
    .track i {
      transition: none;
    }
  }
</style>
