<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";

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
</script>

{#if controller.featureLockReason("mic")}
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
      {controller.requestingPerm === "microphone" ? "…" : "Grant Microphone access"}
    </button>
  </div>
{/if}

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
</div>
