<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Choice from "$lib/components/Choice.svelte";
  import IconChip from "~icons/lucide/cpu";
  import IconCloud from "~icons/lucide/cloud";
  import IconSpeech from "~icons/lucide/audio-lines";
  import IconLock from "~icons/lucide/lock";
  import Segmented from "$lib/components/Segmented.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelDownloadRow from "$lib/settings/ui/ModelDownloadRow.svelte";
  import { downloadMeta } from "$lib/settings/ui/model-download";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";
  import IconExternal from "~icons/lucide/external-link";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import {
    transcriptionStatusLabel,
    appleSpeechPermissionLabel,
    appleSpeechPermissionHint,
  } from "$lib/settings/state/models-format";
  import { TRANSCRIPTION_LANGUAGE_OPTIONS } from "$lib/settings/transcription-languages";
  import type { AudioTranscriptionMemoryMode } from "$lib/types";

  const c = getSettingsController();
  const rec = c.rec;
  const models = c.models;

  // Store-read aliases.
  const loadingTranscriptionModelStatus = $derived(models.loadingTranscriptionModelStatus);
  const transcriptionModelError = $derived(models.transcriptionModelError);
  const startingTranscriptionDownload = $derived(models.startingTranscriptionDownload);
  const cancellingTranscriptionDownload = $derived(models.cancellingTranscriptionDownload);
  const transcriptionDownloadError = $derived(models.transcriptionDownloadError);
  const deletingUnusedTranscriptionModels = $derived(models.deletingUnusedTranscriptionModels);
  const deleteUnusedTranscriptionModelsMessage = $derived(models.deleteUnusedTranscriptionModelsMessage);
  const deleteUnusedTranscriptionModelsError = $derived(models.deleteUnusedTranscriptionModelsError);
  const requestingAppleSpeechPermission = $derived(models.requestingAppleSpeechPermission);
  const appleSpeechPermissionError = $derived(models.appleSpeechPermissionError);

  // Controller derived selectors.
  const transcriptionProviderOptions = $derived(c.transcriptionProviderOptions);
  const transcriptionModelOptions = $derived(c.transcriptionModelOptions);
  const m = $derived(c.selectedTranscriptionModel);
  const selectedAppleSpeechPermissionStatus = $derived(c.selectedAppleSpeechPermissionStatus);
  const selectedAppleSpeechNeedsPermission = $derived(c.selectedAppleSpeechNeedsPermission);
  const selectedTranscriptionDownloadProgress = $derived(c.selectedTranscriptionDownloadProgress);
  const selectedTranscriptionDownloadRunning = $derived(c.selectedTranscriptionDownloadRunning);

  // Controller / store action methods.
  const loadTranscriptionModelStatus = () => c.loadTranscriptionModelStatus();
  const chooseTranscriptionProvider = (provider: string) => c.chooseTranscriptionProvider(provider);

  // Kit `mx-choice--list` rows: what each provider IS (icon, where it runs,
  // one line of copy). Model availability lives on the Model row below.
  const PROVIDER_META: Record<string, { icon: typeof IconChip; tag: string; description: string }> = {
    parakeet: { icon: IconChip, tag: "on-device", description: "Fast, English-first transcription on the Neural Engine." },
    local_whisper: { icon: IconChip, tag: "on-device", description: "Many languages; slower and heavier on memory." },
    apple_speech_on_device: { icon: IconSpeech, tag: "on-device", description: "Built into macOS; needs Speech Recognition permission." },
    deepgram: { icon: IconCloud, tag: "cloud", description: "Uploads mic and system audio to your own Deepgram account. Needs an API key and your consent." },
  };
  const providerChoices = $derived(
    (transcriptionProviderOptions.length > 0
      ? transcriptionProviderOptions
      : [
          { value: "parakeet", label: "Parakeet" },
          { value: "local_whisper", label: "Local Whisper" },
          { value: "apple_speech_on_device", label: "Apple Speech" },
          { value: "deepgram", label: "Deepgram" },
        ]
    ).map((o) => ({ value: o.value, label: o.label, ...PROVIDER_META[o.value] })),
  );
  const chooseTranscriptionModel = (value: string) => c.chooseTranscriptionModel(value);
  const requestAppleSpeechPermission = () => c.requestAppleSpeechPermission();
  const openAppleSpeechPrivacySettings = () => c.openAppleSpeechPrivacySettings();
  const startSelectedTranscriptionModelDownload = () => c.startSelectedTranscriptionModelDownload();
  const cancelSelectedTranscriptionModelDownload = () => c.cancelSelectedTranscriptionModelDownload();
  const requestDeleteUnusedTranscriptionModels = () => c.requestDeleteUnusedTranscriptionModels();
  const deleteUnusedDesc = "Removes app-managed transcription model files except the selected one.";

  // ─── Deepgram API-key management (panel-local; direct invoke, no controller) ──
  let deepgramKeyInput = $state("");
  let deepgramKeyPresent = $state(false);
  let deepgramAuthStatus = $state<string | null>(null);
  let deepgramSaving = $state(false);
  let deepgramSaveError = $state<string | null>(null);
  let deepgramChecking = $state(false);
  let deepgramCheckResult = $state<{ ok: boolean; message: string } | null>(null);

  async function loadDeepgramKeyState() {
    try {
      deepgramKeyPresent = await invoke<boolean>("transcription_has_deepgram_key");
      deepgramAuthStatus = await invoke<string | null>("transcription_deepgram_auth_status");
    } catch (e) {
      // Denied ≠ missing (ADR 0048 amendment): a keychain/vault read error must not
      // render as "no key saved" — surface it on the existing warn line instead.
      deepgramAuthStatus = String(e);
    }
  }

  // Validate the key against Deepgram (GET /v1/auth/token — no audio). Mirrors the AI runtime's
  // connection test; also refreshes the auth-status line, which the probe may have set.
  async function checkDeepgram() {
    deepgramChecking = true;
    deepgramCheckResult = null;
    try {
      deepgramCheckResult = await invoke<{ ok: boolean; message: string }>("transcription_test_deepgram");
    } catch (e) {
      deepgramCheckResult = { ok: false, message: String(e) };
    } finally {
      deepgramChecking = false;
      await loadDeepgramKeyState();
    }
  }

  async function saveDeepgramKey() {
    deepgramSaving = true;
    deepgramSaveError = null;
    deepgramCheckResult = null;
    try {
      await invoke("transcription_set_deepgram_key", { key: deepgramKeyInput });
      deepgramKeyInput = "";
      await loadDeepgramKeyState();
      // Deepgram availability = key presence, so refresh the model-status pill now that it changed.
      await loadTranscriptionModelStatus();
      // Evaluate availability right away so a bad key/model surfaces on save, not on the first job.
      await checkDeepgram();
    } catch (e) {
      deepgramSaveError = String(e);
    } finally {
      deepgramSaving = false;
    }
  }

  async function removeDeepgramKey() {
    deepgramSaveError = null;
    deepgramCheckResult = null;
    try {
      await invoke("transcription_clear_deepgram_key");
      await loadDeepgramKeyState();
      // Key gone → Deepgram is now unavailable; refresh the model-status pill to reflect it.
      await loadTranscriptionModelStatus();
    } catch (e) {
      deepgramSaveError = String(e);
    }
  }

  // Load presence + auth status once each time the provider becomes deepgram.
  // `deepgramLoaded` is a plain (non-reactive) let, so the effect only depends
  // on `draftTranscriptionProvider` — it never re-runs from the $state that
  // loadDeepgramKeyState() writes (which would otherwise loop).
  let deepgramLoaded = false;
  $effect(() => {
    if (rec.draftTranscriptionProvider === "deepgram") {
      if (!deepgramLoaded) {
        deepgramLoaded = true;
        // Start each visit with a clean action-result slate — these transient messages only render
        // inside the deepgram block, so without this they'd re-appear stale on a later re-entry.
        deepgramCheckResult = null;
        deepgramSaveError = null;
        void loadDeepgramKeyState();
      }
    } else {
      deepgramLoaded = false;
    }
  });
