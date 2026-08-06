<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("speakers");

  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ModelFootprintHint from "$lib/settings/ui/ModelFootprintHint.svelte";
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

  // The separation row's own denominator (G8): what this preset weighs, read
  // from the model DTO — i.e. the corrected registry, where speakrs really is
  // 419 MB. Absent while the status is still loading; never hard-coded.
  const separationFootprint = $derived.by(() => {
    const size = selectedSpeakerModel?.download?.byteSize ?? null;
    if (!size) return null;
    return `${selectedSpeakerModel?.provider ?? "speakrs"} · on device · ${formatBytes(size)} of models, loaded on demand.`;
  });
</script>

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

  <!-- Two switches under one indexed row (page 11 draws it exactly this way):
       separation is anonymous, recognition is a SECOND opt-in gated on it. The
       marketing hero this replaced was the one card left in a shell whose whole
       argument is hairlines instead of card edges. -->
  <SettingRow
    label="Speaker separation"
    description={separationFootprint ??
      "Runs on device, after microphone transcription. Separation is anonymous; recognition is a second opt-in."}
    full
  >
    {#snippet control()}
      <div class="ss-grp speaker-switches">
        <div class="ss-mrow">
          <span class="ss-mrow__n speaker-switches__n">Separate speakers in transcripts</span>
          <span class="ss-mrow__meta">
            <Switch
              bind:checked={rec.draftSpeakerSeparateSpeakers}
              ariaLabel="Separate speakers in transcripts"
            />
          </span>
        </div>
        <div class="ss-mrow">
          <span class="ss-mrow__n speaker-switches__n">Recognize saved people</span>
          <span class="ss-mrow__meta">
            <Switch
              bind:checked={rec.draftSpeakerRecognizeSavedPeople}
              disabled={!rec.draftSpeakerSeparateSpeakers}
              ariaLabel="Recognize saved people"
            />
          </span>
        </div>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Helper timeout"
    description="Stops speaker analysis if the local helper runs too long. Existing queued jobs keep the timeout they were created with."
    full
  >
    {#snippet aside()}
      <span class="row-value">{rec.draftSpeakerTimeoutMinutes}m</span>
    {/snippet}
    {#snippet control()}
      <Slider
        bind:value={rec.draftSpeakerTimeoutMinutes}
        min={1}
        max={60}
        step={1}
        ariaLabel="Speaker helper timeout"
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
            <span
              class="model-status__pill"
              class:model-status__pill--ok={selectedSpeakerModel.available}
            >{selectedSpeakerModel.available ? "available" : "unavailable"}</span>
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
                  {#if cancellingSpeakerDownload}<ButtonSpinner />Cancelling{:else}Cancel download{/if}
                </button>
              </div>
            {:else}
              <div class="debug-log-actions">
                <button type="button" class="btn btn--primary" onclick={startSelectedSpeakerModelDownload} disabled={startingSpeakerDownload || selectedSpeakerModel.available} aria-busy={startingSpeakerDownload}>
                  {#if startingSpeakerDownload}<ButtonSpinner />Starting{:else}Download ({formatBytes(selectedSpeakerModel.download.byteSize)}){/if}
                </button>
                <button type="button" class="btn btn--danger" onclick={deleteSelectedSpeakerModel} disabled={deletingSpeakerModel || selectedSpeakerDownloadRunning || !selectedSpeakerModel.available} aria-busy={deletingSpeakerModel}>
                  {#if deletingSpeakerModel}<ButtonSpinner />Deleting{:else}Delete speaker model{/if}
                </button>
              </div>
            {/if}
            <p class="group-hint">Downloads this preset's segmentation and speaker-embedding models into app-managed storage.</p>
            <ModelFootprintHint byteSize={selectedSpeakerModel.download.byteSize} />
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

</style>
