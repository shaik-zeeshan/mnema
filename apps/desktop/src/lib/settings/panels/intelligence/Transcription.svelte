<script lang="ts">
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelMissingFiles from "$lib/settings/ui/ModelMissingFiles.svelte";
  import {
    transcriptionStatusLabel,
    appleSpeechPermissionLabel,
    appleSpeechPermissionHint,
  } from "$lib/settings/state/models-format";
  import { formatBytes } from "$lib/settings/state/format";
  import { TRANSCRIPTION_LANGUAGE_OPTIONS } from "$lib/settings/transcription-languages";
  import type { AudioTranscriptionMemoryMode } from "$lib/types";

  const c = getSettingsController();
  const rec = c.rec;
  const models = c.models;

  // Near-the-control autosave cue: transcription drafts persist through the
  // "processing" recording domain. Mirror its saving/just-saved state beside the
  // toggle rows so the edit is confirmed at the point of interaction — matching
  // Capture/Audio/Video/Storage — not only at the remote rail footer.
  const processingSaving = $derived(c.rec.savingRecDomains.processing);
  const processingSaved = $derived(c.recSavedDomain === "processing");

  // Store-read aliases.
  const loadingTranscriptionModelStatus = $derived(models.loadingTranscriptionModelStatus);
  const transcriptionModelError = $derived(models.transcriptionModelError);
  const startingTranscriptionDownload = $derived(models.startingTranscriptionDownload);
  const cancellingTranscriptionDownload = $derived(models.cancellingTranscriptionDownload);
  const transcriptionDownloadError = $derived(models.transcriptionDownloadError);
  const deletingUnusedTranscriptionModels = $derived(models.deletingUnusedTranscriptionModels);
  const deleteUnusedTranscriptionModelsMessage = $derived(models.deleteUnusedTranscriptionModelsMessage);
  const deletedUnusedTranscriptionModelLabels = $derived(models.deletedUnusedTranscriptionModelLabels);
  const skippedUnusedTranscriptionModelLabels = $derived(models.skippedUnusedTranscriptionModelLabels);
  const skippedTranscriptionProcessingJobModelLabels = $derived(models.skippedTranscriptionProcessingJobModelLabels);
  const deleteUnusedTranscriptionModelsError = $derived(models.deleteUnusedTranscriptionModelsError);
  const requestingAppleSpeechPermission = $derived(models.requestingAppleSpeechPermission);
  const appleSpeechPermissionError = $derived(models.appleSpeechPermissionError);

  // Controller derived selectors.
  const transcriptionProviderOptions = $derived(c.transcriptionProviderOptions);
  const transcriptionModelOptions = $derived(c.transcriptionModelOptions);
  const selectedTranscriptionModel = $derived(c.selectedTranscriptionModel);
  const selectedAppleSpeechPermissionStatus = $derived(c.selectedAppleSpeechPermissionStatus);
  const selectedAppleSpeechNeedsPermission = $derived(c.selectedAppleSpeechNeedsPermission);
  const selectedTranscriptionDownloadProgress = $derived(c.selectedTranscriptionDownloadProgress);
  const selectedTranscriptionDownloadRunning = $derived(c.selectedTranscriptionDownloadRunning);
  const selectedTranscriptionDownloadPercent = $derived(c.selectedTranscriptionDownloadPercent);

  // Controller / store action methods.
  const loadTranscriptionModelStatus = () => c.loadTranscriptionModelStatus();
  const chooseTranscriptionProvider = (provider: string) => c.chooseTranscriptionProvider(provider);
  const chooseTranscriptionModel = (value: string) => c.chooseTranscriptionModel(value);
  const requestAppleSpeechPermission = () => c.requestAppleSpeechPermission();
  const openAppleSpeechPrivacySettings = () => c.openAppleSpeechPrivacySettings();
  const startSelectedTranscriptionModelDownload = () => c.startSelectedTranscriptionModelDownload();
  const cancelSelectedTranscriptionModelDownload = () => c.cancelSelectedTranscriptionModelDownload();
  const requestDeleteUnusedTranscriptionModels = () => c.requestDeleteUnusedTranscriptionModels();
