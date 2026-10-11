<script lang="ts">
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Slider from "$lib/components/Slider.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelDownloadRow from "$lib/settings/ui/ModelDownloadRow.svelte";
  import { downloadMeta } from "$lib/settings/ui/model-download";
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
  const m = $derived(c.selectedSpeakerModel);
  const speakerModelOptions = $derived(c.speakerModelOptions);
  const selectedSpeakerPresetKey = $derived(c.selectedSpeakerPresetKey);
  const selectedSpeakerDownloadProgress = $derived(c.selectedSpeakerDownloadProgress);
  const selectedSpeakerDownloadRunning = $derived(c.selectedSpeakerDownloadRunning);

  // Controller action methods.
  const loadSpeakerModelStatus = () => c.loadSpeakerModelStatus();
  const chooseSpeakerModel = (value: string) => c.chooseSpeakerModel(value);
  const startSelectedSpeakerModelDownload = () => c.startSelectedSpeakerModelDownload();
  const cancelSelectedSpeakerModelDownload = () => c.cancelSelectedSpeakerModelDownload();
  const deleteSelectedSpeakerModel = () => c.deleteSelectedSpeakerModel();
</script>

<SettingGroup
  id="settings-section-speakers"
  title="Speakers"
  hint="Anonymous separation first; naming saved people only when you opt in."
>
  {#snippet actions()}
    <ReloadButton
      onclick={loadSpeakerModelStatus}
      busy={loadingSpeakerModelStatus}
      title="Refresh"
      label="Refresh speaker model status"
    />
  {/snippet}

  <SettingRow
    label="Separate speakers in transcripts"
    description="Queue local diarization after successful microphone transcription."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftSpeakerSeparateSpeakers} ariaLabel="Separate speakers in transcripts" />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Recognize saved people"
    description="Opt in to matching against confirmed local Person voice profiles."
    disabled={!rec.draftSpeakerSeparateSpeakers}
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftSpeakerRecognizeSavedPeople}
        disabled={!rec.draftSpeakerSeparateSpeakers}
        ariaLabel="Recognize saved people"
      />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Helper timeout"
    description="Stops speaker analysis if it runs too long. Queued jobs keep the timeout they were created with."
    disabled={!rec.draftSpeakerSeparateSpeakers}
  >
    {#snippet control()}
      <div class="spk-ctl">
        <Slider
          bind:value={rec.draftSpeakerTimeoutMinutes}
          min={1}
          max={60}
          step={1}
          unit="m"
          ariaLabel="Helper timeout"
          disabled={!rec.draftSpeakerSeparateSpeakers}
        />
        <span class="num spk-v">{rec.draftSpeakerTimeoutMinutes}m</span>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Preset"
    description="Pick by intent. Switching is reversible, but saved voices need a one-time re-tag under a new preset."
    disabled={!rec.draftSpeakerSeparateSpeakers}
  >
    {#snippet control()}
      <div class="spk-ctl">
        <Combobox
          value={selectedSpeakerPresetKey}
          onValueChange={chooseSpeakerModel}
          disabled={!rec.draftSpeakerSeparateSpeakers || switchingSpeakerModel}
          searchPlaceholder="Search presets…"
          options={speakerModelOptions.length > 0 ? speakerModelOptions : [
            { value: selectedSpeakerPresetKey, label: "Loading preset options" },
          ]}
        />
      </div>
    {/snippet}
  </SettingRow>

  {#if speakerModelError}
    <ModelDownloadRow label="Speaker model" error={`Couldn’t load speaker model status: ${speakerModelError}`} />
  {:else if !m}
    <ModelDownloadRow
      label="Speaker model"
      description={loadingSpeakerModelStatus ? "Checking installed speaker models…" : undefined}
      warning={loadingSpeakerModelStatus ? null : "No speaker model status is available."}
    />
  {:else if selectedSpeakerDownloadRunning}
    <ModelDownloadRow
      label="Speaker model"
      progress={downloadMeta("speaker model", selectedSpeakerDownloadProgress)}
      onCancel={cancelSelectedSpeakerModelDownload}
      cancelling={cancellingSpeakerDownload}
    />
  {:else}
    {@const failed = m.status === "failed" || !!speakerDownloadError}
    <ModelDownloadRow
      label="Speaker model"
      description={`${m.displayName} · ${speakerStatusLabel(m).toLowerCase()}${m.download ? ` · ${formatBytes(m.download.byteSize)}` : ""}`}
      status={m.available ? "installed" : null}
      chip={m.available ? null : failed ? { text: "failed", tone: "danger" } : { text: m.status === "incomplete" ? "incomplete" : "missing", tone: "warn" }}
      error={speakerDownloadError ? `Speaker model action failed: ${speakerDownloadError}` : m.failureMessage}
      controls={deleteModel}
      note={speakerModelDeleteMessage ? deleted : undefined}
      download={!m.available && m.download
        ? { bytes: m.download.byteSize, starting: startingSpeakerDownload, onclick: startSelectedSpeakerModelDownload }
        : null}
      info={m}
    />
  {/if}
</SettingGroup>

{#snippet deleted()}
  {#if speakerModelDeleteMessage}
    <span class="mx-inline" aria-live="polite">{speakerModelDeleteMessage}</span>
  {/if}
{/snippet}

{#snippet deleteModel()}
  {#if m?.available}
    <button
      type="button"
      class="mx-btn mx-btn--ghost"
      onclick={deleteSelectedSpeakerModel}
      disabled={deletingSpeakerModel}
      aria-busy={deletingSpeakerModel}
    >
      {#if deletingSpeakerModel}<ButtonSpinner />Deleting{:else}Delete{/if}
    </button>
  {/if}
{/snippet}

<style>
  /* Right-aligned controls keep a readable width (.st-select). */
  .spk-ctl {
    width: 240px;
  }

  /* The slider's value, as Capture's sensitivity rows show it. */
  .spk-ctl:has(.spk-v) {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }

  .spk-v {
    min-width: 34px;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    text-align: right;
  }
</style>
