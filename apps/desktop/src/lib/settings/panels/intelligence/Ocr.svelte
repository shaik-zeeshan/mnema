<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
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
  const confirmingDeleteUnusedOcrModels = $derived(models.confirmingDeleteUnusedOcrModels);
  const deleteUnusedOcrModelsMessage = $derived(models.deleteUnusedOcrModelsMessage);
  const deletedUnusedOcrModelLabels = $derived(models.deletedUnusedOcrModelLabels);
  const skippedUnusedOcrModelLabels = $derived(models.skippedUnusedOcrModelLabels);
  const skippedOcrProcessingJobModelLabels = $derived(models.skippedOcrProcessingJobModelLabels);
  const deleteUnusedOcrModelsError = $derived(models.deleteUnusedOcrModelsError);

  // Controller derived selectors.
  const ocrProviderOptions = $derived(c.ocrProviderOptions);
  const ocrModelOptions = $derived(c.ocrModelOptions);
  const selectedOcrModel = $derived(c.selectedOcrModel);
  const selectedOcrDownloadProgress = $derived(c.selectedOcrDownloadProgress);
  const selectedOcrDownloadRunning = $derived(c.selectedOcrDownloadRunning);
  const selectedOcrDownloadPercent = $derived(c.selectedOcrDownloadPercent);

  // Controller / store action methods.
  const loadOcrModelStatus = () => c.loadOcrModelStatus();
  const chooseOcrProvider = (provider: string) => c.chooseOcrProvider(provider);
  const chooseOcrModel = (value: string) => c.chooseOcrModel(value);
  const startSelectedOcrModelDownload = () => c.startSelectedOcrModelDownload();
  const cancelSelectedOcrModelDownload = () => c.cancelSelectedOcrModelDownload();
  const requestDeleteUnusedOcrModels = () => c.requestDeleteUnusedOcrModels();
  // The legacy confirmation block (`confirmingDeleteUnusedOcrModels`) is inert —
  // `requestDeleteUnusedOcrModels` runs its own dialog and performs the delete —
  // so this verbatim onclick target routes to the same public path.
  const deleteUnusedOcrModels = () => c.requestDeleteUnusedOcrModels();
</script>

<SettingGroup
  id="settings-section-ocr"
  title="OCR &amp; Previews"
  hint="Choose the OCR engine, inspect model availability, and tune preview caching."
