<!--
  The onboarding shell (issue #195, slice 5).

  Window chrome (traffic-light dots, the step label, the playhead ruler and
  "N / 8") is identical on every screen, so it is built once here. The eight
  screens under `screens/` are swapped into the stage; each one owns its content
  and nothing else. Navigation, persistence, the two hard gates and the atomic
  commit all live on `OnboardingFlow`.

  The previous accordion page (FeatureStack + FeatureRow + the 12 *Body.svelte
  files) was deleted in slice 17, along with the "attention item" concept.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import "./onboarding-shell.css";
  import { OnboardingFlow, STEP_COUNT } from "./onboarding-flow.svelte";
  import WelcomeScreen from "./screens/WelcomeScreen.svelte";
  import PermissionsScreen from "./screens/PermissionsScreen.svelte";
  import CaptureStorageScreen from "./screens/CaptureStorageScreen.svelte";
  import YourSettingsScreen from "./screens/YourSettingsScreen.svelte";
  import ChangeSettingsScreen from "./screens/ChangeSettingsScreen.svelte";
  import SetupScreen from "./screens/SetupScreen.svelte";
  import VoiceScreen from "./screens/VoiceScreen.svelte";
  import FinaleScreen from "./screens/FinaleScreen.svelte";

  const flow = new OnboardingFlow();
  const c = flow.controller;

  // Playhead geometry, ported from the mockup: a 144px track with eight ticks
  // at 9px + 18px each. The fill and the head both sit at the current step.
  const TICKS = Array.from({ length: STEP_COUNT }, (_, i) => 9 + i * 18);
  const headX = $derived(TICKS[flow.stepPosition - 1] ?? TICKS[0]);

  const announcement = $derived(
    `Step ${flow.stepPosition} of ${STEP_COUNT}: ${flow.stepLabel}.`,
  );

  // Mount loaders. The `untrack` is MANDATORY: without it, editing a draft
  // re-runs init and reverts the edit (known onboarding/settings mount-effect
  // bug in this app — see CLAUDE memory "Settings init effect untrack").
  $effect(() => {
    untrack(() => {
      void flow.load();
    });
  });

  // Download-progress + recording-settings-changed listeners; guard against a
  // late resolve landing after teardown.
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

  // Custom-input validation (raw → clamped numbers). Feeds the range half of the
  // Capture & Storage gate.
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

  // Move focus to the new screen's heading on every transition, so a keyboard or
  // screen-reader user is not stranded on the (now unmounted) button they just
  // pressed. Skipped on first mount.
  let stage = $state<HTMLElement | null>(null);
  let focusPrimed = false;
  $effect(() => {
    flow.step;
    if (!focusPrimed) {
      focusPrimed = true;
      return;
    }
    requestAnimationFrame(() => stage?.focus());
  });
</script>

<div class="ob-win">
  <span class="ob-sr-only" role="status" aria-live="polite">{announcement}</span>

  <div class="ob-bar">
    <div class="ob-dots" aria-hidden="true"><i></i><i></i><i></i></div>
    <span class="ob-name">{flow.stepLabel}</span>
    <div class="ob-ph">
      <div class="ob-ph-track" aria-hidden="true">
        {#each TICKS as x (x)}
          <i class="ob-ph-tick" style="left:{x}px"></i>
        {/each}
        <b class="ob-ph-fill" style="width:{headX}px"></b>
        <em class="ob-ph-head" style="left:{headX}px"></em>
      </div>
      <span class="ob-ph-lbl">
        {flow.stepPosition} / {STEP_COUNT}{flow.stepSuffix ? ` ${flow.stepSuffix}` : ""}
      </span>
    </div>
  </div>

  <div class="ob-stage" bind:this={stage} tabindex="-1">
    {#if flow.step === "welcome"}
      <WelcomeScreen {flow} onContinue={() => flow.next()} />
    {:else if flow.step === "permissions"}
      <PermissionsScreen {flow} onContinue={() => flow.next()} onBack={() => flow.back()} />
    {:else if flow.step === "captureStorage"}
      <CaptureStorageScreen {flow} onContinue={() => flow.next()} onBack={() => flow.back()} />
    {:else if flow.step === "yourSettings"}
      <YourSettingsScreen
        {flow}
        onContinue={() => flow.next()}
        onBack={() => flow.back()}
        onChangeSettings={() => flow.goTo("changeSettings")}
      />
    {:else if flow.step === "changeSettings"}
      <ChangeSettingsScreen {flow} onContinue={() => flow.next()} onBack={() => flow.back()} />
    {:else if flow.step === "setup"}
      <SetupScreen {flow} onContinue={() => flow.next()} onBack={() => flow.back()} />
    {:else if flow.step === "voice"}
      <VoiceScreen
        {flow}
        onContinue={() => flow.next()}
        onBack={() => flow.back()}
        onSkip={() => flow.next()}
      />
    {:else}
      <FinaleScreen
        {flow}
        onBack={() => flow.back()}
        onFinish={(startRecording = true) => flow.finish(startRecording)}
      />
    {/if}
  </div>
</div>

<style>
  /* The stage is a programmatic focus target (moved on every step transition,
     not by a user tab), so it must not paint a focus ring. */
  :global(.ob-stage:focus) {
    outline: none;
  }
</style>
