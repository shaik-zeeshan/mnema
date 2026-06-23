<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import { formatBytes } from "./onboarding-mapping";

  let { controller }: { controller: OnboardingController } = $props();

  // The row toggle owns "separate speakers" (the feature enable). This body
  // carries the rest: recognition opt-in, helper timeout, and the on-device
  // model preset + download. Mirrors Speakers.svelte; the status pill reuses the
  // shared `.pill` family (granted/pending/denied).
  const model = $derived(controller.selectedSpeakerModel);

  const pillClass = $derived.by(() => {
    if (!model) return "pending";
    if (model.available) return "granted";
    if (controller.selectedSpeakerDownloadRunning) return "pending";
    return "denied";
  });
</script>

<div class="group">
  <div class="note muted">
    Speaker separation runs <b>locally</b> after microphone transcription — keep
    Audio transcription on so there's a transcript to split.
  </div>

  <div class="ctl">
    <div class="ctl-label">
      <div class="name">Recognize saved people</div>
      <div class="desc">
        Match voices against confirmed local Person profiles. Off keeps speakers
        anonymous (Speaker 1, Speaker 2…).
      </div>
    </div>
    <div class="ctl-field">
      <Switch bind:checked={controller.draftSpeakerRecognizeSavedPeople} />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Timing</div>
  <div class="slider-block">
    <Slider
      bind:value={controller.draftSpeakerTimeoutMinutes}
      min={1}
      max={60}
      step={1}
      label="Helper timeout"
      unit="m"
    />
    <span class="kbd-hint">Stops speaker analysis if the local helper runs longer than this.</span>
  </div>
</div>

<div class="group">
  <div class="group-title">Model</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Speaker model preset</div>
      <div class="desc">
        Each preset bundles a segmentation and a speaker-embedding model. The
        download size is shown in the list.
      </div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <Combobox
        value={controller.selectedSpeakerPresetKey}
        onValueChange={(v) => controller.chooseSpeakerModel(v)}
        searchPlaceholder="Search presets…"
        options={controller.speakerModelOptions.length > 0
          ? controller.speakerModelOptions
          : [{ value: controller.selectedSpeakerPresetKey, label: "Loading preset options" }]}
      />
    </div>
  </div>

  {#if controller.speakerModelError}
    <div class="note">Failed to load speaker model status: {controller.speakerModelError}</div>
  {:else if model}
    <div class="model-card">
      <div class="model-top">
        <div class="model-id">
          {model.displayName}
          <div class="meta">{controller.speakerStatusLabel(model)}{model.description ? ` · ${model.description}` : ""}</div>
        </div>
        <span class="pill {pillClass}">
          <span class="d"></span>{controller.speakerStatusLabel(model)}
        </span>
      </div>

      {#if model.download}
        {#if controller.selectedSpeakerDownloadRunning}
          <div class="dl">
            <div class="dl-track">
              <div class="dl-fill" style={`width: ${controller.selectedSpeakerDownloadPercent ?? 8}%`}></div>
            </div>
            <div class="dl-meta">
              <span>
                <b>{controller.selectedSpeakerDownloadPercent ?? 0}%</b>
                · {controller.selectedSpeakerDownloadProgress?.status ?? "downloading"}
              </span>
              <button
                type="button"
                class="btn sm danger"
                disabled={controller.cancellingSpeakerDownload}
                onclick={() => controller.cancelSelectedSpeakerModelDownload()}
              >
                {controller.cancellingSpeakerDownload ? "Cancelling…" : "Cancel"}
              </button>
            </div>
          </div>
        {:else}
          <div class="dl-meta">
            <span>{model.available ? "Downloaded" : "Not downloaded"} · {formatBytes(model.download.byteSize)}</span>
            <button
              type="button"
              class="btn sm accent"
              disabled={controller.startingSpeakerDownload || model.available}
              onclick={() => controller.startSelectedSpeakerModelDownload()}
            >
              {controller.startingSpeakerDownload ? "Starting…" : "Download"}
            </button>
          </div>
        {/if}
        {#if controller.speakerDownloadError}
          <div class="note">Download failed: {controller.speakerDownloadError}</div>
        {/if}
      {:else if !model.available}
        <div class="note">
          This preset is missing, but no packaged download artifact is available in the
          current manifest.
        </div>
      {/if}
    </div>
  {:else if controller.loadingSpeakerModelStatus}
    <div class="note muted">Checking installed speaker models…</div>
  {:else}
    <div class="note">No speaker model status is available.</div>
  {/if}
</div>
