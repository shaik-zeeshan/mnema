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
      <h2 id="welcome-title" class="welcome__title">
        Your <em>memory</em>,
        <br />on rewind.
      </h2>
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
          Begin setup
          <span class="btn__arrow" aria-hidden="true">→</span>
        </button>
        <span class="welcome__meta">≈ 60 seconds</span>
      </div>
      <button
        type="button"
        class="welcome__accel"
        onclick={() => c.applyRecommendedSetup()}
        disabled={c.loading}
      >
        Use recommended setup →
      </button>
    </div>
  </section>
</div>
