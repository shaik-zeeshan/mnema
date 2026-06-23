<script lang="ts">
  // Phase 3 of the onboarding flow: the restored "finale" bookend. A full-height
  // crest that summarizes the armed config and starts (or skips) recording via
  // the controller. Presentational — all state/logic lives on the controller.
  // The decorative `.finale*` styles live in `onboarding-ui.css`, namespaced
  // under `.ob-screen` (this root is NOT inside `.onboarding-shell`). Finishing
  // is gated on `c.ctaDisabled` (model readiness + zero attention items).
  import type { OnboardingController } from "./onboarding.svelte";

  let { controller }: { controller: OnboardingController } = $props();
  // `controller` is a stable instance; alias via $derived so template reads of
  // `c.<reactive field>` stay reactive (mirrors AskAiBody's `$derived` pattern).
  const c = $derived(controller);

  // Compact segment-length label: "1m 30s" / "5m" for ≥60s, "45s" otherwise.
  function formatDuration(value: number): string {
    if (value >= 60) {
      const minutes = Math.floor(value / 60);
      const seconds = value % 60;
      return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`;
    }
    return `${value}s`;
  }

  const sourceSummary = $derived(
    c.selectedSourceCount > 0
      ? `${c.selectedSourceCount} source${c.selectedSourceCount === 1 ? "" : "s"} armed`
      : "No sources armed",
  );
</script>

<div class="ob-screen">
  <section class="finale" aria-labelledby="finale-title">
    <div class="finale__bg" aria-hidden="true">
      <div class="finale__rings"></div>
      <div class="finale__rings finale__rings--alt"></div>
    </div>
    <div class="finale__inner">
      <span class="finale__crest">
        <span class="finale__crest-dot"></span>
        All set
      </span>
      <h2 id="finale-title" class="finale__title">Press record.</h2>
      <p class="finale__tag">
        Setup is complete. {sourceSummary} · {c.draftFrameRate} fps · {formatDuration(c.draftSegmentDuration)}
        segments{c.draftPauseCaptureOnInactivity
          ? ` · idle pause @ ${formatDuration(c.draftIdleTimeoutSeconds)}`
          : ""}.
      </p>

      {#if c.errorMessage}
        <div class="finale__err" role="alert">{c.errorMessage}</div>
      {/if}

      <div class="finale__chips">
        <span class="chip" data-on={c.draftCaptureScreen}>Screen</span>
        <span class="chip" data-on={c.draftCaptureMicrophone}>Mic</span>
        <span class="chip" data-on={c.draftCaptureSystemAudio && c.draftCaptureScreen}>Sys audio</span>
        <span class="chip" data-on={c.draftOcrEnabled}>OCR</span>
        <span class="chip" data-on={c.draftTranscriptionEnabled}>Transcript</span>
      </div>

      <div class="finale__cta">
        <button
          type="button"
          class="cta finale__rec"
          onclick={() => c.finish(true)}
          disabled={c.ctaDisabled}
        >
          <span class="finale__rec-dot" aria-hidden="true"></span>
          {c.starting ? "Starting…" : "Start recording"}
        </button>
        <button
          type="button"
          class="finale__link"
          onclick={() => c.finish(false)}
          disabled={c.ctaDisabled}
        >
          {c.completing && !c.starting ? "Opening…" : "Just open the dashboard →"}
        </button>
      </div>

      <button type="button" class="finale__back" onclick={() => c.backToConfigure()}>
        ← Back to setup
      </button>

      <p class="finale__foot">You can change anything later in <em>Settings</em>.</p>
    </div>
  </section>
</div>

<style>
  /* Failure banner near the CTA — a failed save / start-recording / complete
     would otherwise leave the user pressing a button that silently does nothing.
     Terminal/green danger tokens, consistent with the rest of onboarding. */
  .finale__err {
    margin: 4px auto 0;
    max-width: 40ch;
    padding: 10px 14px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--app-danger);
    background: var(--app-danger-bg);
    border: 1px solid var(--app-danger-border);
    border-radius: 8px;
  }
</style>
