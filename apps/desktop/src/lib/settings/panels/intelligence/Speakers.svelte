<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelMissingFiles from "$lib/settings/ui/ModelMissingFiles.svelte";
  import { speakerStatusLabel } from "$lib/settings/state/models-format";
  import { formatBytes } from "$lib/settings/state/format";

  const c = getSettingsController();
  const rec = c.rec;
  const models = c.models;

  // Store-read aliases.
  const switchingSpeakerModel = $derived(models.switchingSpeakerModel);
  const loadingSpeakerModelStatus = $derived(models.loadingSpeakerModelStatus);
  const speakerModelError = $derived(models.speakerModelError);
  const startingSpeakerDownload = $derived(models.startingSpeakerDownload);
  const cancellingSpeakerDownload = $derived(models.cancellingSpeakerDownload);
  const speakerDownloadError = $derived(models.speakerDownloadError);
  const deletingSpeakerModel = $derived(models.deletingSpeakerModel);
  const speakerModelDeleteMessage = $derived(models.speakerModelDeleteMessage);

  // Controller derived selectors.
  const selectedSpeakerModel = $derived(c.selectedSpeakerModel);
  const speakerModelOptions = $derived(c.speakerModelOptions);
  const selectedSpeakerPresetKey = $derived(c.selectedSpeakerPresetKey);
  const selectedSpeakerDownloadProgress = $derived(c.selectedSpeakerDownloadProgress);
  const selectedSpeakerDownloadRunning = $derived(c.selectedSpeakerDownloadRunning);
  const selectedSpeakerDownloadPercent = $derived(c.selectedSpeakerDownloadPercent);

  // Controller action methods.
  const loadSpeakerModelStatus = () => c.loadSpeakerModelStatus();
  const chooseSpeakerModel = (value: string) => c.chooseSpeakerModel(value);
  const startSelectedSpeakerModelDownload = () => c.startSelectedSpeakerModelDownload();
  const cancelSelectedSpeakerModelDownload = () => c.cancelSelectedSpeakerModelDownload();
  const deleteSelectedSpeakerModel = () => c.deleteSelectedSpeakerModel();
</script>

