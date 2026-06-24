<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import { semanticSearchProgressPercent } from "$lib/settings/state/models-format";
  import { formatBytes } from "$lib/settings/state/format";

  const c = getSettingsController();
  const rec = c.rec;
  const models = c.models;

  // Page-local locale hints (1:1 port of the legacy +page.svelte consts).
  const osLocale = typeof navigator !== "undefined" ? (navigator.language ?? "") : "";
  const osIsNonEnglish = osLocale.length > 0 && !osLocale.toLowerCase().startsWith("en");

  // Store-read aliases.
  const loadingSemanticSearchModelStatus = $derived(models.loadingSemanticSearchModelStatus);
  const semanticSearchModelStatus = $derived(models.semanticSearchModelStatus);
  const semanticSearchModelError = $derived(models.semanticSearchModelError);
  const semanticSearchSupportedModels = $derived(models.semanticSearchSupportedModels);
  const loadingSemanticSearchSupportedModels = $derived(models.loadingSemanticSearchSupportedModels);
  const semanticSearchSupportedModelsError = $derived(models.semanticSearchSupportedModelsError);
  const semanticSearchDownloadError = $derived(models.semanticSearchDownloadError);
  const startingSemanticSearchDownload = $derived(models.startingSemanticSearchDownload);
  const cancellingSemanticSearchDownload = $derived(models.cancellingSemanticSearchDownload);
  const semanticSearchReindexing = $derived(models.semanticSearchReindexing);
  const semanticSearchReindexMessage = $derived(models.semanticSearchReindexMessage);

  // rec field alias.
  const semanticSearchSelectedModelId = $derived(rec.semanticSearchSelectedModelId);

  // Controller derived selectors.
  const semanticSearchModelOptions = $derived(c.semanticSearchModelOptions);
  const semanticSearchPickedModel = $derived(c.semanticSearchPickedModel);
  const semanticSearchPickedProgress = $derived(c.semanticSearchPickedProgress);

  // Controller / store action methods.
  const loadSemanticSearchModelStatus = () => c.loadSemanticSearchModelStatus();
  const setSemanticSearchEnabled = (value: boolean) => c.setSemanticSearchEnabled(value);
  const cancelSemanticSearchModelDownload = () => c.cancelSemanticSearchModelDownload();
  const startSemanticSearchPickedDownload = (
    model: Parameters<typeof c.startSemanticSearchPickedDownload>[0],
  ) => c.startSemanticSearchPickedDownload(model);
  const chooseSemanticSearchPickedModel = (
    model: Parameters<typeof c.chooseSemanticSearchPickedModel>[0],
  ) => c.chooseSemanticSearchPickedModel(model);
</script>

<SettingGroup
  id="settings-section-semanticSearch"
  title="Semantic Search Model"
  hint="Meaning-based search runs fully on-device — on the GPU where available, otherwise the CPU. Pick a supported model, then Mnema embeds your captures in the background. Nothing is downloaded until you choose a model. The model stays on as a background indexer — it keeps re-embedding new captures (ongoing CPU/GPU and battery while it catches up), and switching models re-indexes every existing capture."
