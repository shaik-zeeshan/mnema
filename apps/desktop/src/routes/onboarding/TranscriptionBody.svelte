<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import type { AudioTranscriptionMemoryMode } from "$lib/types";
  import Segmented from "$lib/components/Segmented.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import AdvancedReveal from "./AdvancedReveal.svelte";
  import { formatBytes } from "./onboarding-mapping";
  import { TRANSCRIPTION_LANGUAGE_OPTIONS } from "$lib/settings/transcription-languages";

  let { controller }: { controller: OnboardingController } = $props();

  // Mirrors Transcription.svelte: the Combobox selects the model; the card
  // reflects the resolved `selectedTranscriptionModel` and drives the in-app
  // download. The status pill reuses the mockup's `.pill` family.
  const model = $derived(controller.selectedTranscriptionModel);

  const pillClass = $derived.by(() => {
    if (!model) return "pending";
    if (model.available) return "granted";
    if (controller.selectedTranscriptionDownloadRunning) return "pending";
    return "denied";
  });
</script>

{#if controller.transcriptionRequestedWhileOff}
  <div class="lock-callout">
    <div class="lock-callout-text">
      Microphone or System audio is set to be transcribed, but Audio transcription is off.
      Turn it on to transcribe captured speech.
    </div>
  </div>
{/if}

<div class="group">
  <div class="group-title">Provider</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Transcription provider</div>
      <div class="desc">
        Local Whisper is on-device and private; Apple Speech needs no download; Parakeet is a
        fast NeMo model.
      </div>
    </div>
    <div class="ctl-field">
      <Segmented
        value={controller.draftTranscriptionProvider}
        onValueChange={(v) => controller.chooseTranscriptionProvider(v)}
        ariaLabel="Transcription provider"
        options={[
          { value: "local_whisper", label: "Local Whisper" },
          { value: "apple_speech_on_device", label: "Apple Speech" },
          { value: "parakeet", label: "Parakeet" },
        ]}
      />
    </div>
  </div>
</div>

<div class="group">
  <div class="group-title">Model</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Model</div>
      <div class="desc">Bigger models are more accurate, slower, and use more disk.</div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <Combobox
        value={controller.draftTranscriptionModelId ?? "__os_managed__"}
        onValueChange={(v) => controller.chooseTranscriptionModel(v)}
        searchPlaceholder="Search models…"
        options={controller.transcriptionModelOptions.length > 0
          ? controller.transcriptionModelOptions
          : [{ value: controller.draftTranscriptionModelId ?? "__os_managed__", label: "Loading model options" }]}
      />
    </div>
  </div>

  {#if controller.transcriptionModelError}
    <div class="note">Failed to load model status: {controller.transcriptionModelError}</div>
  {:else if model}
    <div class="model-card">
      <div class="model-top">
        <div class="model-id">
          {model.displayName}
          <div class="meta">{controller.transcriptionStatusLabel(model)}{model.description ? ` · ${model.description}` : ""}</div>
        </div>
        <span class="pill {pillClass}">
          <span class="d"></span>{controller.transcriptionStatusLabel(model)}
        </span>
      </div>

      {#if model.management === "app_managed"}
        {#if controller.selectedTranscriptionDownloadRunning}
          <div class="dl">
            <div class="dl-track">
              <div class="dl-fill" style={`width: ${controller.selectedTranscriptionDownloadPercent ?? 8}%`}></div>
            </div>
            <div class="dl-meta">
              <span>
                <b>{controller.selectedTranscriptionDownloadPercent ?? 0}%</b>
                · {controller.selectedTranscriptionDownloadProgress?.status ?? "downloading"}
              </span>
              <button
                type="button"
                class="btn sm danger"
                disabled={controller.cancellingTranscriptionDownload}
                onclick={() => controller.cancelSelectedTranscriptionModelDownload()}
              >
                {controller.cancellingTranscriptionDownload ? "Cancelling…" : "Cancel"}
              </button>
            </div>
          </div>
        {:else if model.download}
          <div class="dl-meta">
            <span>{model.available ? "Downloaded" : "Not downloaded"} · {formatBytes(model.download.byteSize)}</span>
            <button
              type="button"
              class="btn sm accent"
              disabled={controller.startingTranscriptionDownload || model.available}
              onclick={() => controller.startSelectedTranscriptionModelDownload()}
            >
              {controller.startingTranscriptionDownload ? "Starting…" : "Download"}
            </button>
          </div>
        {:else if !model.available}
          <div class="note">
            This app-managed model is missing, but no packaged download artifact is available
            in the current manifest.
          </div>
        {/if}
        {#if controller.transcriptionDownloadError}
          <div class="note">Download failed: {controller.transcriptionDownloadError}</div>
        {/if}
      {:else}
        <div class="note muted">This provider is managed by macOS. There is no app-managed model download.</div>
      {/if}
    </div>
  {:else if controller.loadingTranscriptionModelStatus}
    <div class="note muted">Checking installed transcription models…</div>
  {:else}
    <div class="note">No model status is available for the selected provider.</div>
  {/if}
</div>

<div class="group">
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Language</div>
      <div class="desc">Auto detects the spoken language, or pin a specific one.</div>
    </div>
    <div class="ctl-field">
      <Combobox
        value={controller.draftTranscriptionLanguage || "auto"}
        onValueChange={(v) => (controller.draftTranscriptionLanguage = v)}
        options={TRANSCRIPTION_LANGUAGE_OPTIONS}
        searchPlaceholder="Search languages…"
      />
    </div>
  </div>
</div>

{#if controller.draftTranscriptionProvider === "parakeet"}
  <div class="group">
    <div class="ctl stack-field">
      <div class="ctl-label">
        <div class="name">Parakeet memory mode</div>
        <div class="desc">Balanced unloads when idle; Low memory minimizes footprint; Performance keeps it hot.</div>
      </div>
      <div class="ctl-field">
        <Segmented
          value={controller.draftTranscriptionMemoryMode}
          onValueChange={(v) => (controller.draftTranscriptionMemoryMode = v as AudioTranscriptionMemoryMode)}
          ariaLabel="Parakeet memory mode"
          options={[
            { value: "balanced", label: "Balanced" },
            { value: "low_memory", label: "Low memory" },
            { value: "performance", label: "Performance" },
          ]}
        />
      </div>
    </div>
  </div>

  <div class="group">
    <AdvancedReveal>
      {#if controller.draftTranscriptionMemoryMode === "balanced"}
        <div class="slider-block">
          <Slider
            bind:value={controller.draftTranscriptionIdleUnloadSeconds}
            min={0}
            max={1800}
            step={30}
            label="Idle unload"
            formatValue={(v) => (v === 0 ? "off" : v >= 60 ? `${Math.floor(v / 60)}m${v % 60 ? ` ${v % 60}s` : ""}` : `${v}s`)}
          />
          <span class="kbd-hint">Unload the model after this much idle time to free memory.</span>
        </div>
      {/if}
      <div class="slider-block">
        <Slider
          bind:value={controller.draftTranscriptionChunkSeconds}
          min={0}
          max={300}
          step={5}
          label="Chunk duration"
          formatValue={(v) => (v === 0 ? "off" : `${v}s`)}
        />
        <span class="kbd-hint">Chunking caps peak activation memory; 0 disables chunking.</span>
      </div>
    </AdvancedReveal>
  </div>
{/if}