</script>

<SettingGroup
  id="settings-section-transcription"
  title="Transcription"
  hint="Local speech-to-text provider and model setup for microphone audio."
>
  {#snippet actions()}
    <ReloadButton
      onclick={loadTranscriptionModelStatus}
      busy={loadingTranscriptionModelStatus}
      title="Refresh"
      label="Refresh transcription model status"
    />
  {/snippet}

  <SettingRow
    label="Enable audio transcription"
    description="Master switch for source-specific audio transcription"
    saving={processingSaving}
    saved={processingSaved}
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftTranscriptionEnabled} ariaLabel="Enable audio transcription" />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Transcribe microphone"
    description="Automatically queue transcription for committed microphone audio segments"
    disabled={!rec.draftTranscriptionEnabled}
    saving={processingSaving}
    saved={processingSaved}
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftTranscriptionMicrophoneEnabled}
        disabled={!rec.draftTranscriptionEnabled}
        ariaLabel="Transcribe microphone"
      />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Transcribe system audio"
    description="Transcribe system audio only when speech is detected."
    disabled={!rec.draftTranscriptionEnabled}
    saving={processingSaving}
    saved={processingSaved}
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftTranscriptionSystemAudioEnabled}
        disabled={!rec.draftTranscriptionEnabled}
        ariaLabel="Transcribe system audio"
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Provider" full saving={processingSaving} saved={processingSaved}>
    {#snippet control()}
      <RadioGroup
        value={rec.draftTranscriptionProvider}
        onValueChange={chooseTranscriptionProvider}
        options={transcriptionProviderOptions.length > 0 ? transcriptionProviderOptions : [
          { value: "local_whisper", label: "Local Whisper", description: "Model status is loading" },
          { value: "apple_speech_on_device", label: "Apple Speech (on-device)", description: "Model status is loading" },
          { value: "parakeet", label: "Parakeet", description: "Model status is loading" },
        ]}
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Model" full saving={processingSaving} saved={processingSaved}>
    {#snippet control()}
      <Combobox
        value={rec.draftTranscriptionModelId ?? "__os_managed__"}
        onValueChange={chooseTranscriptionModel}
        searchPlaceholder="Search models…"
        options={transcriptionModelOptions.length > 0 ? transcriptionModelOptions : [
          { value: rec.draftTranscriptionModelId ?? "__os_managed__", label: "Loading model options" },
        ]}
      />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Language"
    description="Use auto for automatic language detection, or enter a language hint such as en. Settings changes affect future audio segments; already-queued jobs keep their admitted provider/model payload."
    full
    saving={processingSaving}
    saved={processingSaved}
  >
    {#snippet control()}
      <Combobox
        value={rec.draftTranscriptionLanguage || "auto"}
        onValueChange={(v) => (rec.draftTranscriptionLanguage = v)}
        options={TRANSCRIPTION_LANGUAGE_OPTIONS}
        searchPlaceholder="Search languages…"
      />
    {/snippet}
  </SettingRow>

  {#if rec.draftTranscriptionProvider === "parakeet"}
    <SettingRow label="Parakeet memory mode" full>
      {#snippet control()}
        <Segmented
          value={rec.draftTranscriptionMemoryMode}
          onValueChange={(value) => rec.draftTranscriptionMemoryMode = value as AudioTranscriptionMemoryMode}
          ariaLabel="Parakeet memory mode"
          options={[
            { value: "balanced", label: "Balanced" },
            { value: "low_memory", label: "Low memory" },
            { value: "performance", label: "Performance" },
          ]}
        />
      {/snippet}
    </SettingRow>
    {#if rec.draftTranscriptionMemoryMode === "balanced"}
      <SettingRow label="Idle unload seconds" full>
        {#snippet control()}
          <Stepper
            id="transcription-idle-unload"
            bind:value={
              () => String(rec.draftTranscriptionIdleUnloadSeconds),
              (v) => { rec.draftTranscriptionIdleUnloadSeconds = parseInt(v, 10) || 0; }
            }
            min={0}
            max={1800}
            step={30}
            unit="s"
            ariaLabel="idle unload seconds"
          />
        {/snippet}
      </SettingRow>
    {/if}
    <SettingRow
      label="Chunk seconds"
      description="Choose the int8 Parakeet model for lower disk and runtime weight memory. Chunking limits peak activation memory; set chunk seconds to 0 to disable chunking."
      full
    >
      {#snippet control()}
        <Stepper
          id="transcription-chunk-seconds"
          bind:value={
            () => String(rec.draftTranscriptionChunkSeconds),
            (v) => { rec.draftTranscriptionChunkSeconds = parseInt(v, 10) || 0; }
          }
          min={0}
          max={300}
          step={15}
          unit="s"
          ariaLabel="chunk seconds"
        />
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow label="Selected model status" full divider={false}>
    {#snippet control()}
      <div class="tx-stack">
        {#if transcriptionModelError}
          <p class="group-hint group-hint--warn" role="alert">Failed to load model status: {transcriptionModelError}</p>
        {:else if selectedTranscriptionModel}
          <div class="model-status" class:model-status--available={selectedTranscriptionModel.available}>
            <div>
              <div class="model-status__title">{selectedTranscriptionModel.displayName}</div>
              <div class="model-status__meta">{transcriptionStatusLabel(selectedTranscriptionModel)}</div>
            </div>
            <span
              class="model-status__pill"
              class:model-status__pill--ok={selectedTranscriptionModel.available}
            >{selectedTranscriptionModel.available ? "available" : "unavailable"}</span>
          </div>
          <p class="group-hint">{selectedTranscriptionModel.description}</p>
          {#if selectedAppleSpeechPermissionStatus}
            <div class="permission-callout" class:permission-callout--ok={selectedAppleSpeechPermissionStatus === "available"}>
              <div class="permission-callout__copy">
                <span class="permission-callout__eyebrow">Apple Speech status</span>
                <strong>{appleSpeechPermissionLabel(selectedAppleSpeechPermissionStatus)}</strong>
                <p>{appleSpeechPermissionHint(selectedAppleSpeechPermissionStatus)}</p>
              </div>
              {#if selectedAppleSpeechNeedsPermission}
                {#if selectedAppleSpeechPermissionStatus === "permission_not_determined"}
                  <button
                    type="button"
                    class="btn btn--ghost"
                    onclick={requestAppleSpeechPermission}
                    disabled={requestingAppleSpeechPermission}
                    aria-busy={requestingAppleSpeechPermission}
                  >
                    {#if requestingAppleSpeechPermission}<ButtonSpinner />Requesting{:else}Get permission{/if}
                  </button>
                {:else}
                  <button type="button" class="btn btn--ghost" onclick={openAppleSpeechPrivacySettings}>
                    Open System Settings
                  </button>
                {/if}
              {/if}
            </div>
            {#if appleSpeechPermissionError}
              <p class="group-hint group-hint--warn" role="alert">Permission request failed: {appleSpeechPermissionError}</p>
            {/if}
          {/if}
          {#if selectedTranscriptionModel.installPath}
            <p class="group-hint"><strong>Install path:</strong> <span class="model-path">{selectedTranscriptionModel.installPath}</span></p>
          {/if}
          <ModelMissingFiles files={selectedTranscriptionModel.missingFiles} />
          {#if selectedTranscriptionModel.failureMessage}
            <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedTranscriptionModel.failureMessage}</p>
          {/if}
          {#if selectedTranscriptionModel.licenseLabel || selectedTranscriptionModel.sourceUrl}
            <p class="group-hint">
              {#if selectedTranscriptionModel.licenseLabel}<strong>License:</strong> {selectedTranscriptionModel.licenseLabel}. {/if}
              {#if selectedTranscriptionModel.sourceUrl}<strong>Source:</strong> {selectedTranscriptionModel.sourceUrl}{/if}
            </p>
          {/if}
          {#if selectedTranscriptionModel.management === "app_managed"}
            {#if selectedTranscriptionModel.download}
              {#if selectedTranscriptionDownloadRunning}
                <div class="download-progress" aria-live="polite">
                  <div class="download-progress__bar">
                    <span style={`width: ${selectedTranscriptionDownloadPercent ?? 8}%`}></span>
                  </div>
                  <p class="group-hint">
                    {selectedTranscriptionDownloadProgress?.status ?? "downloading"}
                    {#if selectedTranscriptionDownloadPercent !== null} · {selectedTranscriptionDownloadPercent}%{/if}
                    {#if selectedTranscriptionDownloadProgress?.message} · {selectedTranscriptionDownloadProgress.message}{/if}
                  </p>
                  <button type="button" class="btn btn--ghost" onclick={cancelSelectedTranscriptionModelDownload} disabled={cancellingTranscriptionDownload} aria-busy={cancellingTranscriptionDownload}>
                    {#if cancellingTranscriptionDownload}<ButtonSpinner />Cancelling{:else}Cancel download{/if}
                  </button>
                </div>
              {:else}
                <button type="button" class="btn btn--ghost" onclick={startSelectedTranscriptionModelDownload} disabled={startingTranscriptionDownload || selectedTranscriptionModel.available} aria-busy={startingTranscriptionDownload}>
                  {#if startingTranscriptionDownload}<ButtonSpinner />Starting{:else}Download ({formatBytes(selectedTranscriptionModel.download.byteSize)}){/if}
                </button>
              {/if}
              <p class="group-hint">Download support validates sha256 before marking this model installed.</p>
            {:else if !selectedTranscriptionModel.available}
              <p class="group-hint group-hint--warn">
                This app-managed model is missing, but no packaged download artifact is available in the current manifest.
              </p>
            {/if}
            {#if transcriptionDownloadError}
              <p class="group-hint group-hint--warn" role="alert">Download failed: {transcriptionDownloadError}</p>
            {/if}
          {:else}
            <p class="group-hint">This provider is managed by macOS. There is no app-managed model download.</p>
          {/if}
          <div class="debug-log-actions">
            <button type="button" class="btn btn--danger" onclick={requestDeleteUnusedTranscriptionModels} disabled={deletingUnusedTranscriptionModels || selectedTranscriptionDownloadRunning} aria-busy={deletingUnusedTranscriptionModels}>
              {#if deletingUnusedTranscriptionModels}<ButtonSpinner />Deleting…{:else}Delete unused transcription models{/if}
            </button>
          </div>
          <p class="group-hint">Removes app-managed transcription model files except the model selected above.</p>
          {#if deleteUnusedTranscriptionModelsMessage}
            <div class="cleanup-result" aria-live="polite">
              <strong>{deleteUnusedTranscriptionModelsMessage}</strong>
              {#if deletedUnusedTranscriptionModelLabels.length > 0}
                <p>Deleted:</p>
                <ul>
                  {#each deletedUnusedTranscriptionModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedUnusedTranscriptionModelLabels.length > 0}
                <p>Skipped active downloads:</p>
                <ul>
                  {#each skippedUnusedTranscriptionModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedTranscriptionProcessingJobModelLabels.length > 0}
                <p>Skipped running jobs:</p>
                <ul>
                  {#each skippedTranscriptionProcessingJobModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
          {#if deleteUnusedTranscriptionModelsError}
            <p class="group-hint group-hint--warn" role="alert">Delete failed: {deleteUnusedTranscriptionModelsError}</p>
          {/if}
        {:else if loadingTranscriptionModelStatus}
          <p class="group-hint">Checking installed transcription models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No model status is available for the selected provider.</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* The status row stacks the bordered model-status, permission, download, and
     cleanup sub-blocks; the primitives only gap whole rows. */
  .tx-stack {
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