</script>

<SettingGroup
  id="settings-section-transcription"
  title="Transcription"
  hint="Speech-to-text for microphone and system audio."
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
    description="Master switch for source-specific audio transcription."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftTranscriptionEnabled} ariaLabel="Enable audio transcription" />
    {/snippet}
  </SettingRow>

  <SettingRow label="Transcribe microphone" source="mic" disabled={!rec.draftTranscriptionEnabled}>
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
    source="sysaudio"
    description="Transcribe system audio only when speech is detected."
    disabled={!rec.draftTranscriptionEnabled}
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftTranscriptionSystemAudioEnabled}
        disabled={!rec.draftTranscriptionEnabled}
        ariaLabel="Transcribe system audio"
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Provider" description="Switching to Deepgram asks for consent first — it uploads mic and system audio." full>
    {#snippet control()}
      <Choice
        list
        value={rec.draftTranscriptionProvider}
        onValueChange={chooseTranscriptionProvider}
        ariaLabel="Transcription provider"
        options={providerChoices}
      />
    {/snippet}
  </SettingRow>

  {#if rec.draftTranscriptionProvider === "deepgram"}
    <SettingRow
      label="Deepgram API key"
      description={deepgramKeyPresent ? "Key saved to the macOS keychain." : "Paste the API key from your Deepgram account."}
      full
    >
      {#snippet control()}
        <div class="tx-stack">
          <div class="tx-key">
            <input
              id="deepgram-api-key"
              class="mx-input"
              type="password"
              autocomplete="off"
              aria-label="Deepgram API key"
              placeholder={deepgramKeyPresent ? "A key is saved — enter a new one to replace it" : "Paste your Deepgram API key"}
              disabled={deepgramSaving}
              bind:value={deepgramKeyInput}
            />
            <button
              class="mx-btn"
              type="button"
              disabled={deepgramSaving || deepgramKeyInput.trim().length === 0}
              aria-busy={deepgramSaving}
              onclick={saveDeepgramKey}
            >
              {#if deepgramSaving}<ButtonSpinner />Saving{:else}Save key{/if}
            </button>
            {#if deepgramKeyPresent}
              <button
                class="mx-btn"
                type="button"
                disabled={deepgramSaving || deepgramChecking}
                aria-busy={deepgramChecking}
                onclick={checkDeepgram}
              >
                {#if deepgramChecking}<ButtonSpinner />Checking{:else}Check connection{/if}
              </button>
              <button class="mx-btn mx-btn--ghost" type="button" disabled={deepgramSaving} onclick={removeDeepgramKey}>
                Remove key
              </button>
            {/if}
          </div>
          {#if deepgramSaveError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />Couldn’t save key: {deepgramSaveError}</span>
          {/if}
          <!-- The auth status is the standing rejection; the check result only
               adds news when there is no rejection on file (else it repeats it). -->
          {#if deepgramAuthStatus}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{deepgramAuthStatus}</span>
          {:else if deepgramCheckResult}
            <span class="mx-inline" data-tone={deepgramCheckResult.ok ? "ok" : "warn"} role="status">
              {#if deepgramCheckResult.ok}<IconCheck width="13" height="13" />{:else}<IconCloudOff width="13" height="13" />{/if}{deepgramCheckResult.message}
            </span>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  {#if transcriptionModelError}
    <ModelDownloadRow label="Model" error={`Couldn’t load model status: ${transcriptionModelError}`} />
  {:else if !m}
    <ModelDownloadRow
      label="Model"
      description={loadingTranscriptionModelStatus ? "Checking installed transcription models…" : undefined}
      warning={loadingTranscriptionModelStatus ? null : "No model status is available for the selected provider."}
    />
  {:else if selectedAppleSpeechPermissionStatus && selectedAppleSpeechPermissionStatus !== "available"}
    <!-- Apple Speech: denied / not asked / unusable is a permission notice, never "missing". -->
    <SettingRow label="Model" full>
      {#snippet control()}
        <div class="tx-stack">
          <div class="mx-notice" data-tone="warn">
            <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
            <b class="mx-notice__title">Apple Speech · {appleSpeechPermissionLabel(selectedAppleSpeechPermissionStatus)}</b>
            <p class="mx-notice__text">{appleSpeechPermissionHint(selectedAppleSpeechPermissionStatus)}</p>
            {#if selectedAppleSpeechNeedsPermission}
              <div class="mx-notice__acts">
                {#if selectedAppleSpeechPermissionStatus === "permission_not_determined"}
                  <button
                    type="button"
                    class="mx-btn mx-btn--sm"
                    onclick={requestAppleSpeechPermission}
                    disabled={requestingAppleSpeechPermission}
                    aria-busy={requestingAppleSpeechPermission}
                  >
                    {#if requestingAppleSpeechPermission}<ButtonSpinner />Requesting{:else}Get permission{/if}
                  </button>
                {:else}
                  <button type="button" class="mx-btn mx-btn--sm" onclick={openAppleSpeechPrivacySettings}>
                    Open System Settings<IconExternal width="12" height="12" />
                  </button>
                {/if}
              </div>
            {/if}
          </div>
          {#if appleSpeechPermissionError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />Permission request failed: {appleSpeechPermissionError}</span>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {:else if m.management !== "app_managed"}
    <ModelDownloadRow
      label="Model"
      description={rec.draftTranscriptionProvider === "deepgram"
        ? "Deepgram runs in the cloud. Availability depends on the API key above; there is no local model to download."
        : `${m.displayName} · ${transcriptionStatusLabel(m).toLowerCase()}`}
      status={m.available && rec.draftTranscriptionProvider !== "deepgram" ? "available" : null}
      chip={m.available ? null : { text: "unavailable", tone: "warn" }}
      controls={modelSelect}
    />
  {:else if selectedTranscriptionDownloadRunning}
    <ModelDownloadRow
      label="Model"
      description="Download is checked (sha256) before it’s marked installed."
      progress={downloadMeta(m.displayName, selectedTranscriptionDownloadProgress)}
      onCancel={cancelSelectedTranscriptionModelDownload}
      cancelling={cancellingTranscriptionDownload}
    />
  {:else}
    {@const failed = m.status === "failed" || !!transcriptionDownloadError}
    <ModelDownloadRow
      label="Model"
      description={m.available
        ? `${m.displayName} · installed`
        : failed
          ? m.displayName
          : `${m.displayName} isn’t downloaded${m.missingFiles.length > 0 ? ` (${m.missingFiles.length} ${m.missingFiles.length === 1 ? "file" : "files"} missing)` : ""}. Audio waits until it is.`}
      status={m.available ? "available" : null}
      chip={m.available ? null : failed ? { text: "failed", tone: "danger" } : { text: "missing", tone: "warn" }}
      error={transcriptionDownloadError ? `Download failed: ${transcriptionDownloadError}` : m.failureMessage}
      warning={!m.available && !m.download
        ? "This app-managed model is missing, but no packaged download artifact is available in the current manifest."
        : null}
      controls={modelSelect}
      download={!m.available && m.download
        ? { bytes: m.download.byteSize, starting: startingTranscriptionDownload, onclick: startSelectedTranscriptionModelDownload }
        : null}
      info={m}
      folder={{ path: models.transcriptionModelStatus?.modelsDirectory, reveal: m.available ? m.installPath : null }}
    />
  {/if}

  <SettingRow
    label="Language"
    description="Auto detects the spoken language, or pick a hint. Applies to future audio; queued jobs keep theirs."
  >
    {#snippet control()}
      <div class="tx-ctl">
        <Combobox
          value={rec.draftTranscriptionLanguage || "auto"}
          onValueChange={(v) => (rec.draftTranscriptionLanguage = v)}
          options={TRANSCRIPTION_LANGUAGE_OPTIONS}
          searchPlaceholder="Search languages…"
        />
      </div>
    {/snippet}
  </SettingRow>

  {#if rec.draftTranscriptionProvider === "parakeet"}
    <SettingRow label="Parakeet memory mode">
      {#snippet control()}
        <Segmented
          md
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
      <SettingRow label="Idle unload" description="Free the model after this long without audio.">
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
      description="Chunking limits peak memory; 0 turns it off. The int8 model also lowers disk and memory use."
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

  {#if m}
    {#if deleteUnusedTranscriptionModelsMessage || deleteUnusedTranscriptionModelsError}
      <SettingRow label="Delete unused models" description={deleteUnusedDesc} full>
        {#snippet aside()}{@render deleteUnused()}{/snippet}
        {#snippet control()}
          {#if deleteUnusedTranscriptionModelsError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />Delete failed: {deleteUnusedTranscriptionModelsError}</span>
          {:else}
            <span class="mx-inline" data-tone="ok" aria-live="polite"><IconCheck width="13" height="13" />{deleteUnusedTranscriptionModelsMessage}</span>
          {/if}
        {/snippet}
      </SettingRow>
    {:else}
      <SettingRow label="Delete unused models" description={deleteUnusedDesc}>
        {#snippet control()}{@render deleteUnused()}{/snippet}
      </SettingRow>
    {/if}
  {/if}
</SettingGroup>

{#snippet modelSelect()}
  {#if transcriptionModelOptions.length > 1}
    <div class="tx-ctl">
      <Combobox
        value={rec.draftTranscriptionModelId ?? "__os_managed__"}
        onValueChange={chooseTranscriptionModel}
        searchPlaceholder="Search models…"
        options={transcriptionModelOptions}
      />
    </div>
  {/if}
{/snippet}

{#snippet deleteUnused()}
  <button
    type="button"
    class="mx-btn mx-btn--ghost"
    onclick={requestDeleteUnusedTranscriptionModels}
    disabled={deletingUnusedTranscriptionModels || selectedTranscriptionDownloadRunning}
    aria-busy={deletingUnusedTranscriptionModels}
  >
    {#if deletingUnusedTranscriptionModels}<ButtonSpinner />Deleting…{:else}Delete unused{/if}
  </button>
{/snippet}

<style>
  /* Right-aligned selects/comboboxes keep a readable width (.st-select). */
  .tx-ctl {
    width: 240px;
  }

  .tx-stack {
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 100%;
  }

  /* settings.html .st-keyfield: key input + its actions on one line. */
  .tx-key {
    display: flex;
    gap: var(--s-2);
    width: 100%;
  }

  .tx-key .mx-input {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: var(--text-base);
  }
</style>

