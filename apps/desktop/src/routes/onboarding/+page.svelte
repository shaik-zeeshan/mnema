<script lang="ts">
  import { untrack } from "svelte";
  // Load the global onboarding stylesheet here too: FeatureStack (which also
  // imports it) is UNMOUNTED during the welcome/finale phases, so the welcome
  // and finale `.ob-screen` styles must be loaded from the always-mounted page.
  // The import is global + idempotent, so the duplicate in FeatureStack is fine.
  import "./onboarding-ui.css";
  import "./onboarding-body.css";
  import "./onboarding-screens.css";
  import FeatureStack from "./FeatureStack.svelte";
  import FeatureRow from "./FeatureRow.svelte";
  import WelcomeScreen from "./WelcomeScreen.svelte";
  import FinaleScreen from "./FinaleScreen.svelte";
  import { FEATURES } from "./feature-model";
  import { OnboardingController } from "./onboarding.svelte";
  import PermissionsBody from "./PermissionsBody.svelte";
  import ScreenBody from "./ScreenBody.svelte";
  import StorageBody from "./StorageBody.svelte";
  import MicBody from "./MicBody.svelte";
  import SystemAudioBody from "./SystemAudioBody.svelte";
  import OcrBody from "./OcrBody.svelte";
  import TranscriptionBody from "./TranscriptionBody.svelte";
  import SpeakersBody from "./SpeakersBody.svelte";
  import PrivacyBody from "./PrivacyBody.svelte";
  import AskAiBody from "./AskAiBody.svelte";
  import SemanticSearchBody from "./SemanticSearchBody.svelte";

  const c = new OnboardingController();

  // Cross-phase wayfinding: map the three-state phase machine
  // (welcome → configure → done) to a 1-based step so the shell can show a quiet
  // "Welcome · Configure · Finish" stepper telling the user where they are and
  // what remains. Purely presentational — derived from the existing phase state.
  const phaseStep = $derived(c.phase === "welcome" ? 1 : c.phase === "configure" ? 2 : 3);

  // Spoken step announcement for the polite live region below: a phase change is a
  // full content swap (welcome hero ↔ accordion ↔ finale crest), so without an
  // announcement a screen-reader user only hears silence after activating the CTA.
  const phaseAnnouncement = $derived(
    c.phase === "welcome"
      ? "Step 1 of 3: Welcome."
      : c.phase === "configure"
        ? "Step 2 of 3: Configure what mnema records."
        : "Step 3 of 3: Finish and start recording.",
  );

  // Move focus to the new phase's heading on every transition. Each phase renders
  // a different component tree, so without this the focus is stranded on the
  // (now-unmounted) button the user just pressed — keyboard/SR users would land on
  // <body>. The headings carry `tabindex="-1"` + `data-ob-phase-heading` so they
  // accept programmatic focus. The first run (initial mount) is skipped so we
  // don't yank focus to the heading before the user has interacted.
  let phaseFocusPrimed = false;
  $effect(() => {
    c.phase; // track phase transitions
    if (!phaseFocusPrimed) {
      phaseFocusPrimed = true;
      return;
    }
    requestAnimationFrame(() => {
      document.querySelector<HTMLElement>("[data-ob-phase-heading]")?.focus();
    });
  });

  // Mount loaders. The `untrack` is MANDATORY: without it, editing a draft
  // re-runs init and reverts the edit (known onboarding/settings mount-effect
  // bug in this app — see CLAUDE memory "Settings init effect untrack").
  $effect(() => {
    untrack(() => {
      void c.load();
      void c.loadModelStatuses();
    });
  });

  // Listeners (OCR/transcription download progress + recording-settings-changed)
  // set up async; guard against a late resolve landing after teardown.
  $effect(() => {
    let unlisten: (() => void) | undefined;
    let destroyed = false;
    void c.startListeners().then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });
    return () => {
      destroyed = true;
      unlisten?.();
    };
  });

  // Custom-input validation effects (parse raw → clamped numbers). The clamp
  // ranges live on the controller so the result matches the legacy page.
  $effect(() => {
    c.customWidthRaw;
    untrack(() => c.syncCustomWidth());
  });
  $effect(() => {
    c.customHeightRaw;
    untrack(() => c.syncCustomHeight());
  });
  $effect(() => {
    c.draftCustomMbpsRaw;
    untrack(() => c.syncCustomMbps());
  });
</script>

