<!--
  The onboarding shell (issue #195, slice 5).

  Window chrome (the 52px strip: lights, the step name, the flow ruler and
  "N / 8") is identical on every screen, so it is built once here. The eight
  screens under `screens/` are swapped into the stage; each one owns its content
  and nothing else. Navigation, persistence, the two hard gates and the atomic
  commit all live on `OnboardingFlow`.

  The previous accordion page (FeatureStack + FeatureRow + the 12 *Body.svelte
  files) was deleted in slice 17, along with the "attention item" concept.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { closeCurrentWindow } from "$lib/surface-windows";
  import "./onboarding-shell.css";
  import { OnboardingFlow, RULER, STEP_COUNT } from "./onboarding-flow.svelte";
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
    // `flow.load()` opens the ChatGPT device-login subscription (`ai.init()`),
    // which is owned by the controller precisely so it survives every SCREEN —
    // but not this page. Nothing else drops it, so without this the listener
    // (and the whole controller graph it closes over) outlives the `goto("/")`
    // at Finish/Skip and keeps re-verifying against a dead store on every later
    // ChatGPT sign-in from Settings.
    return () => c.ai.disposeChatgptLoginUpdates();
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

  <!-- The window is undecorated and this strip replaces the shell's Close chip
       and drag region: the first light IS the close button (the other two are
       drawn), and every non-control part of the strip drags the window. -->
  <header class="ob-bar" data-tauri-drag-region>
    <div class="ob-lights" data-tauri-drag-region>
      <button type="button" aria-label="Close window" onclick={() => void closeCurrentWindow()}></button>
      <i aria-hidden="true"></i><i aria-hidden="true"></i>
    </div>
    <span class="ob-bar__name" data-tauri-drag-region>
      {flow.stepLabel}{#if flow.stepSuffix}<small>{flow.stepSuffix}</small>{/if}
    </span>
    <div class="ob-rail" aria-hidden="true" data-tauri-drag-region>
      {#each RULER as tick (tick.position)}
        <i
          data-opt={tick.optional || undefined}
          data-done={tick.position < flow.stepPosition || undefined}
          data-cur={tick.position === flow.stepPosition || undefined}
        ></i>
      {/each}
    </div>
    <span class="ob-bar__count" aria-hidden="true" data-tauri-drag-region><b>{flow.stepPosition}</b> / {STEP_COUNT}</span>
  </header>

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
