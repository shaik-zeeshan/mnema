<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // Sensitivity descriptor tiers mirror Capture.svelte's system-audio copy.
  const sensitivityHint = (value: number): string => {
    if (value >= 80) return "Very high — quiet system sounds keep capture active.";
    if (value >= 60) return "High — moderate system audio counts as activity.";
    if (value >= 40) return "Medium — typical media playback triggers activity. Recommended.";
    if (value >= 20) return "Low — only louder system audio keeps capture active.";
    return "Very low — only very loud system audio triggers activity.";
  };
</script>

{#if controller.featureLockReason("sysaudio")}
  <div class="lock-callout">
    <div class="lock-callout-text">
      System audio access is required before you can capture system sound.
    </div>
    <button
      type="button"
      class="btn accent sm"
      disabled={controller.requestingPerm === "systemAudio"}
      onclick={() => controller.requestPermission("systemAudio")}
    >
      {controller.requestingPerm === "systemAudio" ? "…" : "Grant System audio access"}
    </button>
  </div>
{/if}

<div class="group">
  <div class="note muted">
    System audio requires <b>Screen capture</b> (always on) and the macOS audio
    entitlement granted in Permissions.
  </div>

  <div class="ctl">
    <div class="ctl-label">
      <div class="name">Transcribe system audio</div>
      <div class="desc">Transcribe sound from meetings, videos, and apps.</div>
    </div>
    <div class="ctl-field">
      <Switch bind:checked={controller.draftTranscriptionSystemAudioEnabled} />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Detection</div>
  <div class="slider-block">
    <Slider
      bind:value={controller.draftSystemAudioActivitySensitivity}
      min={0}
      max={100}
      step={1}
      label="System audio activity sensitivity"
      unit="%"
    />
    <span class="kbd-hint">{sensitivityHint(controller.draftSystemAudioActivitySensitivity)}</span>
  </div>
</div>
