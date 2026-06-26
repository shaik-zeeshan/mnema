<script lang="ts">
  // Phase 1 of the onboarding flow: the restored "welcome" bookend. A full-height
  // hero that fills `.onboarding-root` and hands off to the accordion via the
  // controller's phase machine. Presentational — all state/logic lives on the
  // controller. The decorative `.welcome*` styles live in `onboarding-ui.css`,
  // namespaced under `.ob-screen` (this root is NOT inside `.onboarding-shell`).
  import type { OnboardingController } from "./onboarding.svelte";

  let { controller }: { controller: OnboardingController } = $props();
  // `controller` is a stable instance; alias via $derived so template reads of
  // `c.<reactive field>` stay reactive (mirrors AskAiBody's `$derived` pattern).
  const c = $derived(controller);
</script>

<div class="ob-screen">
  <section class="welcome" aria-labelledby="welcome-title">
    <div class="welcome__bg" aria-hidden="true">
      <div class="welcome__grid"></div>
      <div class="welcome__halo"></div>
    </div>
    <div class="welcome__inner">
      <span class="welcome__eyebrow">
        <span class="welcome__pulse"></span>
        Welcome
      </span>
      <h1 id="welcome-title" class="welcome__title" tabindex="-1" data-ob-phase-heading>
        Your <em>memory</em>,
        <br />on rewind.
      </h1>
      <p class="welcome__tag">
        mnema quietly records your screen so you can scrub back to anything you've
        seen — searchable, local, and yours.
      </p>
      <ul class="welcome__loop" aria-hidden="true">
        <li><span></span>capture</li>
        <li><span></span>index</li>
        <li><span></span>recall</li>
      </ul>
      <div class="welcome__cta">
        <button
          type="button"
          class="cta"
          onclick={() => c.beginSetup()}
          disabled={c.loading}
        >
          {#if c.loading}
            <span class="cta__spin" aria-hidden="true"></span>
            Setting up…
          {:else}
            Begin setup
            <span class="btn__arrow" aria-hidden="true">→</span>
          {/if}
        </button>
        <span class="welcome__meta">About a minute</span>
      </div>
      <div class="welcome__accel-wrap">
        <button
          type="button"
          class="ghost welcome__accel"
          onclick={() => c.applyRecommendedSetup()}
          disabled={c.loading || c.applyingRecommended}
        >
          {#if c.applyingRecommended}
            Applying…
          {:else}
            Use recommended defaults
          {/if}
        </button>
        <!-- The old "Apply recommended defaults" read as if it might finish setup
             or start recording. Spell out what it actually does: presets the
             capture/processing options, then still drops into the permissions
             step (nothing records until you grant access and finish). -->
        <p class="welcome__accel-note">
          Turns on screen recording, on-screen text search, and transcription with
          sensible settings — then takes you to grant permissions. Nothing records
          until you finish setup.
        </p>
      </div>
      {#if c.errorMessage}
        <div class="welcome__err" role="alert">
          <span>{c.errorMessage}</span>
          <button
            type="button"
            class="ghost welcome__retry"
            onclick={() => c.load()}
            disabled={c.loading}
          >
            {c.loading ? "Retrying…" : "Retry"}
          </button>
        </div>
      {/if}
    </div>
  </section>
</div>

<style>
  /* Loading affordance for the "Begin setup" CTA — without it the button only
     dimmed (no label/spinner change) while `c.loading`, reading as dead. A small
     accent-on-bg spinner + "Setting up…" label confirms work is in flight,
     matching the accel button's "Applying…" idiom. */
  .cta__spin {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--app-bg) 35%, transparent);
    border-top-color: var(--app-bg);
    animation: welcome-cta-spin 0.7s linear infinite;
    flex: 0 0 auto;
  }
  @keyframes welcome-cta-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .cta__spin {
      animation: none;
    }
  }

  /* Surfaces a failed "Use recommended setup" — without it, a failed privacy-app
     exclusion silently leaves the recommended apps un-excluded (a privacy
     regression). Terminal/green danger tokens. */
  .welcome__err {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 12px;
    flex-wrap: wrap;
    margin: 12px auto 0;
    max-width: 44ch;
    padding: 10px 14px;
    font-size: 12px;
    line-height: 1.5;
    color: var(--app-danger);
    background: var(--app-danger-bg);
    border: 1px solid var(--app-danger-border);
    border-radius: 8px;
  }

  /* Recovery affordance: the welcome-phase load (`c.load()`) can fail and would
     otherwise strand a first-run user with no path forward. Retry re-runs the
     load; it disables + relabels while in flight via `c.loading`. */
  .welcome__retry {
    flex: 0 0 auto;
    padding: 4px 12px;
    font-size: 12px;
    border: 1px solid var(--app-danger-border);
    border-radius: 6px;
    color: var(--app-danger);
  }
  .welcome__retry:disabled {
    opacity: 0.6;
    cursor: default;
  }
</style>
