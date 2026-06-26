<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import type { MicrophoneVadAdapter } from "$lib/types";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { useLockCalloutSlot } from "./FeatureRow.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // Sensitivity descriptor tiers mirror Capture.svelte's microphone copy. The
  // onboarding draft has no input-device field (device selection lives in
  // Settings), so the mockup's device picker is intentionally omitted.
  const sensitivityHint = (value: number): string => {
    if (value >= 80) return "Very high — whispers and background noise keep capture active.";
    if (value >= 60) return "High — quiet speech counts as activity.";
    if (value >= 40) return "Medium — normal speech triggers activity. Recommended.";
    if (value >= 20) return "Low — only louder audio keeps capture active.";
    return "Very low — only very loud audio triggers activity.";
  };

  // With a Segmented the per-option blurb that RadioGroup showed inline collapses
  // to one contextual hint line for the active adapter.
  const vadHint = (value: MicrophoneVadAdapter): string => {
    switch (value) {
      case "silero":
        return "Silero — default on-device speech detector; falls back to WebRTC when unavailable.";
      case "webrtc":
        return "WebRTC — lightweight on-device speech detector.";
      case "off":
        return "Off — legacy microphone peak-level activity, tuned by the sensitivity slider below.";
    }
  };

  // Hoist the unmet-prerequisite callout OUT of FeatureRow's inert `.body-inner`
  // (registered every render so it tracks the lock state) — otherwise its "Grant
  // Microphone access" button renders but is itself inert and does nothing.
  const setLockCallout = useLockCalloutSlot();
  const lockReason = $derived(controller.featureLockReason("mic"));
  $effect(() => {
    setLockCallout(lockReason ? lockCallout : null);
    return () => setLockCallout(null);
  });
</script>

{#snippet lockCallout()}
  <!-- macOS won't re-prompt once mic access is denied, so `requestPermission`
       deep-links to System Settings instead — mirror PermissionsBody and relabel
       the button ("Open Settings"/"Opening…") so it doesn't promise a prompt that
       never appears. `permissionAction` returns the mode for the current state. -->
  {@const micAction = controller.permissionAction(controller.permissions?.microphone)}
  <div class="lock-callout">
    <div class="lock-callout-text">
      Microphone access is required before you can record your mic.
    </div>
    <button
      type="button"
      class="btn accent sm"
      disabled={controller.requestingPerm === "microphone"}
      onclick={() => controller.requestPermission("microphone")}
    >
      {#if controller.requestingPerm === "microphone"}
        {micAction?.mode === "settings" ? "Opening…" : "Requesting…"}
      {:else}
        {micAction?.mode === "settings" ? "Open Settings" : "Grant Microphone access"}
      {/if}
    </button>
  </div>
{/snippet}

<div class="group">
  <div class="ctl">
    <div class="ctl-label">
      <div class="name">Transcribe microphone</div>
      <div class="desc">Run captured mic audio through transcription.</div>
    </div>
    <div class="ctl-field">
      <Switch bind:checked={controller.draftTranscriptionMicrophoneEnabled} />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Detection</div>
  <!-- Onboarding always forces activityMode to
       "system_input_or_screen_or_audio" (syncDraftsInto + buildSettingsRequestFrom),
       so unlike Capture.svelte there is no activity-mode {#if} gate — the VAD
       control always renders when the mic body is shown. Three mutually-exclusive
       adapters fit a Segmented (matching the provider pickers in the other
       bodies); the per-adapter detail moves to the hint line below. -->
  <Segmented
    value={controller.draftMicrophoneVadAdapter}
    onValueChange={(v) => controller.chooseMicrophoneVadAdapter(v)}
    ariaLabel="Microphone voice activity detection"
    disabled={!controller.draftCaptureMicrophone}
    options={[
      { value: "silero", label: "Silero" },
      { value: "webrtc", label: "WebRTC" },
      { value: "off", label: "Off" },
    ]}
  />
  <span class="kbd-hint">{vadHint(controller.draftMicrophoneVadAdapter)}</span>

  <!-- The peak-level sensitivity slider only matters when VAD is Off (matches
       Capture.svelte). With Silero/WebRTC the adapter owns activity decisions. -->
  {#if controller.draftMicrophoneVadAdapter === "off"}
    <div class="slider-block">
      <Slider
        bind:value={controller.draftMicrophoneActivitySensitivity}
        min={0}
        max={100}
        step={1}
        label="Microphone activity sensitivity"
        unit="%"
      />
      <span class="kbd-hint">{sensitivityHint(controller.draftMicrophoneActivitySensitivity)}</span>
    </div>
  {/if}
</div>
