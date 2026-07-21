<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import { useLockCalloutSlot } from "./FeatureRow.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // Sensitivity descriptor tiers mirror Capture.svelte's system-audio copy.
  const sensitivityHint = (value: number): string => {
    if (value >= 80) return "Very high — quiet system sounds keep capture active.";
    if (value >= 60) return "High — moderate system audio counts as activity.";
    if (value >= 40) return "Medium — typical media playback triggers activity. Recommended.";
    if (value >= 20) return "Low — only louder system audio keeps capture active.";
    return "Very low — only very loud system audio triggers activity.";
  };

  // Hoist the callout OUT of FeatureRow's inert `.body-inner` — otherwise its
  // "Grant System audio access" button renders but is inert.
  //
  // Driven by the permission itself rather than `featureLockReason`, which is
  // null for sysaudio and must stay so: a tap's grant is unreadable and its
  // prompt only fires on a real recording, so this is an early offer, not an
  // unmet prerequisite (ADR 0052). Unlike MicBody's, it is therefore shown only
  // while the feature is ON — with no lock to open, a Grant button on a switched
  // off feature is just noise. It clears once a tap has proved the grant by
  // delivering sound ("assumed_working" → no action).
  const setLockCallout = useLockCalloutSlot();
  const sysAction = $derived(controller.permissionAction(controller.permissions?.systemAudio));
  // Once the prompt has been raised this session, the "get it out of the way"
  // offer is served — re-requesting is a macOS no-op and the callout would
  // wrongly read as still-ungranted (the grant itself is unreadable, ADR 0052).
  // `possibly_blocked` keeps the callout: that copy routes to System Settings.
  const promptServed = $derived(
    controller.sysAudioPromptRaised && controller.permissions?.systemAudio === "not_determined",
  );
  const showGrant = $derived(
    controller.draftCaptureSystemAudio && sysAction !== null && !promptServed,
  );
  $effect(() => {
    setLockCallout(showGrant ? lockCallout : null);
    return () => setLockCallout(null);
  });
</script>

{#snippet lockCallout()}
  <div class="lock-callout">
    <div class="lock-callout-text">
      {#if controller.permissions?.systemAudio === "possibly_blocked"}
        System audio hasn't recorded any sound yet. If you denied the prompt,
        allow Mnema under Privacy &amp; Security → Screen &amp; System Audio Recording.
      {:else}
        macOS asks for system audio access the first time a recording captures
        sound. Grant it now to get the prompt out of the way.
      {/if}
    </div>
    <button
      type="button"
      class="btn accent sm"
      disabled={controller.requestingPerm === "systemAudio"}
      onclick={() => controller.requestPermission("systemAudio")}
    >
      {#if controller.requestingPerm === "systemAudio"}
        {sysAction?.mode === "settings" ? "Opening…" : "Requesting…"}
      {:else}
        {sysAction?.mode === "settings" ? "Open Settings" : "Grant System audio access"}
      {/if}
    </button>
  </div>
{/snippet}

<div class="group">
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
