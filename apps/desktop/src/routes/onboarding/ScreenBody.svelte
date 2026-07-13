<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import ScreenResolutionControl from "$lib/components/ScreenResolutionControl.svelte";
  import VideoBitrateControl from "$lib/components/VideoBitrateControl.svelte";
  import CaptureRateControl from "$lib/components/CaptureRateControl.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Switch from "$lib/components/Switch.svelte";

  let { controller }: { controller: OnboardingController } = $props();
</script>

<div class="group">
  <div class="group-title">Video</div>

  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Resolution</div>
      <div class="desc">Higher resolution = sharper text, larger files.</div>
    </div>
    <div class="ctl-field">
      <ScreenResolutionControl
        bind:mode={controller.draftResolutionMode}
        bind:preset={controller.draftResolutionPreset}
        bind:widthRaw={controller.customWidthRaw}
        bind:heightRaw={controller.customHeightRaw}
        customErrors={controller.customResolutionErrors}
      />
    </div>
  </div>

  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Video bitrate</div>
      <div class="desc">Quality of the encoded stream.</div>
    </div>
    <div class="ctl-field">
      <VideoBitrateControl
        bind:mode={controller.draftBitrateMode}
        bind:preset={controller.draftBitratePreset}
        bind:customMbpsRaw={controller.draftCustomMbpsRaw}
        customMbps={controller.draftCustomMbps}
        customErrors={controller.customBitrateErrors}
      />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Timing</div>

  <div class="slider-block">
    <CaptureRateControl bind:value={controller.draftFrameRate} />
  </div>

  <div class="slider-block">
    <Slider
      bind:value={controller.draftSegmentDuration}
      min={10}
      max={300}
      step={10}
      label="Segment duration"
      formatValue={(v) => (v >= 60 ? `${Math.floor(v / 60)}m ${v % 60}s` : `${v}s`)}
    />
    <span class="kbd-hint">Capped at 5 minutes per segment.</span>
  </div>
</div>

<div class="group">
  <div class="group-title">Idle</div>

  <div class="ctl">
    <div class="ctl-label">
      <div class="name">Pause capture when idle</div>
      <div class="desc">
        Pause recording after the system is idle, and resume when activity returns.
      </div>
    </div>
    <div class="ctl-field">
      <Switch bind:checked={controller.draftPauseCaptureOnInactivity} />
    </div>
  </div>

  {#if controller.draftPauseCaptureOnInactivity}
    <div class="slider-block">
      <Slider
        bind:value={controller.draftIdleTimeoutSeconds}
        min={5}
        max={300}
        step={5}
        label="Idle timeout"
        formatValue={(v) =>
          v >= 60 ? `${Math.floor(v / 60)}m${v % 60 > 0 ? ` ${v % 60}s` : ""}` : `${v}s`}
      />
      <span class="kbd-hint">
        Capture pauses after {controller.draftIdleTimeoutSeconds}s of system-wide inactivity, and
        resumes when input is detected again.
      </span>
    </div>
  {/if}
</div>
