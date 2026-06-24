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
  {#if c.phase === "welcome"}
    <WelcomeScreen controller={c} />
  {:else if c.phase === "done"}
    <FinaleScreen controller={c} />
  {:else}
    <FeatureStack
      onCount={c.onCount}
      attentionCount={c.attentionCount}
      ctaLabel="Review &amp; finish →"
      ctaDisabled={!c.canProceedToFinale}
      errorMessage={c.errorMessage}
      onFinish={() => c.reviewAndFinish()}
      secondaryLabel="← Back"
      onSecondary={() => c.backToWelcome()}
    >
    {#each FEATURES as f (f.id)}
      <FeatureRow
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
  }
</style>
