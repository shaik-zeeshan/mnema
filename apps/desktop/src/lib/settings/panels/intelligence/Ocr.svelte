<script lang="ts">
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import { OCR_LANGUAGE_OPTIONS } from "$lib/settings/ocr-languages";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelDownloadRow from "$lib/settings/ui/ModelDownloadRow.svelte";
  import { downloadMeta } from "$lib/settings/ui/model-download";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";
  import { ocrStatusLabel } from "$lib/settings/state/models-format";
  import { formatBytes } from "$lib/settings/state/format";
  import type { OcrTesseractPageSegmentationMode, OcrTesseractPreprocessMode } from "$lib/types";

  const c = getSettingsController();
  const rec = c.rec;
  const models = c.models;

  // Store-read aliases.
  const loadingOcrModelStatus = $derived(models.loadingOcrModelStatus);
  const ocrModelError = $derived(models.ocrModelError);
  const startingOcrDownload = $derived(models.startingOcrDownload);
  const cancellingOcrDownload = $derived(models.cancellingOcrDownload);
  const ocrDownloadError = $derived(models.ocrDownloadError);
  const deletingUnusedOcrModels = $derived(models.deletingUnusedOcrModels);
  const deleteUnusedOcrModelsMessage = $derived(models.deleteUnusedOcrModelsMessage);
  const deleteUnusedOcrModelsError = $derived(models.deleteUnusedOcrModelsError);

  // Controller derived selectors.
  const ocrProviderOptions = $derived(c.ocrProviderOptions);
  const ocrModelOptions = $derived(c.ocrModelOptions);
  const m = $derived(c.selectedOcrModel);
  const selectedOcrDownloadProgress = $derived(c.selectedOcrDownloadProgress);
  const selectedOcrDownloadRunning = $derived(c.selectedOcrDownloadRunning);

  // Controller / store action methods.
  const loadOcrModelStatus = () => c.loadOcrModelStatus();
  const chooseOcrProvider = (provider: string) => c.chooseOcrProvider(provider);
  const chooseOcrModel = (value: string) => c.chooseOcrModel(value);
  const startSelectedOcrModelDownload = () => c.startSelectedOcrModelDownload();
  const cancelSelectedOcrModelDownload = () => c.cancelSelectedOcrModelDownload();
  const requestDeleteUnusedOcrModels = () => c.requestDeleteUnusedOcrModels();
  const deleteUnusedDesc = "Removes app-managed OCR model files except the selected one.";
</script>

<SettingGroup
  id="settings-section-ocr"
  title="OCR &amp; Previews"
  hint="Engine, model and preview caching."
