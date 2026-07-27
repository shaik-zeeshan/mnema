<!--
  Screen 7 / 8 — Voice (issue #195, slice 15). Mockup frames 07a + 07b.

  CONTRACT
    props
      flow        OnboardingFlow. Reads:
                    flow.features.speakerSeparation      is diarization even on
                    flow.controller.selectedSpeakerModel .available / download progress —
                                                         speakrs is work-list item #1
                                                         precisely so this screen can run
      onContinue  () => void — advance to the Finale after enrolling.
      onBack      () => void — return to Setup.
      onSkip      () => void — skip enrollment. Same destination as onContinue, but
                  a SEPARATE prop because skipping is a first-class outcome and
                  must read as one, not as a de-emphasised Continue.
    emits
      Those three. The bounded recorder and the enrollment embedder are Tauri
      commands this screen calls directly (slices 13 + 14).
    owns
      Two states. Speakrs ready → inline enrollment with a supplied sentence, a
      level meter, playback confirmation and a working retry loop over the three
      typed rejections (MultipleSpeakers / TooShort / NoSpeech). Speakrs still
      downloading → say so, with "Set this up later" as the PRIMARY action.
      Never a spinner, never a wall. Two honesty lines: the voiceprint never
      leaves this device, and recognition is imperfect and will not label every
      turn.
    must not
      Gate finishing. Enroll anyone other than the account owner. Infer identity
      from capture family — a microphone records whoever is in the room.
    gates
      None. Skip is always available.

  WHY THE PLUMBING LOOKS LIKE THIS
    · ALL enrollment judgment is the backend embedder's. This screen never
      inspects a clip; it renders the tagged `status` it is handed back. There is
      no client-side "too quiet" heuristic anywhere below, deliberately.
    · Recognition is switched on by `enroll_account_owner_voice` itself, so the
      screen does not touch settings.
    · The live-session guard (pause mic → record → resume) lives in
      `native_capture/lifecycle.rs`, so no capture state is managed here either.
    · Readiness comes from `classifyReadiness` — the same classifier the Setup
      screen uses. A second notion of "ready" is exactly the bug this issue is
      about.
-->
<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { classifyReadiness } from "$lib/onboarding/model-readiness";
  import { describeError, formatBytes } from "$lib/settings/state/format";
  // Wire shape, clip length, the read-aloud sentence AND the three rejection
  // messages are shared with the Settings enrollment door (slice 16): two doors
  // onto one feature must not hand the user different words to read or
  // different explanations of the same verdict. Only the typography below is
  // this screen's own.
  import {
    ENROLLMENT_CLIP_MS,
    ENROLLMENT_SENTENCE,
    rejectionMessage,
    type VoiceEnrollmentOutcome,
  } from "$lib/voice-enrollment";
  import type { OnboardingFlow } from "../onboarding-flow.svelte";

  let {
    flow,
    onContinue,
    onBack,
    onSkip,
  }: {
    flow: OnboardingFlow;
    onContinue: () => void;
    onBack: () => void;
    onSkip: () => void;
  } = $props();

  /** Bars in the level meter (mockup 07a draws eighteen). */
  const BARS = 18;

  type Phase = "idle" | "recording" | "judging" | "rejected" | "enrolled" | "error";

  // ── State ────────────────────────────────────────────────────────────────
  let phase = $state<Phase>("idle");
  let takeNumber = $state(0);
  let clipPath = $state<string | null>(null);
  /** Measured wall-clock length of the last completed take. */
  let takeMs = $state(0);
  /** Ring of the last `BARS` polled microphone levels, 0-1. */
  let levels = $state<number[]>(new Array(BARS).fill(0));
  let peak = $state(0);
  let elapsedMs = $state(0);
  let rejection = $state<VoiceEnrollmentOutcome | null>(null);
  let failure = $state<string | null>(null);

  // ── Which of the two screens is this? ────────────────────────────────────
  // One notion of ready, shared with Setup. `speakerSeparation` off is its own
  // reason: a voiceprint with nothing diarized to match it against is a lie.
  const readiness = $derived(
    classifyReadiness(
      flow.controller.selectedSpeakerModel?.available ?? false,
      flow.controller.selectedSpeakerDownloadProgress,
    ),
  );
  const canEnroll = $derived(readiness === "ready" && flow.features.speakerSeparation);
  const download = $derived(flow.controller.selectedSpeakerDownloadProgress);
  const notReadyReason = $derived(
    !flow.features.speakerSeparation
      ? "Who’s speaking is switched off, so there is nothing to match a voice against."
      : readiness === "downloading"
        ? "The model that tells voices apart is still arriving."
        : readiness === "failed"
          ? "The model that tells voices apart didn’t finish downloading."
          : "The model that tells voices apart isn’t here yet.",
  );

  // ── The take ─────────────────────────────────────────────────────────────
  let ticker: ReturnType<typeof setInterval> | null = null;

  function stopTicker(): void {
    if (ticker !== null) clearInterval(ticker);
    ticker = null;
  }
  $effect(() => stopTicker);

  async function pollLevel(startedAt: number): Promise<void> {
    elapsedMs = Date.now() - startedAt;
    // `null` simply means no sample yet — the meter stays at its floor.
    const level = await invoke<number | null>("get_microphone_activity_level").catch(() => null);
    const value = Math.min(1, Math.max(0, level ?? 0));
    levels = [...levels.slice(1), value];
    peak = Math.max(peak, value);
  }

  async function recordTake(): Promise<void> {
    if (phase === "recording" || phase === "judging") return;
    rejection = null;
    failure = null;
    levels = new Array(BARS).fill(0);
    peak = 0;
    elapsedMs = 0;
    phase = "recording";

    const startedAt = Date.now();
    stopTicker();
    ticker = setInterval(() => void pollLevel(startedAt), 150);
    try {
      const path = await invoke<string>("record_bounded_microphone_clip", {
        durationMs: ENROLLMENT_CLIP_MS,
      });
      stopTicker();
      takeMs = Date.now() - startedAt;
      takeNumber += 1;
      clipPath = path;
      phase = "judging";
      // Judgment is entirely the embedder's; we render whatever it returns.
      const outcome = await invoke<VoiceEnrollmentOutcome>("enroll_account_owner_voice", {
        request: { clipPath: path, displayName: null },
      });
      if (outcome.status === "enrolled") {
        phase = "enrolled";
      } else {
        rejection = outcome;
        phase = "rejected";
      }
    } catch (err) {
      failure = describeError(err);
      phase = "error";
    } finally {
      stopTicker();
    }
  }

  // ── Playback confirmation ────────────────────────────────────────────────
  let player: HTMLAudioElement | null = null;

  function playTake(): void {
    if (!clipPath) return;
    player?.pause();
    player = new Audio(convertFileSrc(clipPath));
    player.play().catch((err: unknown) => {
      failure = describeError(err);
    });
  }

  // ── Formatting ───────────────────────────────────────────────────────────
  function clock(ms: number): string {
    const total = Math.max(0, Math.round(ms / 1000));
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  }

  const takeLine = $derived(
    phase === "recording"
      ? `${clock(elapsedMs)} of about ${clock(ENROLLMENT_CLIP_MS)}`
      : phase === "judging"
        ? "Checking the take…"
        : takeNumber === 0
          ? `Ready when you are · about ${clock(ENROLLMENT_CLIP_MS)}`
          : `Take ${takeNumber} · ${clock(takeMs)} · peaked at ${Math.round(peak * 100)}%`,
  );

  // Freeze the meter once the take is over: the bars then describe the take
  // that was judged, which is what the reading below them refers to.
  const busy = $derived(phase === "recording" || phase === "judging");

  const downloadLine = $derived(
    download && download.totalBytes
      ? `${formatBytes(download.downloadedBytes)} of ${formatBytes(download.totalBytes)}`
      : download
        ? formatBytes(download.downloadedBytes)
        : null,
  );
  const downloadPercent = $derived(
    download?.totalBytes
      ? Math.min(100, Math.round((download.downloadedBytes / download.totalBytes) * 100))
      : 0,
  );

  // Re-entry: an owner voiceprint may already exist from a previous run.
  $effect(() => {
    void invoke<number | null>("get_account_owner_person_id")
      .then((id) => {
        if (id !== null && phase === "idle") phase = "enrolled";
      })
      .catch(() => {
        // A read failure is not a reason to hide the enrollment surface.
      });
  });
</script>

<h1 class="ob-sr-only">Voice</h1>

<div class="split">
  <div class="col">
    {#if canEnroll}
      <p class="ob-disp sm">Fifteen seconds so Mnema can use your name.</p>
      <p class="ob-fine why">Otherwise your side is filed as “Speaker&nbsp;1”.</p>
    {:else}
      <p class="ob-disp sm">Not yet — and that is fine.</p>
      <p class="ob-fine why">This step waits for you, never the other way round.</p>
    {/if}
  </div>

  <div class="col mid">
    {#if canEnroll}
      <span class="ob-m">Read this out loud</span>
      <p class="quote">“{ENROLLMENT_SENTENCE}”</p>

      <div class="reading">
        <div class="meter" class:live={busy} aria-hidden="true">
          {#each levels as level, index (index)}
            <i class:lit={level > 0.04} style="height:{4 + Math.round(level * 34)}px"></i>
          {/each}
        </div>
        <span class="ob-fine ob-num" role="status">{takeLine}</span>
      </div>

      {#if phase === "enrolled"}
        <p class="callout ok"><b>Enrolled.</b> Mnema will pick your voice out from now on.</p>
      {:else if failure}
        <p class="callout bad"><b>That didn’t record.</b> {failure}</p>
      {:else if rejection && rejection.status !== "enrolled"}
        <!-- Words shared with Settings; only the callout treatment is local. -->
        <p class="callout">{rejectionMessage(rejection)}</p>
      {/if}

      <div class="ob-acts acts">
        {#if phase === "enrolled"}
          <button class="ob-btn primary" onclick={onContinue}>Continue&nbsp; →</button>
          <button class="ob-btn" onclick={recordTake}>Record again</button>
        {:else}
          <button class="ob-btn primary" onclick={recordTake} disabled={busy}>
            {#if phase === "recording"}
              Recording…
            {:else if phase === "judging"}
              Checking…
            {:else if takeNumber === 0}
              Record fifteen seconds
            {:else}
              Record again
            {/if}
          </button>
        {/if}
        {#if clipPath}
          <button class="ob-btn" onclick={playTake} disabled={busy}>
            ▸ Play take {takeNumber}
          </button>
        {/if}
        <button class="ob-btn ghost wrap" onclick={onSkip}>
          Skip — you can do this later in Settings
        </button>
      </div>
    {:else}
      <p class="ob-lead">{notReadyReason}</p>

      {#if readiness === "downloading"}
        <div class="track"><i class="sheen" style="width:{downloadPercent}%"></i></div>
        {#if downloadLine}
          <p class="ob-fine ob-num bytes">{downloadLine}</p>
        {/if}
      {/if}

      <div class="ob-acts acts">
        <!-- Both land on the same step; the difference is what the user is told
             will happen next, which is why the mockup keeps both. -->
        <button class="ob-btn primary" onclick={onSkip}>Set this up later&nbsp; →</button>
        <button class="ob-btn ghost" onclick={onSkip}>Skip voice entirely</button>
      </div>
      {#if readiness === "downloading"}
        <p class="ob-fine after">A card will be waiting on your dashboard the moment it is ready.</p>
      {/if}
    {/if}
  </div>
</div>

<div class="foot">
  <hr class="ob-rule" />
  <div class="honesty">
    <button class="ob-btn ghost" onclick={onBack}>← Back</button>
    <p class="ob-fine">
      The voiceprint never leaves this device, and recognition labels many turns — not every turn.
    </p>
  </div>
</div>

<style>
  /* Mockup frames 07a/07b (`chosen-cinematic-rewind.html` 1510-1643, scoped
     style 1442-1507). Colours are `--app-*` tokens; the mockup's hexes are
     dark-theme only. */
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
  .why {
    margin-top: 16px;
  }

  /* The mockup's quote was a 21px one-liner. The real sentence is the shared
     three-sentence read (`ENROLLMENT_SENTENCE`) — the recorder captures a fixed
     15 s, so a shorter read would leave most of every clip as silence. At the
     mockup's size it wrapped to six lines and pushed the rejection callout past
     the 1040x680 frame (`.split` clips, it does not scroll). 17px over a wider
     measure lands it at five, which fits with any callout showing. */
  .quote {
    font-size: 17px;
    line-height: 1.55;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
    max-width: 60ch;
    margin: 12px 0 0;
    text-wrap: pretty;
  }

  .reading {
    display: flex;
    align-items: flex-end;
    gap: 22px;
    margin-top: 30px;
  }

  /* Functional feedback, not ambient motion: the bars are real polled levels
     and they stop moving the moment the take does. */
  .meter {
    display: flex;
    align-items: flex-end;
    gap: 3px;
    height: 44px;
    flex: none;
  }
  .meter i {
    width: 3px;
    background: var(--app-border-hover);
    display: block;
    border-radius: 1px;
    height: 4px;
    transition: height 140ms linear;
  }
  .meter i.lit {
    background: var(--app-text-subtle);
  }
  .meter.live i.lit {
    background: var(--app-accent);
  }

  .callout {
    border-left: 2px solid var(--app-warn);
    background: var(--app-warn-bg);
    padding: 12px 16px;
    font-size: var(--text-base);
    color: var(--app-text);
    line-height: 1.65;
    margin: 26px 0 0;
    max-width: 62ch;
  }
  .callout b {
    font-weight: 400;
    color: var(--app-warn);
  }
  .callout.ok {
    border-left-color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .callout.ok b {
    color: var(--app-accent-strong);
  }
  .callout.bad {
    border-left-color: var(--app-danger);
    background: var(--app-danger-bg);
  }
  .callout.bad b {
    color: var(--app-danger);
  }

  .acts {
    margin-top: 26px;
  }
  /* The skip is long by design — let it take its own line rather than shrink. */
  .acts .wrap {
    flex-basis: 100%;
    text-align: left;
  }

  .track {
    height: 3px;
    background: var(--app-border-strong);
    margin-top: 26px;
    max-width: 420px;
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
  .bytes {
    margin-top: 12px;
  }
  .after {
    margin-top: 16px;
  }

  .foot {
    margin-top: auto;
    padding-top: 24px;
    flex: none;
  }
  .honesty {
    display: flex;
    align-items: baseline;
    gap: 18px;
    margin-top: 14px;
  }
  .honesty .ob-fine {
    max-width: none;
  }

  @media (prefers-reduced-motion: reduce) {
    .meter i,
    .track i {
      transition: none;
    }
    .track i.sheen::after {
      animation: none;
    }
  }
</style>
