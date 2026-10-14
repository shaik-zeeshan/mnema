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
      level meter and a working retry loop over the three
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
  import { invoke } from "@tauri-apps/api/core";
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
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconArrow from "~icons/lucide/arrow-right";
  import IconCheck from "~icons/lucide/check";
  import IconMic from "~icons/lucide/mic";

  /** Notice titles for the three rejections (the body is `rejectionMessage`). */
  const REJECTION_TITLE: Record<string, string> = {
    tooShort: "Too short to learn from",
    noSpeech: "Too quiet to hear",
    multipleSpeakers: "More than one voice",
  };

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

  // No playback confirmation here, deliberately: this screen submits the take
  // in the same breath as recording it, and `enroll_account_owner_voice`
  // destroys the clip when it judges it (stored, rejected, or failed alike). The
  // Settings enrollment door is the surface with a review stage, so it is the
  // one that can honestly offer a replay.

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

<div class="ob-main">
  <div class="ob-split">
    {#if canEnroll}
      <div class="ob-head">
        <span class="mx-kicker">Voice</span>
        <h1 class="mx-display">Fifteen seconds so Mnema can use your name.</h1>
        <p class="ob-fine">Otherwise your side is filed as “Speaker&nbsp;1”.</p>
      </div>

      <div class="mx-panel ob-instr">
        <span class="mx-label">Read this out loud</span>
        <p class="ob-quote">“{ENROLLMENT_SENTENCE}”</p>

        <div class="ob-take">
          <!-- Real polled levels; they stop moving the moment the take does. -->
          <div class="ob-meter" class:voice={takeNumber > 0 || busy} aria-hidden="true">
            {#each levels as level, index (index)}
              <i style="height:{4 + Math.round(level * 36)}px"></i>
            {/each}
          </div>
          <span class="num" role="status">{takeLine}</span>
        </div>

        {#if phase === "enrolled"}
          <div class="mx-notice" data-tone="ok">
            <span class="mx-notice__icon"><IconCheck width="15" height="15" /></span>
            <b class="mx-notice__title">Enrolled.</b>
            <p class="mx-notice__text">Mnema will pick your voice out from now on.</p>
          </div>
        {:else if failure}
          <div class="mx-notice" data-tone="danger">
            <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
            <b class="mx-notice__title">That didn’t record.</b>
            <p class="mx-notice__text">{failure}</p>
          </div>
        {:else if rejection && rejection.status !== "enrolled"}
          <!-- Words shared with Settings (`rejectionMessage`); only the title is local. -->
          <div class="mx-notice" data-tone="warn">
            <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
            <b class="mx-notice__title">{REJECTION_TITLE[rejection.status]}</b>
            <p class="mx-notice__text">{rejectionMessage(rejection)}</p>
          </div>
        {/if}

        <!-- The instrument's own button stays with the instrument; everything
             that LEAVES this screen lives in the footer. -->
        <div class="ob-acts">
          {#if phase === "enrolled"}
            <button class="mx-btn" onclick={recordTake}>Record again</button>
          {:else}
            <button class="mx-btn mx-btn--primary" onclick={recordTake} disabled={busy} aria-busy={busy || undefined}>
              <IconMic width="14" height="14" />
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
        </div>
      </div>
    {:else}
      <div class="ob-head">
        <h1 class="mx-display">Not yet — and that is fine.</h1>
        <p class="ob-fine">This step waits for you, never the other way round.</p>
      </div>
      <div class="ob-head">
        <p class="mx-prose">{notReadyReason}</p>
        {#if readiness === "downloading"}
          <div class="ob-track"><b style:width="{downloadPercent}%"></b></div>
          {#if downloadLine}<p class="ob-fine num">{downloadLine}</p>{/if}
          <p class="ob-fine">A card will be waiting on your dashboard the moment it is ready.</p>
        {/if}
      </div>
    {/if}
  </div>
</div>

<footer class="ob-foot">
  <p class="ob-foot__note">
    The voiceprint never leaves this device, and recognition labels many turns — not every turn.
  </p>
  <button class="mx-btn mx-btn--ghost" onclick={onBack}>Back</button>
  {#if canEnroll}
    {#if phase === "enrolled"}
      <button class="mx-btn mx-btn--primary" onclick={onContinue}>
        Continue<IconArrow class="ob-btn-arrow" width="14" height="14" />
      </button>
    {:else}
      <button class="mx-btn" onclick={onSkip}>Skip — you can do this later in Settings</button>
    {/if}
  {:else}
    <!-- Both land on the same step; the difference is what the user is told
         will happen next, which is why the mockup keeps both. -->
    <button class="mx-btn mx-btn--ghost" onclick={onSkip}>Skip voice entirely</button>
    <button class="mx-btn mx-btn--primary" onclick={onSkip}>
      Set this up later<IconArrow class="ob-btn-arrow" width="14" height="14" />
    </button>
  {/if}
</footer>

<style>
  .ob-split {
    display: grid;
    grid-template-columns: minmax(0, 300px) minmax(0, 1fr);
    gap: var(--s-7);
    align-items: center;
  }
  .ob-split .mx-display {
    font-size: 34px;
  }
  .ob-instr {
    display: grid;
    gap: var(--s-4);
    padding: var(--s-5);
  }
  .ob-quote {
    margin: 0;
    font: 400 17px / 1.55 var(--font-sans);
    color: var(--app-text-strong);
    max-width: 50ch;
    text-wrap: pretty;
  }
  .ob-take {
    display: flex;
    align-items: center;
    gap: var(--s-4);
  }
  .ob-take .num {
    font-size: var(--text-base);
    color: var(--app-text-muted);
  }
  .ob-meter {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 44px;
  }
  .ob-meter i {
    width: 3px;
    border-radius: 2px;
    background: var(--chart-grey-2);
    transition: height 120ms var(--ease-quart);
  }
  .ob-meter.voice i {
    background: var(--app-source-mic);
  }
  .ob-track {
    max-width: 420px;
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
  @media (prefers-reduced-motion: reduce) {
    .ob-meter i,
    .ob-track b {
      transition: none;
    }
  }
</style>
