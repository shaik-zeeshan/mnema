<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
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

<SettingGroup
  id="settings-section-speakers"
  title="Speaker analysis"
  hint="Anonymous diarization first; saved-person recognition only when you explicitly opt in."
>
  {#snippet actions()}
    <button class="btn btn--ghost btn--sm" onclick={loadSpeakerModelStatus} disabled={loadingSpeakerModelStatus}>
      {loadingSpeakerModelStatus ? "Checking" : "Refresh"}
    </button>
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
        <SelectMenu
          value={selectedSpeakerPresetKey}
          onValueChange={chooseSpeakerModel}
          disabled={!rec.draftSpeakerSeparateSpeakers || switchingSpeakerModel}
          label="Preset"
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
            <p class="group-hint"><strong>Install path:</strong> {selectedSpeakerModel.installPath}</p>
          {/if}
          {#if selectedSpeakerModel.missingFiles.length > 0}
            <p class="group-hint group-hint--warn"><strong>Missing files:</strong> {selectedSpeakerModel.missingFiles.join(", ")}</p>
          {/if}
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
                <button class="btn btn--ghost" onclick={cancelSelectedSpeakerModelDownload} disabled={cancellingSpeakerDownload}>
                  {cancellingSpeakerDownload ? "Cancelling" : "Cancel download"}
                </button>
              </div>
            {:else}
              <div class="debug-log-actions">
                <button class="btn btn--ghost" onclick={startSelectedSpeakerModelDownload} disabled={startingSpeakerDownload || selectedSpeakerModel.available}>
                  {startingSpeakerDownload ? "Starting" : `Download (${formatBytes(selectedSpeakerModel.download.byteSize)})`}
                </button>
                <button class="btn btn--danger" onclick={deleteSelectedSpeakerModel} disabled={deletingSpeakerModel || selectedSpeakerDownloadRunning || !selectedSpeakerModel.available}>
                  {deletingSpeakerModel ? "Deleting" : "Delete speaker model"}
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
</style>