<div class="onboarding-root">
  <!-- Polite step announcement for assistive tech — phase transitions otherwise
       swap the whole screen silently. Visually hidden; mirrors the access-request
       page's `.sr-only` live-status pattern. -->
  <span class="ob-sr-only" role="status" aria-live="polite">{phaseAnnouncement}</span>
  <nav class="ob-progress" aria-label="Setup progress">
    <ol class="ob-progress__list">
      <li
        class="ob-progress__step"
        class:is-active={phaseStep === 1}
        class:is-done={phaseStep > 1}
        aria-current={phaseStep === 1 ? "step" : undefined}
      >
        <span class="ob-progress__dot" aria-hidden="true"></span>
        <span class="ob-progress__label">Welcome</span>
      </li>
      <li
        class="ob-progress__step"
        class:is-active={phaseStep === 2}
        class:is-done={phaseStep > 2}
        aria-current={phaseStep === 2 ? "step" : undefined}
      >
        <span class="ob-progress__dot" aria-hidden="true"></span>
        <span class="ob-progress__label">Configure</span>
      </li>
      <li
        class="ob-progress__step"
        class:is-active={phaseStep === 3}
        aria-current={phaseStep === 3 ? "step" : undefined}
      >
        <span class="ob-progress__dot" aria-hidden="true"></span>
        <span class="ob-progress__label">Finish</span>
      </li>
    </ol>
  </nav>
  {#if c.phase === "welcome"}
    <WelcomeScreen controller={c} />
  {:else if c.phase === "done"}
    <FinaleScreen controller={c} />
  {:else}
    <FeatureStack
      onCount={c.onCount}
      attentionCount={c.attentionCount}
      blockReason={c.configureBlockReason}
      onJumpToAttention={c.firstConfigureBlockerId ? () => c.jumpToFirstAttention() : undefined}
      ctaLabel="Review &amp; finish →"
      ctaDisabled={!c.canProceedToFinale}
      errorMessage={c.errorMessage}
      onFinish={() => c.reviewAndFinish()}
      secondaryLabel="← Back"
      onSecondary={() => c.backToWelcome()}
    >
    {#each FEATURES as f (f.id)}
      <FeatureRow
        featureId={f.id}
        icon={f.icon}
        name={f.name}
        eyebrow={f.eyebrow}
        sub={f.sub}
        required={f.required}
        enabled={c.isEnabled(f.id)}
        open={c.openId === f.id}
        attention={c.featureAttention(f.id)}
        toggleDisabled={c.featureToggleDisabled(f.id)}
        lockReason={c.featureLockReason(f.id)}
        download={c.featureDownload(f.id)}
        onToggle={() => c.toggleFeature(f.id)}
        onExpand={() => c.setOpen(f.id)}
      >
        {#snippet body()}
          {#if f.id === "permissions"}
            <PermissionsBody controller={c} />
          {:else if f.id === "screen"}
            <ScreenBody controller={c} />
          {:else if f.id === "storage"}
            <StorageBody controller={c} />
          {:else if f.id === "mic"}
            <MicBody controller={c} />
          {:else if f.id === "sysaudio"}
            <SystemAudioBody controller={c} />
          {:else if f.id === "ocr"}
            <OcrBody controller={c} />
          {:else if f.id === "transcribe"}
            <TranscriptionBody controller={c} />
          {:else if f.id === "speakers"}
            <SpeakersBody controller={c} />
          {:else if f.id === "privacy"}
            <PrivacyBody controller={c} />
          {:else if f.id === "askai"}
            <AskAiBody controller={c} />
          {:else if f.id === "semanticSearch"}
            <SemanticSearchBody controller={c} />
          {/if}
        {/snippet}
      </FeatureRow>
    {/each}
    </FeatureStack>
  {/if}
</div>

<style>
  /* Give the accordion shell a real viewport height to resolve against. The
     onboarding route renders full-window (no main titlebar), and the shell's
     `.onboarding-shell { height: 100% }` needs a definite-height parent —
     WKWebView collapses `height:100%` against a flex-stretched parent, so we
     pin the root to the dynamic viewport height and let the shell flex to fill.
  */
  .onboarding-root {
    height: 100vh;
    height: 100dvh;
    display: flex;
    flex-direction: column;
    min-height: 0;
    position: relative;
  }

  /* Visually-hidden live region (step announcements). Same recipe as the
     access-request page's `.sr-only`. */
  .ob-sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  /* Programmatic focus targets (phase headings) must not paint a focus ring — the
     focus is moved by the phase-transition effect, not a user tab, so a ring here
     would read as a stray highlight. */
  :global(.onboarding-root [data-ob-phase-heading]:focus) {
    outline: none;
  }
  :global(.onboarding-root [data-ob-phase-heading]:focus-visible) {
    outline: none;
  }

  /* Quiet cross-phase wayfinding stepper, pinned top-center across all three
     phases (welcome hero / configure accordion / finale crest) so the user can
     always see where they are and what remains. Non-interactive — purely a
     signifier — so it ignores pointer events. */
  .ob-progress {
    position: absolute;
    top: 16px;
    left: 0;
    right: 0;
    z-index: 5;
    display: flex;
    justify-content: center;
    pointer-events: none;
    font-family: var(--app-font-mono);
  }
  .ob-progress__list {
    display: flex;
    align-items: center;
    gap: 18px;
    margin: 0;
    padding: 6px 14px;
    list-style: none;
    border-radius: var(--app-radius-pill, 999px);
    background: color-mix(in srgb, var(--app-surface) 70%, transparent);
    border: 1px solid var(--app-border);
  }
  .ob-progress__step {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: var(--text-sm);
    letter-spacing: 0.02em;
    color: var(--app-text-subtle);
  }
  /* Connector tick between steps. */
  .ob-progress__step:not(:last-child)::after {
    content: "";
    width: 14px;
    height: 1px;
    margin-left: 11px;
    background: var(--app-border);
  }
  .ob-progress__dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: transparent;
    flex: 0 0 auto;
  }
  /* Completed steps: filled accent dot, muted label. */
  .ob-progress__step.is-done {
    color: var(--app-text-muted);
  }
  .ob-progress__step.is-done .ob-progress__dot {
    background: var(--app-accent);
    border-color: var(--app-accent);
  }
  /* Current step: accent label + weight per the active-nav contract, glowing dot. */
  .ob-progress__step.is-active {
    color: var(--app-accent);
    font-weight: 600;
  }
  .ob-progress__step.is-active .ob-progress__dot {
    background: var(--app-accent);
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-bg);
  }
</style>
