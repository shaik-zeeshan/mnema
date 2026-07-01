<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import type { AudioTranscriptionMemoryMode } from "$lib/types";
  import Segmented from "$lib/components/Segmented.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import AdvancedReveal from "./AdvancedReveal.svelte";
  import { formatBytes } from "./onboarding-mapping";
  import { TRANSCRIPTION_LANGUAGE_OPTIONS } from "$lib/settings/transcription-languages";

  let { controller }: { controller: OnboardingController } = $props();

  // Mirrors Transcription.svelte: the Combobox selects the model; the card
  // reflects the resolved `selectedTranscriptionModel` and drives the in-app
  // download. The status pill reuses the mockup's `.pill` family.
  const model = $derived(controller.selectedTranscriptionModel);

  // Provider picker is a Segmented control (matching MicBody's adapter picker):
  // a small static enum of providers. Options + per-provider availability come
  // from the live model status (mirrors controller-processing's option deriving);
  // fall back to static labels before status arrives. Segmented can't render
  // per-option descriptions, so the availability subtitle that RadioGroup showed
  // inline collapses to one contextual hint line for the active provider.
  // Providers are NOT disabled when their model is unavailable: selecting a
  // provider is how you reach the Model section below to download its model, so
  // disabling an unavailable provider would make its model undownloadable.
  const transcriptionProviderOptions = $derived(
    (controller.transcriptionModelStatus?.providers ?? []).length > 0
      ? (controller.transcriptionModelStatus?.providers ?? []).map((provider) => ({
          value: provider.provider,
          label: provider.displayName,
        }))
      : [
          { value: "local_whisper", label: "Local Whisper" },
          { value: "apple_speech_on_device", label: "Apple Speech (on-device)" },
          { value: "parakeet", label: "Parakeet" },
        ],
  );

  // Contextual availability hint for the active provider (replaces RadioGroup's
  // per-option subtitle).
  const transcriptionProviderHint = $derived.by(() => {
    const providers = controller.transcriptionModelStatus?.providers ?? [];
    if (providers.length === 0) return "Model status is loading.";
    const active = providers.find((p) => p.provider === controller.draftTranscriptionProvider);
    if (!active) return "No available model detected for the selected provider.";
    return active.models.some((m) => m.available)
      ? "At least one model is available."
      : "No available model detected for this provider.";
  });

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
      <div class="slider-block">
        <Segmented
          value={controller.draftTranscriptionProvider}
          onValueChange={(v) => controller.chooseTranscriptionProvider(v)}
          ariaLabel="Transcription provider"
          options={transcriptionProviderOptions}
        />
        <span class="kbd-hint">{transcriptionProviderHint}</span>
      </div>
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
    <div class="note">
      <div>Failed to load model status: {controller.transcriptionModelError}</div>
      <div class="dl-meta">
        <span>This is a fetch error, not a missing model — retry to recheck.</span>
        <button
          type="button"
          class="btn sm"
          disabled={controller.loadingTranscriptionModelStatus}
          onclick={() => controller.loadTranscriptionModelStatus()}
        >
          {controller.loadingTranscriptionModelStatus ? "Retrying…" : "Retry"}
        </button>
      </div>
    </div>
  {:else if model}
    <div class="model-card">
      <div class="model-top">
        <div class="model-id">
          {model.displayName}
          <!-- Status lives in the pill only (mirrors SemanticSearchBody); the meta
               line carries the model description, not a second copy of the status. -->
          {#if model.description}
            <div class="meta">{model.description}</div>
          {/if}
        </div>
        <span class="pill {pillClass}">
          <span class="d"></span>{controller.transcriptionStatusLabel(model)}
        </span>
      </div>

      {#if model.management === "app_managed"}
        {#if controller.selectedTranscriptionDownloadRunning}
          <div class="dl">
            <div class="dl-track">
              <div
                class="dl-fill"
                class:dl-fill--indeterminate={controller.selectedTranscriptionDownloadPercent === null}
                style={controller.selectedTranscriptionDownloadPercent === null
                  ? undefined
                  : `width: ${controller.selectedTranscriptionDownloadPercent}%`}
              ></div>
            </div>
            <div class="dl-meta">
              <span>
                <b>{controller.selectedTranscriptionDownloadPercent === null
                    ? "…"
                    : `${controller.selectedTranscriptionDownloadPercent}%`}</b>
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
      <!-- Stepper (not Slider) here mirrors Settings' Transcription.svelte so the
           same value snaps identically across onboarding and Settings (matching
           min/max/step). -->
      {#if controller.draftTranscriptionMemoryMode === "balanced"}
        <div class="ctl stack-field">
          <div class="ctl-label">
            <div class="name">Idle unload</div>
          </div>
          <div class="ctl-field">
            <Stepper
              id="onboarding-transcription-idle-unload"
              bind:value={
                () => String(controller.draftTranscriptionIdleUnloadSeconds),
                (v) => { controller.draftTranscriptionIdleUnloadSeconds = parseInt(v, 10) || 0; }
              }
              min={0}
              max={1800}
              step={30}
              unit="s"
              ariaLabel="idle unload seconds"
            />
            <span class="kbd-hint">Unload the model after this much idle time to free memory.</span>
          </div>
        </div>
      {/if}
      <div class="ctl stack-field">
        <div class="ctl-label">
          <div class="name">Chunk duration</div>
        </div>
        <div class="ctl-field">
          <Stepper
            id="onboarding-transcription-chunk-seconds"
            bind:value={
              () => String(controller.draftTranscriptionChunkSeconds),
              (v) => { controller.draftTranscriptionChunkSeconds = parseInt(v, 10) || 0; }
            }
            min={0}
            max={300}
            step={15}
            unit="s"
            ariaLabel="chunk seconds"
          />
          <span class="kbd-hint">Chunking caps peak activation memory; 0 disables chunking.</span>
        </div>
      </div>
    </AdvancedReveal>
  </div>
{/if}