>
  {#snippet actions()}
    <ReloadButton
      onclick={() => void loadSemanticSearchModelStatus()}
      busy={loadingSemanticSearchModelStatus}
      title="Refresh"
      label="Refresh semantic search model status"
    />
  {/snippet}

  <SettingRow
    label="Enable semantic search"
    description="Fuse meaning-based results with keyword search. Inert until a model below is installed."
  >
    {#snippet control()}
      <Switch
        checked={rec.draftSemanticSearchEnabled}
        onCheckedChange={(value) => void setSemanticSearchEnabled(value)}
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Model" full>
    {#snippet control()}
      <div class="ss-stack">
        {#if osIsNonEnglish}
          <p class="group-hint">
            Your system language ({osLocale}) isn’t English — the Multilingual tier is recommended so
            non-English captures aren’t degraded by the English default.
          </p>
        {/if}

        {#if semanticSearchModelError}
          <p class="group-hint group-hint--warn">Model status failed: {semanticSearchModelError}</p>
        {/if}

        {#if semanticSearchModelStatus}
          <Combobox
            label=""
            placeholder="Select a model — recommended tiers first…"
            searchPlaceholder="Search models…"
            value={c.semanticSearchPickedModelId}
            onValueChange={(v) => (c.semanticSearchPickedModelId = v)}
            options={semanticSearchModelOptions}
          />
          <p class="group-hint">
            Recommended tiers are listed first. Pick from the supported on-device models;
            multilingual models are marked. Nothing downloads until you choose below.
          </p>

          {#if semanticSearchSupportedModelsError}
            <p class="group-hint group-hint--warn">Custom model list failed: {semanticSearchSupportedModelsError}</p>
          {:else if loadingSemanticSearchSupportedModels && semanticSearchSupportedModels.length === 0}
            <p class="group-hint">Loading supported models…</p>
          {/if}

          {#if semanticSearchPickedModel}
            {@const picked = semanticSearchPickedModel}
            {@const progress = semanticSearchPickedProgress}
            {@const installed = picked.available}
            {@const selected = semanticSearchSelectedModelId === picked.modelId}
            {@const downloading =
              !!progress &&
              (progress.status === "downloading" ||
                progress.status === "starting" ||
                progress.status === "installing")}
            <div class="settings-group ss-picked" role="group" aria-label={picked.displayName}>
              <div class="row-actions" style="justify-content: space-between; align-items: flex-start;">
                <div>
                  <strong>{picked.displayName}</strong>
                  <p class="group-hint">{picked.description}</p>
                  <p class="group-hint">{picked.metaLine}</p>
                </div>
                <span class="badge {selected ? 'badge--ok' : 'badge--neutral'} badge--sm">
                  {selected
                    ? "Active"
                    : installed
                      ? "Installed"
                      : downloading
                        ? `Downloading ${progress ? semanticSearchProgressPercent(progress) : 0}%`
                        : progress && progress.status === "failed"
                          ? "Failed"
                          : "Not installed"}
                </span>
              </div>

              {#if downloading && progress}
                <div class="download-progress" aria-live="polite">
                  <div class="download-progress__bar">
                    <span style={`width: ${semanticSearchProgressPercent(progress)}%`}></span>
                  </div>
                  <p class="group-hint">
                    {semanticSearchProgressPercent(progress)}% · {formatBytes(progress.downloadedBytes)}{progress.totalBytes ? ` of ${formatBytes(progress.totalBytes)}` : ""}
                  </p>
                </div>
              {/if}

              {#if downloading || !installed || !selected}
                <div class="row-actions">
                  {#if downloading}
                    <button
                      class="btn btn--ghost btn--sm"
                      onclick={() => void cancelSemanticSearchModelDownload()}
                      disabled={cancellingSemanticSearchDownload}
                    >
                      Cancel
                    </button>
                  {:else if !installed}
                    <!-- Step 1: download. Mnema never auto-downloads (ADR 0036). -->
                    <button
                      class="btn btn--primary btn--sm"
                      onclick={() => void startSemanticSearchPickedDownload(picked)}
                      disabled={!picked.provider || startingSemanticSearchDownload}
                    >
                      {picked.approxDownloadBytes != null
                        ? `Download (${formatBytes(picked.approxDownloadBytes)})`
                        : "Download"}
                    </button>
                  {:else if !selected}
                    <!-- Step 2: use (installed, not yet active). -->
                    <button
                      class="btn btn--primary btn--sm"
                      onclick={() => void chooseSemanticSearchPickedModel(picked)}
                    >
                      Use this model
                    </button>
                  {/if}
                  {#if !installed && !downloading}
                    <span class="action-hint">Step 1: download · Step 2: use this model</span>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}

          {#if semanticSearchDownloadError}
            <p class="group-hint group-hint--warn">Download failed: {semanticSearchDownloadError}</p>
          {/if}
          {#if semanticSearchReindexing}
            <p class="group-hint">Re-indexing — clearing existing vectors…</p>
          {:else if semanticSearchReindexMessage}
            <p class="group-hint">{semanticSearchReindexMessage}</p>
          {/if}
        {:else if loadingSemanticSearchModelStatus}
          <p class="group-hint">Checking installed search models…</p>
        {:else}
          <p class="group-hint group-hint--warn">No search model status is available.</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Wide "Model" row stacks the picker, hints, and the bordered picked-model
     sub-block; the primitives only gap rows, not the contents of one control
     slot. */
  .ss-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
