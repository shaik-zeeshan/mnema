<!--
  Screen 8 / 8 — Finale (issue #195, slice 10).

  Capture starts HERE, on arrival — after the Voice slot, so the bounded
  recorder had the microphone free. The screen then ends on EVIDENCE rather
  than a summary: the real first frame off the real pipeline, its real
  timestamp, and the first words OCR actually read off it.

  Four states, all real (mockup frames 1644-1792):
    · waiting  — capture started, no frame yet. Shows the CONFIGURED capture
                 interval, not a spinner; a spinner would not say how long.
    · success  — `mx-rec` running pill with its clock; first frame, then the
                 first OCR hit revealed letter by letter, once (reduced
                 motion shows it whole).
                 Audio-only when the screen is not live (Screen Recording
                 denied, mic or system audio still on): it says so and waits
                 for no frame.
    · idle     — nothing granted, so nothing is recording. No heartbeat: there
                 is nothing alive to beat, and that absence is the point.
    · failed   — capture could not start, named with the real backend reason.

  This screen NEVER saves settings, completes onboarding, or starts capture
  itself — `onFinish` is the single atomic commit path. It only decides whether
  to ask for capture (`startRecording`), then reads what happened.
-->
<script lang="ts">
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { fpsToIntervalS } from "$lib/components/capture-rate";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import type { FrameDto, FramePreviewDto, GetFramePreviewRequest } from "$lib/types";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconExternal from "~icons/lucide/external-link";
  import IconLock from "~icons/lucide/lock";

  let {
    flow,
    onBack,
    onFinish,
  }: {
    flow: OnboardingFlow;
    onBack: () => void;
    onFinish: (startRecording?: boolean) => Promise<void>;
  } = $props();

  const c = $derived(flow.controller);

  type Phase = "starting" | "running" | "idle" | "failed";
  let phase = $state<Phase>("starting");
  let failureReason = $state<string | null>(null);
  let frame = $state<FrameDto | null>(null);
  let previewUrl = $state<string | null>(null);
  let nowMs = $state(Date.now());
  let rechecking = $state(false);

  const intervalS = $derived(fpsToIntervalS(c.draftFrameRate));
  const screenGranted = $derived(c.permissions?.screen === "granted");

  // What will ACTUALLY record. Every capture source defaults ON in the resolved
  // settings — the PERMISSION is what makes each one real — so the live check
  // has to AND the two. System audio has no readable grant at all (ADR 0052),
  // so a raised prompt (or sound already arriving) is the strongest answer it
  // can ever give. Read straight off the controller, not off the resolver's
  // snapshot, so "Re-check" sees a grant made since the commit.
  const liveSources = $derived({
    screen: flow.features.screen && screenGranted,
    microphone: flow.features.microphone && c.permissions?.microphone === "granted",
    systemAudio:
      flow.features.systemAudio
      && (c.sysAudioPromptRaised || c.permissions?.systemAudio === "assumed_working"),
  });
  const willCapture = $derived(
    liveSources.screen || liveSources.microphone || liveSources.systemAudio,
  );
  // A screen denial gates nothing: with an audio source live the session runs
  // audio-only, and the evidence column must not promise a frame.
  const audioOnly = $derived(phase === "running" && !liveSources.screen);
  const audioName = $derived(
    liveSources.microphone && liveSources.systemAudio
      ? "Audio"
      : liveSources.microphone
        ? "Microphone"
        : "System audio",
  );
  const screenOff = $derived(screenGranted ? "Screen capture is turned off" : "Screen Recording is off");

  const ocrText = $derived(firstLine(frame?.ocrText ?? null));
  const secondsAgo = $derived(
    frame ? Math.max(0, Math.round((nowMs - Date.parse(frame.capturedAt)) / 1000)) : 0,
  );

  function firstLine(text: string | null): string | null {
    const line = (text ?? "")
      .split("\n")
      .map((l) => l.trim())
      .find((l) => l.length > 0);
    return line ? line.slice(0, 64) : null;
  }

  // ── The REC pill's clock: seconds since capture started on this screen. ──
  let recStartMs = $state<number | null>(null);
  $effect(() => {
    if (phase !== "running") return;
    recStartMs ??= Date.now();
    const timer = setInterval(() => (nowMs = Date.now()), 1000);
    return () => clearInterval(timer);
  });
  const two = (n: number) => String(n).padStart(2, "0");
  const elapsed = $derived.by(() => {
    const t = recStartMs === null ? 0 : Math.max(0, Math.floor((nowMs - recStartMs) / 1000));
    return [Math.floor(t / 3600), Math.floor(t / 60) % 60, t % 60].map(two).join(":");
  });

  async function newestFrameId(): Promise<number | null> {
    try {
      const page = await invoke<FrameDto[]>("list_frames", { request: { limit: 1 } });
      return page[0]?.id ?? null;
    } catch {
      return null;
    }
  }

  async function loadPreview(frameId: number): Promise<void> {
    try {
      const dto = await invoke<FramePreviewDto | null>("get_frame_preview", {
        request: { frameId } satisfies GetFramePreviewRequest,
      });
      if (dto) previewUrl = framePreviewAssetUrl(dto.filePath);
    } catch {
      // A frame can exist before its image is readable (the segment is still
      // in flight). The caption still proves the capture — no placeholder art.
    }
  }

  // Commit once on arrival. `onFinish` is the ONLY path that writes settings,
  // completes onboarding and starts capture; everything below just reads the
  // result. The early-return inside `finishOnboarding` is silent, so its one
  // precondition is checked here to turn a no-op into a named failure.
  async function commit(): Promise<void> {
    const start = willCapture;
    const frames = liveSources.screen;
    const baselineId = frames ? await newestFrameId() : null;
    if (c.settings === null || !c.canSkipToDashboard) {
      failureReason =
        "Your settings could not be saved: the custom resolution or bitrate is not a valid number.";
      phase = "failed";
      return;
    }
    await onFinish(start);
    if (flow.errorMessage) {
      failureReason = flow.errorMessage;
      phase = "failed";
      return;
    }
    if (!start) {
      phase = "idle";
      return;
    }
    phase = "running";
    if (frames) void poll(baselineId);
  }

  // Wait for the pipeline to produce evidence: first a frame row, then the OCR
  // text on it. Bounded — OCR that never lands leaves the frame on screen
  // rather than polling the database forever.
  async function poll(baselineId: number | null): Promise<void> {
    for (let tick = 0; tick < 90; tick += 1) {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      if (cancelled) return;
      nowMs = Date.now();
      try {
        if (!frame) {
          const page = await invoke<FrameDto[]>("list_frames", { request: { limit: 1 } });
          const latest = page[0];
          if (latest && latest.id !== baselineId) {
            frame = latest;
            void loadPreview(latest.id);
          }
        } else if (!frame.ocrText) {
          const fresh = await invoke<FrameDto | null>("get_frame", {
            request: { frameId: frame.id },
          });
          if (fresh) frame = fresh;
          if (!previewUrl) void loadPreview(frame.id);
        } else {
          return;
        }
      } catch {
        // Transient read failures are not worth a state change; the next tick
        // retries and the screen keeps saying exactly what it knows.
      }
    }
  }

  let cancelled = false;
  let committed = false;
  $effect(() => {
    if (committed) return;
    committed = true;
    void commit();
    return () => {
      cancelled = true;
    };
  });

  async function recheck(): Promise<void> {
    if (rechecking) return;
    rechecking = true;
    try {
      await c.refreshPermissions();
      // A grant since the commit means there IS something to record now, and
      // the commit path is idempotent — run it again rather than inventing a
      // second way to start capture.
      if (willCapture) {
        committed = false;
        phase = "starting";
        await commit();
      }
    } finally {
      rechecking = false;
    }
  }

  const openSettings = () => invoke("show_permission_helper", { kind: "screen" });
  const relaunch = () => invoke("request_app_relaunch");
  const openMnema = () => goto("/");
</script>

<div class="ob-main">
  <div class="ob-split">
    <div class="ob-head">
      {#if phase === "running"}
        <span class="mx-rec mx-rec--running"><span class="mx-rec__dot"></span>REC {elapsed}</span>
        <h1 class="ob-finale__title">It has<br />already<br />started.</h1>
        <p class="ob-fine">
          {audioOnly
            ? `${audioName} only — ${screenOff}, so there are no frames yet.`
            : "Capture began the moment you arrived here."}
        </p>
      {:else if phase === "idle"}
        <span class="mx-rec"><span class="mx-rec__dot"></span>NOT RECORDING</span>
        <h1 class="ob-finale__title">Nothing is<br />being<br />recorded.</h1>
        <p class="ob-fine">
          {#if !screenGranted}
            You declined Screen Recording, so Mnema is idle.
          {:else}
            Every capture source is turned off, so Mnema is idle.
          {/if}
        </p>
      {:else if phase === "failed"}
        <span class="mx-rec mx-rec--stopped"><span class="mx-rec__dot"></span>NOT RECORDING</span>
        <h1 class="ob-finale__title">Capture could<br />not start.</h1>
        <p class="ob-reason">{failureReason}</p>
        {#if screenGranted}
          <p class="ob-fine">
            Screen Recording is granted, so this is almost certainly the stale-stream case: macOS hands the
            capture stream out at launch, and Mnema has not been relaunched since the grant.
          </p>
        {/if}
      {:else}
        <span class="mx-rec mx-rec--starting"><span class="mx-spin mx-spin--sm"></span>STARTING…</span>
        <h1 class="ob-finale__title">Setting up.</h1>
        <p class="ob-fine">Saving your settings, then starting capture.</p>
      {/if}
    </div>

    <div class="ob-evidence">
      <div class="ob-head">
        <span class="mx-label">
          {audioOnly
            ? `Listening — ${audioName.toLowerCase()} only`
            : phase === "running"
              ? "Your first frame"
              : "What you would be seeing"}
        </span>
        {#if previewUrl}
          <div class="ob-shot"><img src={previewUrl} alt="The first frame Mnema captured" /></div>
        {:else}
          <div class="ob-shot empty">
            <span class="ob-fine">
              {#if audioOnly}
                No screen frames — {screenOff}.
              {:else if phase === "running" && frame}
                Frame captured — its image is still being written.
              {:else if phase === "running" || phase === "starting"}
                first frame due within {intervalS} {intervalS === 1 ? "second" : "seconds"}
              {:else}
                No frame has been captured.
              {/if}
            </span>
          </div>
        {/if}
        {#if frame}
          <p class="ob-fine num">
            Captured {new Date(frame.capturedAt).toLocaleTimeString("en-GB")} · {secondsAgo}
            {secondsAgo === 1 ? "second" : "seconds"} ago
          </p>
        {/if}
      </div>

      {#if ocrText}
        <div class="ob-head">
          <span class="mx-label">First words read off it · already searchable</span>
          <!-- One letter-by-letter reveal, once; reduced motion shows it whole. -->
          <!-- prettier-ignore -->
          <p class="ob-ocr"><mark>{#each [...ocrText] as ch, i (i)}<span style:--i={i}>{ch}</span>{/each}</mark></p>
        </div>
      {:else if phase === "idle"}
        <p class="ob-fine">Privacy &amp; Security → Screen Recording → Mnema, then relaunch once.</p>
      {:else if audioOnly && !screenGranted}
        <div class="mx-notice" data-tone="warn">
          <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
          <b class="mx-notice__title">Screen Recording is off for Mnema</b>
          <p class="mx-notice__text">
            Turn Screen Recording on in System Settings, then relaunch once — frames start on their own.
          </p>
          <div class="mx-notice__acts">
            <button class="mx-btn mx-btn--sm" onclick={openSettings}>
              Open System Settings<IconExternal width="12" height="12" />
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
</div>

<!-- The same footer every other screen has: state on the left, actions on the right. -->
<footer class="ob-foot">
  <p class="ob-foot__note">
    {#if phase === "idle"}
      Grant it later and capture starts on its own.
    {:else if c.selectedSemanticSearchDownloadRunning && c.selectedSemanticSearchDownloadPercent !== null}
      Preparing · Semantic Search <span class="ob-num">{c.selectedSemanticSearchDownloadPercent}%</span> — turns
      itself on.
    {:else if phase === "running"}
      Everything else runs in the background from here.
    {/if}
  </p>
  {#if phase === "idle"}
    <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
    <button class="mx-btn" onclick={openMnema}>Open Mnema anyway</button>
    <button class="mx-btn" onclick={recheck} disabled={rechecking}>
      {rechecking ? "Re-checking…" : "Re-check"}
    </button>
    <button class="mx-btn mx-btn--primary" onclick={openSettings}>
      Open System Settings<IconExternal width="13" height="13" />
    </button>
  {:else if phase === "failed"}
    <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
    <button class="mx-btn" onclick={openMnema}>Open Mnema anyway</button>
    <button class="mx-btn mx-btn--primary" onclick={relaunch}>Relaunch Mnema</button>
  {:else}
    <button class="mx-btn mx-btn--primary" onclick={openMnema} disabled={phase === "starting"}>
      Open Mnema<IconArrow class="ob-btn-arrow" width="14" height="14" />
    </button>
  {/if}
</footer>

<style>
  .ob-split {
    display: grid;
    grid-template-columns: minmax(0, 360px) minmax(0, 1fr);
    gap: var(--s-7);
    align-items: center;
  }
  .ob-finale__title {
    margin: var(--s-4) 0 0;
    font: 600 60px / 0.98 var(--font-sans);
    letter-spacing: -0.042em;
    color: var(--app-text-strong);
  }
  .mx-rec {
    font: 500 var(--text-sm) / 1 var(--font-mono);
    letter-spacing: 0.04em;
  }
  .ob-reason {
    margin: 0;
    font: 400 var(--text-md) / 1.45 var(--font-mono);
    color: var(--app-danger-text);
  }
  .ob-evidence {
    display: grid;
    gap: var(--s-5);
  }
  .ob-evidence > .ob-head {
    justify-items: stretch;
  }
  .ob-shot {
    position: relative;
    aspect-ratio: 16 / 10;
    overflow: hidden;
    border-radius: var(--r-lg);
    border: 1px solid var(--app-border);
    background: var(--app-surface);
  }
  .ob-shot img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .ob-shot.empty {
    display: grid;
    place-items: center;
    padding: 0 24px;
    text-align: center;
    border-style: dashed;
    background: transparent;
  }
  .ob-ocr {
    margin: 0;
    padding: 12px var(--s-3);
    border-radius: var(--r-md);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    font: 400 var(--text-md) / 1.5 var(--font-mono);
    color: var(--app-text);
    overflow-x: auto;
    white-space: nowrap;
  }
  .ob-ocr mark {
    background: none;
    color: var(--app-accent);
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
    text-underline-offset: 4px;
  }
  .ob-ocr span {
    white-space: pre;
    opacity: 0;
    animation: ob-glyph 360ms var(--ease-quart) forwards;
    animation-delay: calc(var(--i) * 22ms + 300ms);
  }
  @keyframes ob-glyph {
    from {
      opacity: 0;
      filter: blur(3px);
    }
    to {
      opacity: 1;
      filter: none;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .ob-ocr span {
      animation: none;
      opacity: 1;
    }
  }
</style>