{#snippet spinner()}
  <svg class="btn-spinner" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
  </svg>
{/snippet}

<SettingGroup
  id="settings-section-speakers"
  title="Speaker analysis"
  hint="Anonymous diarization first; saved-person recognition only when you explicitly opt in."
>
  {#snippet actions()}
    <ReloadButton
      onclick={loadSpeakerModelStatus}
      busy={loadingSpeakerModelStatus}
      title="Refresh"
      label="Refresh speaker model status"
    />
  {/snippet}

  <SettingRow label="Speaker separation" full>
    {#snippet control()}
      <div class="speaker-settings-hero">
        <div>
          <span class="group-label">Transcript speakers</span>
          <h3>Split the room before naming anyone.</h3>
          <p>Speaker separation runs locally after microphone transcription. Recognition uses only confirmed Person voice embeddings stored in this save directory.</p>
        </div>
        <div class="speaker-settings-hero__toggles">
          <Switch
            bind:checked={rec.draftSpeakerSeparateSpeakers}
            label="Separate speakers in transcripts"
            description="Queue local diarization after successful microphone transcription"
          />
          <Switch
            bind:checked={rec.draftSpeakerRecognizeSavedPeople}
            disabled={!rec.draftSpeakerSeparateSpeakers}
            label="Recognize saved people"
            description="Opt in to matching against confirmed local Person voice profiles"
          />
        </div>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Helper timeout"
    description="Stops speaker analysis if the local helper runs too long. Existing queued jobs keep the timeout they were created with."
    full
  >
    {#snippet control()}
      <Slider
        bind:value={rec.draftSpeakerTimeoutMinutes}
        min={1}
        max={60}
        step={1}
        label="Timeout"
        unit="m"
        disabled={!rec.draftSpeakerSeparateSpeakers}
      />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Speaker model"
    description="Pick a preset by intent. Each preset's download size is shown in the list. Recognition is scoped per preset: switching is safe and reversible, but saved voices need a one-time re-tag under the new preset."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="speaker-stack">
        <Combobox
          value={selectedSpeakerPresetKey}
          onValueChange={chooseSpeakerModel}
          disabled={!rec.draftSpeakerSeparateSpeakers || switchingSpeakerModel}
          label="Preset"
          searchPlaceholder="Search presets…"
          options={speakerModelOptions.length > 0 ? speakerModelOptions : [
            { value: selectedSpeakerPresetKey, label: "Loading preset options" },
          ]}
        />
        {#if speakerModelError}
          <p class="group-hint group-hint--warn">Failed to load speaker model status: {speakerModelError}</p>
        {:else if selectedSpeakerModel}
          <div class="model-status" class:model-status--available={selectedSpeakerModel.available}>
            <div>
              <div class="model-status__title">{selectedSpeakerModel.displayName}</div>
              <div class="model-status__meta">{speakerStatusLabel(selectedSpeakerModel)}</div>
            </div>
            <span class="model-status__pill">{selectedSpeakerModel.available ? "available" : "unavailable"}</span>
          </div>
          <p class="group-hint">{selectedSpeakerModel.description}</p>
          {#if selectedSpeakerModel.installPath}
            <p class="group-hint"><strong>Install path:</strong> <span class="model-path">{selectedSpeakerModel.installPath}</span></p>
          {/if}
          <ModelMissingFiles files={selectedSpeakerModel.missingFiles} />
          {#if selectedSpeakerModel.failureMessage}
            <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedSpeakerModel.failureMessage}</p>
          {/if}
          {#if selectedSpeakerModel.licenseLabel || selectedSpeakerModel.sourceUrl}
            <p class="group-hint">
              {#if selectedSpeakerModel.licenseLabel}<strong>License:</strong> {selectedSpeakerModel.licenseLabel}. {/if}
              {#if selectedSpeakerModel.sourceUrl}<strong>Source:</strong> {selectedSpeakerModel.sourceUrl}{/if}
            </p>
          {/if}
          {#if selectedSpeakerModel.download}
            {#if selectedSpeakerDownloadRunning}
              <div class="download-progress" aria-live="polite">
                <div class="download-progress__bar">
                  <span style={`width: ${selectedSpeakerDownloadPercent ?? 8}%`}></span>
                </div>
                <p class="group-hint">
                  {selectedSpeakerDownloadProgress?.status ?? "downloading"}
                  {#if selectedSpeakerDownloadPercent !== null} · {selectedSpeakerDownloadPercent}%{/if}
                  {#if selectedSpeakerDownloadProgress?.message} · {selectedSpeakerDownloadProgress.message}{/if}
                </p>
                <button type="button" class="btn btn--ghost" onclick={cancelSelectedSpeakerModelDownload} disabled={cancellingSpeakerDownload} aria-busy={cancellingSpeakerDownload}>
                  {#if cancellingSpeakerDownload}{@render spinner()}Cancelling{:else}Cancel download{/if}
                </button>
              </div>
            {:else}
              <div class="debug-log-actions">
                <button type="button" class="btn btn--ghost" onclick={startSelectedSpeakerModelDownload} disabled={startingSpeakerDownload || selectedSpeakerModel.available} aria-busy={startingSpeakerDownload}>
                  {#if startingSpeakerDownload}{@render spinner()}Starting{:else}Download ({formatBytes(selectedSpeakerModel.download.byteSize)}){/if}
                </button>
                <button type="button" class="btn btn--danger" onclick={deleteSelectedSpeakerModel} disabled={deletingSpeakerModel || selectedSpeakerDownloadRunning || !selectedSpeakerModel.available} aria-busy={deletingSpeakerModel}>
                  {#if deletingSpeakerModel}{@render spinner()}Deleting{:else}Delete speaker model{/if}
                </button>
              </div>
            {/if}
            <p class="group-hint">Downloads this preset's segmentation and speaker-embedding models into app-managed storage.</p>
          {/if}
          {#if speakerDownloadError}
            <p class="group-hint group-hint--warn">Speaker model action failed: {speakerDownloadError}</p>
          {/if}
          {#if speakerModelDeleteMessage}
            <p class="group-hint">{speakerModelDeleteMessage}</p>
          {/if}
        {:else if loadingSpeakerModelStatus}
          <p class="group-hint">Checking installed speaker models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No speaker model status is available.</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* The model row stacks the preset picker over the bordered model-status /
     download sub-block; primitives only gap whole rows. */
  .speaker-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  /* Render filesystem paths in mono so they read as machine values, matching
     the Developer log path treatment. */
  .model-path {
    font-family: var(--app-font-mono);
    word-break: break-all;
  }

  /* Inline busy spinner shown beside a button label while an action is in
     flight; reuses the shared settings-icon-spin keyframe. */
  .btn-spinner {
    width: 13px;
    height: 13px;
    margin-right: 6px;
    vertical-align: -2px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    animation: settings-icon-spin 0.7s linear infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .btn-spinner {
      animation: none;
    }
  }
</style>