>
  {#snippet actions()}
    <button class="btn btn--ghost btn--sm" onclick={loadOcrModelStatus} disabled={loadingOcrModelStatus}>
      {loadingOcrModelStatus ? "Checking" : "Refresh"}
    </button>
  {/snippet}

  <SettingRow
    label="Enable OCR"
    description="Automatically queue OCR for captured screen frames when the selected engine is available"
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftOcrEnabled} />
    {/snippet}
  </SettingRow>

  <SettingRow label="Provider" full>
    {#snippet control()}
      <RadioGroup
        value={rec.draftOcrProvider}
        onValueChange={chooseOcrProvider}
        disabled={!rec.draftOcrEnabled}
        options={ocrProviderOptions.length > 0 ? ocrProviderOptions : [
          { value: "apple_vision", label: "Apple Vision", description: "Model status is loading" },
          { value: "tesseract", label: "Tesseract", description: "Model status is loading" },
        ]}
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Model" full>
    {#snippet control()}
      <SelectMenu
        value={rec.draftOcrModelId ?? "__os_managed__"}
        onValueChange={chooseOcrModel}
        disabled={!rec.draftOcrEnabled}
        options={ocrModelOptions.length > 0 ? ocrModelOptions : [
          { value: rec.draftOcrModelId ?? "__os_managed__", label: "Loading model options" },
        ]}
      />
    {/snippet}
  </SettingRow>

  {#if rec.draftOcrProvider === "tesseract"}
    <SettingRow label="Language" full>
      {#snippet control()}
        <input
          id="ocr-language"
          class="text-input"
          bind:value={rec.draftOcrLanguage}
          disabled={!rec.draftOcrEnabled}
          placeholder="eng"
        />
      {/snippet}
    </SettingRow>
  {/if}

  {#if rec.draftOcrProvider === "apple_vision"}
    <SettingRow label="Recognition mode" full>
      {#snippet control()}
        <RadioGroup
          bind:value={rec.draftOcrRecognitionMode}
          disabled={!rec.draftOcrEnabled}
          options={[
            { value: "fast", label: "Fast", description: "Lower CPU usage; default for continuous capture." },
            { value: "accurate", label: "Accurate", description: "Higher OCR cost with better Apple Vision accuracy." },
          ]}
        />
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Language correction"
      description="Let Apple Vision spend extra work correcting recognized text using language models"
    >
      {#snippet control()}
        <Switch bind:checked={rec.draftOcrLanguageCorrection} disabled={!rec.draftOcrEnabled} />
      {/snippet}
    </SettingRow>
  {:else if rec.draftOcrProvider === "tesseract"}
    <SettingRow
      label="Page segmentation"
      description="Use Auto for mixed layouts, Single block for paragraph regions, Single line for titles/labels, Single word for isolated tokens, and Sparse text for screenshots with scattered text."
      full
    >
      {#snippet control()}
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
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Image preprocessing"
      description="Grayscale usually works best for clean UI text. Thresholded black/white can help when edges are muddy or contrast is weak."
      full
    >
      {#snippet control()}
        <SelectMenu
          value={rec.draftOcrTesseractPreprocessMode}
          onValueChange={(value) => { rec.draftOcrTesseractPreprocessMode = value as OcrTesseractPreprocessMode; }}
          disabled={!rec.draftOcrEnabled}
          options={[
            { value: "grayscale", label: "Grayscale" },
            { value: "thresholded", label: "Thresholded" },
          ]}
        />
      {/snippet}
    </SettingRow>
    <SettingRow
      label="Upscale before OCR"
      description="Tesseract works best around 300 DPI. For tiny screenshots, 2× upscaling is a good first step before trying 3× or 4×."
      full
    >
      {#snippet control()}
        <SelectMenu
          value={String(rec.draftOcrTesseractUpscaleFactor)}
          onValueChange={(value) => { rec.draftOcrTesseractUpscaleFactor = parseInt(value, 10) || 1; }}
          disabled={!rec.draftOcrEnabled}
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
      description="Tesseract works best with dark, high-contrast text on a light background. For tiny screenshots, try 2× upscaling first; if edges are muddy, try thresholded preprocessing or a narrow whitelist."
      full
    >
      {#snippet control()}
        <input
          id="ocr-tesseract-whitelist"
          class="text-input"
          bind:value={rec.draftOcrTesseractCharWhitelist}
          disabled={!rec.draftOcrEnabled}
          placeholder="Optional, e.g. ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-"
        />
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow label="OCR availability" full>
    {#snippet control()}
      <p class="group-hint">
        {rec.draftOcrEnabled
          ? "If screen capture is enabled, recording start is blocked until the selected OCR provider is available."
          : "Screen recording can start without OCR while this is disabled."}
        Existing OCR results remain visible after switching engines.
      </p>
    {/snippet}
  </SettingRow>

  <SettingRow label="Selected model status" full divider={false}>
    {#snippet control()}
      <div class="ocr-stack">
        {#if ocrModelError}
          <p class="group-hint group-hint--warn">Failed to load OCR model status: {ocrModelError}</p>
        {:else if selectedOcrModel}
          <div class="model-status" class:model-status--available={selectedOcrModel.available}>
            <div>
              <div class="model-status__title">{selectedOcrModel.displayName}</div>
              <div class="model-status__meta">{ocrStatusLabel(selectedOcrModel)}</div>
            </div>
            <span class="model-status__pill">{selectedOcrModel.available ? "available" : "unavailable"}</span>
          </div>
          <p class="group-hint">{selectedOcrModel.description}</p>
          {#if selectedOcrModel.runtimeMessage}
            <p class="group-hint group-hint--warn"><strong>Runtime:</strong> {selectedOcrModel.runtimeMessage}</p>
          {/if}
          {#if selectedOcrModel.installPath}
            <p class="group-hint"><strong>Install path:</strong> {selectedOcrModel.installPath}</p>
          {/if}
          {#if selectedOcrModel.missingFiles.length > 0}
            <p class="group-hint group-hint--warn"><strong>Missing files:</strong> {selectedOcrModel.missingFiles.join(", ")}</p>
          {/if}
          {#if selectedOcrModel.failureMessage}
            <p class="group-hint group-hint--warn"><strong>Failure:</strong> {selectedOcrModel.failureMessage}</p>
          {/if}
          {#if selectedOcrModel.licenseLabel || selectedOcrModel.sourceUrl}
            <p class="group-hint">
              {#if selectedOcrModel.licenseLabel}<strong>License:</strong> {selectedOcrModel.licenseLabel}. {/if}
              {#if selectedOcrModel.sourceUrl}<strong>Source:</strong> {selectedOcrModel.sourceUrl}{/if}
            </p>
          {/if}
          {#if selectedOcrModel.management === "app_managed"}
            {#if selectedOcrModel.download}
              {#if selectedOcrDownloadRunning}
                <div class="download-progress" aria-live="polite">
                  <div class="download-progress__bar">
                    <span style={`width: ${selectedOcrDownloadPercent ?? 8}%`}></span>
                  </div>
                  <p class="group-hint">
                    {selectedOcrDownloadProgress?.status ?? "downloading"}
                    {#if selectedOcrDownloadPercent !== null} · {selectedOcrDownloadPercent}%{/if}
                    {#if selectedOcrDownloadProgress?.message} · {selectedOcrDownloadProgress.message}{/if}
                  </p>
                  <button class="btn btn--ghost" onclick={cancelSelectedOcrModelDownload} disabled={cancellingOcrDownload}>
                    {cancellingOcrDownload ? "Cancelling" : "Cancel download"}
                  </button>
                </div>
              {:else}
                <button class="btn btn--ghost" onclick={startSelectedOcrModelDownload} disabled={startingOcrDownload || selectedOcrModel.available}>
                  {startingOcrDownload ? "Starting" : `Download (${formatBytes(selectedOcrModel.download.byteSize)})`}
                </button>
              {/if}
            {:else if !selectedOcrModel.available}
              <p class="group-hint group-hint--warn">
                {#if selectedOcrModel.provider === "tesseract"}
                  This provider still needs a Mnema-published self-contained runtime bundle before in-app download can work.
                {:else}
                  This app-managed OCR bundle is missing, and the current manifest does not ship a downloadable artifact yet.
                {/if}
              </p>
            {/if}
            {#if ocrDownloadError}
              <p class="group-hint group-hint--warn">Download failed: {ocrDownloadError}</p>
            {/if}
          {:else}
            <p class="group-hint">This provider is managed by macOS. There is no app-managed model download.</p>
          {/if}
          <div class="debug-log-actions">
            <button class="btn btn--danger" onclick={requestDeleteUnusedOcrModels} disabled={deletingUnusedOcrModels || selectedOcrDownloadRunning}>
              Delete unused OCR models
            </button>
          </div>
          <p class="group-hint">Removes app-managed OCR model files except the model selected above.</p>
          {#if confirmingDeleteUnusedOcrModels}
            <div class="delete-confirmation" role="alert">
              <strong>Delete unused OCR models?</strong>
              <p>This removes app-managed OCR model directories that are not currently selected. The selected model, active downloads, and running OCR jobs are kept. Queued and failed OCR jobs using deleted models are moved to the current OCR selection.</p>
              <div class="debug-log-actions">
                <button class="btn btn--danger" onclick={deleteUnusedOcrModels} disabled={deletingUnusedOcrModels}>
                  {deletingUnusedOcrModels ? "Deleting" : "Confirm delete"}
                </button>
                <button class="btn btn--ghost" onclick={() => { models.confirmingDeleteUnusedOcrModels = false; }} disabled={deletingUnusedOcrModels}>
                  Cancel
                </button>
              </div>
            </div>
          {/if}
          {#if deleteUnusedOcrModelsMessage}
            <div class="cleanup-result" aria-live="polite">
              <strong>{deleteUnusedOcrModelsMessage}</strong>
              {#if deletedUnusedOcrModelLabels.length > 0}
                <p>Deleted:</p>
                <ul>
                  {#each deletedUnusedOcrModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedUnusedOcrModelLabels.length > 0}
                <p>Skipped active downloads:</p>
                <ul>
                  {#each skippedUnusedOcrModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
              {#if skippedOcrProcessingJobModelLabels.length > 0}
                <p>Skipped running jobs:</p>
                <ul>
                  {#each skippedOcrProcessingJobModelLabels as model}
                    <li>{model}</li>
                  {/each}
                </ul>
              {/if}
            </div>
          {/if}
          {#if deleteUnusedOcrModelsError}
            <p class="group-hint group-hint--warn">Delete failed: {deleteUnusedOcrModelsError}</p>
          {/if}
        {:else if loadingOcrModelStatus}
          <p class="group-hint">Checking installed OCR models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No OCR model status is available for the selected provider.</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup title="Preview Cache">
  <SettingRow
    label="Cache duration"
    description="In-memory cache for frame and image previews. Cached entries expire automatically after the selected duration."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="ocr-stack">
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
        {#if rec.draftPreviewCacheTtlSeconds === 0}
          <p class="group-hint"><strong>Disabled</strong> — previews are fetched fresh every time.</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Wide rows stack a control over hints and the bordered model-status /
     download / cleanup sub-blocks; the primitives only gap whole rows. */
  .ocr-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
