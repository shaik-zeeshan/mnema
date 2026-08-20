<!--
  Screen 8 / 8 — Finale (issue #195, slice 10).

  Capture starts HERE, on arrival — after the Voice slot, so the bounded
  recorder had the microphone free. The screen then ends on EVIDENCE rather
  than a summary: the real first frame off the real pipeline, its real
  timestamp, and the first words OCR actually read off it.

  Four states, all real (mockup frames 1644-1792):
    · waiting  — capture started, no frame yet. Shows the CONFIGURED capture
                 interval, not a spinner; a spinner would not say how long.
    · success  — first frame, then the first OCR hit developing out of noise.
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
  import { detectKeyboardPlatform, formatShortcut } from "$lib/keyboard";
  import { parseShortcutBinding } from "$lib/keyboard-binding-utils";
  import { DEFAULT_KEYBOARD_BINDINGS } from "$lib/keyboard-bindings.svelte";

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

  // ── The develop motion: garbled glyphs settling into legible characters,
  //    left to right. A picture of what OCR did to that frame, used once.
  const NOISE = "▞▛▒&@≡±▜∷▓░▙▗▖#%";
  const glyphs = $derived(
    (ocrText ?? "").split("").map((ch, i) => ({
      ch,
      noise: ch.trim() === "" ? null : NOISE[i % NOISE.length],
      delay: i * 42,
    })),
  );

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
    const baselineId = start ? await newestFrameId() : null;
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
    void poll(baselineId);
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

  const openSettings = () => invoke("open_capture_privacy_settings", { kind: "screen" });
  const relaunch = () => invoke("request_app_relaunch");
  const openMnema = () => goto("/");

  // Exactly two shortcuts, named once, on the only screen every user reaches.
  // 18 · Keyboard's finding: everything else in the shortcut design (tooltips,
  // the palette, the map) only reaches someone who already hovers or already
  // expects palettes. This is the one lever that reaches a user who never
  // presses a modifier — and a list of twelve here is a list of zero, so it is
  // two and stays two.
  const platform = detectKeyboardPlatform();
  const firstChords = [
    {
      label: "Start / stop recording",
      binding: DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.bindings.toggleRecording,
    },
    { label: "Quick Recall", binding: DEFAULT_KEYBOARD_BINDINGS.globalShortcuts.bindings.quickRecall },
  ].map(({ label, binding }) => {
    const parsed = parseShortcutBinding(binding);
    return { label, keys: parsed ? formatShortcut(parsed, platform) : [] };
  });
</script>

<div class="split">
  <div class="col">
    {#if phase === "running"}
      <span class="rec"><span class="dot"></span><span class="ob-m tag">Recording</span></span>
      <h1 class="ob-disp big">It has<br />already<br />started.</h1>
      <p class="ob-fine" style="margin-top:18px">Capture began the moment you arrived here.</p>
    {:else if phase === "idle"}
      <span class="rec"><span class="ring-glyph"></span><span class="ob-m tag">Not recording</span></span>
      <h1 class="ob-disp big">Nothing is<br />being<br />recorded.</h1>
      <p class="ob-fine" style="margin-top:18px">
        {#if !screenGranted}
          You declined Screen Recording, so Mnema is idle.
        {:else}
          Every capture source is turned off, so Mnema is idle.
        {/if}
      </p>
    {:else if phase === "failed"}
      <span class="rec"><span class="ring-glyph bad"></span><span class="ob-m tag">Not recording</span></span>
      <h1 class="ob-disp big">Capture could<br />not start.</h1>
      <p class="reason" style="margin-top:18px">{failureReason}</p>
      {#if screenGranted}
        <p class="ob-fine" style="margin-top:12px">
          Screen Recording is granted, so this is almost certainly the stale-stream case: macOS
          hands the capture stream out at launch, and Mnema has not been relaunched since the
          grant.
        </p>
      {/if}
    {:else}
      <span class="rec"><span class="ring-glyph"></span><span class="ob-m tag">Starting</span></span>
      <h1 class="ob-disp big">Setting up.</h1>
      <p class="ob-fine" style="margin-top:18px">
        Saving your settings, then starting capture.
      </p>
    {/if}

    <div class="chords">
      {#each firstChords as chord (chord.label)}
        {#if chord.keys.length}
          <div class="chord">
            <span class="caps">
              {#each chord.keys as key (key)}<kbd>{key}</kbd>{/each}
            </span>
            <span class="ob-fine">{chord.label}</span>
          </div>
        {/if}
      {/each}
    </div>
  </div>

  <div class="col evidence">
    <div>
      <span class="ob-m">
        {phase === "running" ? "Your first frame" : "What you would be seeing"}
      </span>
      {#if previewUrl}
        <div class="shot" style="margin-top:10px">
          <img src={previewUrl} alt="The first frame Mnema captured" />
        </div>
      {:else}
        <div class="shot empty" style="margin-top:10px">
          <span class="ob-fine">
            {#if phase === "running" && frame}
              Frame captured — its image is still being written.
            {:else if phase === "running"}
              first frame due within {intervalS} {intervalS === 1 ? "second" : "seconds"}
            {:else if phase === "starting"}
              first frame due within {intervalS} {intervalS === 1 ? "second" : "seconds"}
            {:else}
              No frame has been captured.
            {/if}
          </span>
        </div>
      {/if}
      {#if frame}
        <p class="ob-fine ob-num" style="margin-top:9px">
          Captured {new Date(frame.capturedAt).toLocaleTimeString("en-GB")} · {secondsAgo}
          {secondsAgo === 1 ? "second" : "seconds"} ago
        </p>
      {/if}
    </div>

    {#if ocrText}
      <div>
        <span class="ob-m">First words read off it · already searchable</span>
        <!-- prettier-ignore -->
        <div class="ocr" style="margin-top:10px"><span class="hit dev">{#each glyphs as g, i (i)}<span class="gl">{#if g.noise}<i style="animation-delay:{g.delay}ms">{g.noise}</i>{/if}<b style="animation-delay:{g.delay}ms">{g.ch}</b></span>{/each}</span></div>
      </div>
    {:else if phase === "idle"}
      <p class="ob-fine">Privacy &amp; Security → Screen Recording → Mnema, then relaunch once.</p>
    {/if}
  </div>
</div>

<!-- The same footer every other screen has: state on the left, actions on the
     right. It used to be pinned to the floor of the 360px column. -->
<div class="ob-foot">
  <hr class="ob-rule" />
  <div class="ob-acts">
    {#if phase === "idle"}
      <span class="ob-fine spacer">Grant it later and capture starts on its own.</span>
    {:else if c.selectedSemanticSearchDownloadRunning && c.selectedSemanticSearchDownloadPercent !== null}
      <span class="ob-fine spacer">
        Preparing · Semantic Search
        <span class="ob-num">{c.selectedSemanticSearchDownloadPercent}%</span> — turns itself on.
      </span>
    {:else if phase === "running"}
      <span class="ob-fine spacer">Everything else runs in the background from here.</span>
    {/if}
    {#if phase === "idle"}
      <button class="ob-btn ghost" onclick={onBack}>← Back</button>
      <button class="ob-btn" onclick={openMnema}>Open Mnema anyway</button>
      <button class="ob-btn" onclick={recheck} disabled={rechecking}>
        {rechecking ? "Re-checking…" : "Re-check"}
      </button>
      <button class="ob-btn primary" onclick={openSettings}>
        Open System&nbsp;Settings&nbsp; ›
      </button>
    {:else if phase === "failed"}
      <button class="ob-btn ghost" onclick={onBack}>← Back</button>
      <button class="ob-btn" onclick={openMnema}>Open Mnema anyway</button>
      <button class="ob-btn primary" onclick={relaunch}>Relaunch Mnema</button>
    {:else}
      <button class="ob-btn primary" onclick={openMnema} disabled={phase === "starting"}>
        Open Mnema&nbsp; →
      </button>
    {/if}
  </div>
</div>

<style>
  .split {
    display: grid;
    grid-template-columns: 360px 1fr;
    gap: 44px;
    /* The two columns centre against each other; nothing is pinned to a
       column's floor any more (the actions moved to `.ob-foot`). */
    align-items: center;
    flex: 1;
    min-height: 0;
  }
  .col {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .evidence {
    gap: 18px;
  }
  .ob-disp.big {
    font-size: 42px;
    margin-top: 16px;
  }
  .reason {
    font-size: var(--text-sm);
    line-height: 1.7;
    color: var(--app-danger);
    margin: 0;
    max-width: 64ch;
  }

  /* ---- one heartbeat: the app coming alive. Running state ONLY. ---- */
  .rec {
    display: inline-flex;
    align-items: center;
    gap: 11px;
  }
  .rec .tag {
    display: inline;
  }
  .rec .dot {
    position: relative;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    flex: none;
    background: var(--app-accent);
    animation: hb 2.8s ease-in-out infinite;
  }
  .rec .dot::after {
    content: "";
    position: absolute;
    inset: -5px;
    border-radius: 50%;
    border: 1px solid var(--app-accent);
    animation: hb-ring 2.8s ease-out infinite;
  }
  @keyframes hb {
    0%,
    100% {
      opacity: 0.42;
      transform: scale(0.78);
    }
    12% {
      opacity: 1;
      transform: scale(1);
    }
    46% {
      opacity: 0.6;
      transform: scale(0.88);
    }
  }
  @keyframes hb-ring {
    0% {
      opacity: 0.55;
      transform: scale(0.5);
    }
    55%,
    100% {
      opacity: 0;
      transform: scale(1.7);
    }
  }
  /* No heartbeat when nothing is recording — the absence is the point. */
  .ring-glyph {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1px solid var(--app-text-subtle);
    display: inline-block;
    flex: none;
  }
  .ring-glyph.bad {
    border-color: var(--app-danger);
  }

  /* ---- the evidence ---- */
  .shot {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    overflow: hidden;
    position: relative;
    aspect-ratio: 16 / 10;
  }
  .shot img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .shot.empty {
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 0 24px;
  }
  .ocr {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    padding: 14px 16px;
    background: var(--app-surface-subtle);
    font-size: var(--text-md);
    color: var(--app-text);
    overflow-x: auto;
  }
  .ocr .hit {
    color: var(--app-accent);
    white-space: nowrap;
  }

  /* ---- text that develops: garbled glyphs settle into characters, once ---- */
  .gl {
    position: relative;
    display: inline-block;
    white-space: pre;
  }
  .gl i {
    position: absolute;
    inset: 0;
    font-style: normal;
    color: var(--app-text-subtle);
  }
  .gl b {
    font-weight: 400;
    display: inline-block;
  }
  .dev .gl i {
    animation: gl-out 2.6s steps(1, end) both;
  }
  .dev .gl b {
    animation: gl-in 2.6s steps(1, end) both;
  }
  @keyframes gl-out {
    0% {
      opacity: 1;
    }
    4% {
      opacity: 0.35;
    }
    8% {
      opacity: 1;
    }
    12% {
      opacity: 0.5;
    }
    14% {
      opacity: 1;
    }
    15%,
    100% {
      opacity: 0;
    }
  }
  @keyframes gl-in {
    0%,
    14% {
      opacity: 0;
    }
    15%,
    100% {
      opacity: 1;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .dev .gl i {
      display: none;
    }
    .dev .gl b {
      animation: none;
      opacity: 1;
    }
    .rec .dot {
      animation: none;
      opacity: 1;
      transform: none;
    }
    .rec .dot::after {
      animation: none;
      opacity: 0.25;
      transform: none;
    }
  }

  /* The two first-run shortcuts. Keycaps sit in a FIXED-width column so every
     label starts at the same x — 18 · Keyboard's one correction to Screen
     Studio's shortcuts panel, which auto-widths its caps and staggers every
     description as a result. */
  .chords {
    margin-top: 30px;
    display: grid;
    gap: 8px;
  }
  .chord {
    display: grid;
    grid-template-columns: 116px minmax(0, 1fr);
    align-items: center;
    gap: 12px;
  }
  .caps {
    display: flex;
    gap: 4px;
  }
  .caps kbd {
    min-width: 22px;
    height: 22px;
    padding: 0 6px;
    display: inline-grid;
    place-items: center;
    border: 1px solid var(--app-border);
    border-radius: 5px;
    background: var(--app-surface-subtle);
    font-family: inherit;
    font-size: 11px;
    line-height: 1;
    color: var(--app-text);
  }
</style>