>
  {#snippet actions()}
    <ReloadButton
      onclick={loadOcrModelStatus}
      busy={loadingOcrModelStatus}
      title="Refresh"
      label="Refresh OCR model status"
    />
  {/snippet}

  <SettingRow
    label="Enable OCR"
    description="Queue OCR for captured screen frames when the selected engine is available. While on, recording waits for the engine."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftOcrEnabled} ariaLabel="Enable OCR" />
    {/snippet}
  </SettingRow>

  <SettingRow label="Provider" description="Existing OCR results stay visible after switching.">
    {#snippet control()}
      <Segmented
        md
        value={rec.draftOcrProvider}
        onValueChange={chooseOcrProvider}
        disabled={!rec.draftOcrEnabled}
        ariaLabel="OCR provider"
        options={ocrProviderOptions.length > 0 ? ocrProviderOptions : [
          { value: "apple_vision", label: "Apple Vision" },
          { value: "tesseract", label: "Tesseract" },
        ]}
      />
    {/snippet}
  </SettingRow>

  <!-- ponytail: a one-model provider (Apple Vision, the bundled Tesseract)
       has nothing to pick, so the row only appears when there is a choice. -->
  {#if ocrModelOptions.length > 1}
    <SettingRow label="Model">
      {#snippet control()}
        <div class="ocr-ctl">
          <Combobox
            value={rec.draftOcrModelId ?? "__os_managed__"}
            onValueChange={chooseOcrModel}
            disabled={!rec.draftOcrEnabled}
            searchPlaceholder="Search models…"
            options={ocrModelOptions}
          />
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  {#if rec.draftOcrProvider === "apple_vision"}
    <SettingRow label="Recognition mode">
      {#snippet control()}
        <Segmented
          md
          bind:value={rec.draftOcrRecognitionMode}
          disabled={!rec.draftOcrEnabled}
          ariaLabel="Recognition mode"
          options={[
            { value: "fast", label: "Fast" },
            { value: "accurate", label: "Accurate" },
          ]}
        />
      {/snippet}
    </SettingRow>
    <SettingRow label="Language correction">
      {#snippet control()}
        <Switch bind:checked={rec.draftOcrLanguageCorrection} disabled={!rec.draftOcrEnabled} ariaLabel="Language correction" />
      {/snippet}
    </SettingRow>
  {:else if rec.draftOcrProvider === "tesseract"}
    <SettingRow label="Language">
      {#snippet control()}
        <div class="ocr-ctl">
          <Combobox
            value={rec.draftOcrLanguage || "eng"}
            onValueChange={(v) => (rec.draftOcrLanguage = v)}
            disabled={!rec.draftOcrEnabled}
            options={OCR_LANGUAGE_OPTIONS}
            searchPlaceholder="Search languages…"
          />
        </div>
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Page segmentation"
      description="Auto for mixed layouts; Sparse text for screenshots with scattered text."
    >
      {#snippet control()}
        <div class="ocr-ctl">
        <SelectMenu
          value={rec.draftOcrTesseractPageSegmentationMode}
          onValueChange={(value) => { rec.draftOcrTesseractPageSegmentationMode = value as OcrTesseractPageSegmentationMode; }}
          disabled={!rec.draftOcrEnabled}
          options={[
            { value: "auto", label: "Auto" },
            { value: "single_block", label: "Single block" },
            { value: "single_line", label: "Single line" },
            { value: "single_word", label: "Single word" },
            { value: "sparse_text", label: "Sparse text" },
          ]}
        />
        </div>
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Image preprocessing"
      description="Grayscale suits clean UI text. Thresholded black/white helps when edges are muddy or contrast is weak."
    >
      {#snippet control()}
        <Segmented
          md
          value={rec.draftOcrTesseractPreprocessMode}
          onValueChange={(value) => { rec.draftOcrTesseractPreprocessMode = value as OcrTesseractPreprocessMode; }}
          disabled={!rec.draftOcrEnabled}
          ariaLabel="Image preprocessing"
          options={[
            { value: "grayscale", label: "Grayscale" },
            { value: "thresholded", label: "Thresholded" },
          ]}
        />
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Upscale before OCR"
      description="Tesseract works best around 300 DPI. For tiny text try 2× before 3× or 4×."
    >
      {#snippet control()}
        <Segmented
          md
          value={String(rec.draftOcrTesseractUpscaleFactor)}
          onValueChange={(value) => { rec.draftOcrTesseractUpscaleFactor = parseInt(value, 10) || 1; }}
          disabled={!rec.draftOcrEnabled}
          mono
          ariaLabel="Upscale before OCR"
          options={[
            { value: "1", label: "1×" },
            { value: "2", label: "2×" },
            { value: "3", label: "3×" },
            { value: "4", label: "4×" },
          ]}
        />
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Character whitelist"
      description="Optional. Only these characters are recognised."
    >
      {#snippet control()}
        <input
          id="ocr-tesseract-whitelist"
          class="mx-input num ocr-ctl"
          aria-label="Tesseract character whitelist"
          bind:value={rec.draftOcrTesseractCharWhitelist}
          disabled={!rec.draftOcrEnabled}
          placeholder="ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"
        />
      {/snippet}
    </SettingRow>
  {/if}

  {#if ocrModelError}
    <ModelDownloadRow label="Model status" error={`Couldn’t load OCR model status: ${ocrModelError}`} />
  {:else if !m}
    <ModelDownloadRow
      label="Model status"
      description={loadingOcrModelStatus ? "Checking installed OCR models…" : undefined}
      warning={loadingOcrModelStatus ? null : "No OCR model status is available for the selected provider."}
    />
  {:else if m.management !== "app_managed"}
    <ModelDownloadRow
      label="Model status"
      description="Managed by macOS — nothing to download."
      status={m.available ? "available" : null}
      chip={m.available ? null : { text: "unavailable", tone: "warn" }}
      warning={m.runtimeMessage}
    />
  {:else if selectedOcrDownloadRunning}
    <ModelDownloadRow
      label="Model status"
      progress={downloadMeta(m.displayName, selectedOcrDownloadProgress)}
      onCancel={cancelSelectedOcrModelDownload}
      cancelling={cancellingOcrDownload}
    />
  {:else}
    {@const failed = m.status === "failed" || !!ocrDownloadError}
    <ModelDownloadRow
      label="Model status"
      description={!m.available && !failed && m.missingFiles.length > 0
        ? `${m.missingFiles.length} ${m.missingFiles.length === 1 ? "file" : "files"} missing — download to complete this model.`
        : `${m.displayName} · ${m.available ? "installed" : failed ? "not installed" : ocrStatusLabel(m).toLowerCase()}${m.download ? ` · ${formatBytes(m.download.byteSize)}` : ""}`}
      status={m.available ? "available" : null}
      chip={m.available ? null : failed ? { text: "failed", tone: "danger" } : { text: m.status === "missing" ? "missing" : "unavailable", tone: "warn" }}
      error={ocrDownloadError ?? m.failureMessage}
      warning={m.runtimeMessage ?? (!m.available && !m.download
        ? m.provider === "tesseract"
          ? "This provider still needs a Mnema-published self-contained runtime bundle before in-app download can work."
          : "This app-managed OCR bundle is missing, and the current manifest does not ship a downloadable artifact yet."
        : null)}
      download={!m.available && m.download
        ? { bytes: m.download.byteSize, starting: startingOcrDownload, onclick: startSelectedOcrModelDownload }
        : null}
      info={m}
      folder={{ path: models.ocrModelStatus?.modelsDirectory, reveal: m.available ? m.installPath : null }}
    />
  {/if}

  {#if m}
    {#if deleteUnusedOcrModelsMessage || deleteUnusedOcrModelsError}
      <SettingRow label="Delete unused models" description={deleteUnusedDesc} full>
        {#snippet aside()}{@render deleteUnused()}{/snippet}
        {#snippet control()}
          {#if deleteUnusedOcrModelsError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />Delete failed: {deleteUnusedOcrModelsError}</span>
          {:else}
            <span class="mx-inline" data-tone="ok" aria-live="polite"><IconCheck width="13" height="13" />{deleteUnusedOcrModelsMessage}</span>
          {/if}
        {/snippet}
      </SettingRow>
    {:else}
      <SettingRow label="Delete unused models" description={deleteUnusedDesc}>
        {#snippet control()}{@render deleteUnused()}{/snippet}
      </SettingRow>
    {/if}
  {/if}

  <SettingRow
    label="Cache duration"
    description={rec.draftPreviewCacheTtlSeconds === 0
      ? "Disabled — previews are fetched fresh every time."
      : "In-memory cache for frame and image previews."}
  >
    {#snippet control()}
      <div class="ocr-ctl">
        <SelectMenu
          value={String(rec.draftPreviewCacheTtlSeconds)}
          onValueChange={(v) => { rec.draftPreviewCacheTtlSeconds = parseInt(v, 10); }}
          options={[
            { value: "0",     label: "Disabled" },
            { value: "300",   label: "5 minutes" },
            { value: "900",   label: "15 minutes" },
            { value: "3600",  label: "1 hour (default)" },
            { value: "21600", label: "6 hours" },
            { value: "86400", label: "24 hours" },
          ]}
        />
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

{#snippet deleteUnused()}
  <button
    type="button"
    class="mx-btn mx-btn--ghost"
    onclick={requestDeleteUnusedOcrModels}
    disabled={deletingUnusedOcrModels || selectedOcrDownloadRunning}
    aria-busy={deletingUnusedOcrModels}
  >
    {#if deletingUnusedOcrModels}<ButtonSpinner />Deleting…{:else}Delete unused{/if}
  </button>
{/snippet}

<style>
  /* Right-aligned selects/comboboxes keep a readable width (.st-select). */
  .ocr-ctl {
    width: 240px;
  }
</style>
