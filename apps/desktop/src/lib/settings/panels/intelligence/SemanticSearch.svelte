<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("semanticSearch");

  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ModelFootprintHint from "$lib/settings/ui/ModelFootprintHint.svelte";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";
  import { semanticCoverage, semanticIndexPrice } from "$lib/settings/state/system-facts";
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
  const deletingSemanticSearchModel = $derived(models.deletingSemanticSearchModel);
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
  // Toggling re-reads the machine facts: off→on swaps the price copy for the
  // coverage meter, and the meter must open on a fresh count, not the one
  // cached when Settings mounted.
  const setSemanticSearchEnabled = async (value: boolean) => {
    await c.setSemanticSearchEnabled(value);
    await systemFacts.refresh();
  };
  const cancelSemanticSearchModelDownload = () => c.cancelSemanticSearchModelDownload();
  const startSemanticSearchPickedDownload = (
    model: Parameters<typeof c.startSemanticSearchPickedDownload>[0],
  ) => c.startSemanticSearchPickedDownload(model);
  const chooseSemanticSearchPickedModel = (
    model: Parameters<typeof c.chooseSemanticSearchPickedModel>[0],
  ) => c.chooseSemanticSearchPickedModel(model);
  const deleteSemanticSearchPickedModel = (
    model: Parameters<typeof c.deleteSemanticSearchPickedModel>[0],
  ) => c.deleteSemanticSearchPickedModel(model);

  // G10's price-before-enable, priced per G8: the index cost is this machine's
  // real un-indexed capture count times the schema's bytes-per-vector. No
  // processing-time figure — there is no measured embedding throughput to
  // honestly quote one from.
  void systemFacts.ensureLoaded();
  const semanticPrice = $derived(semanticIndexPrice(systemFacts.value));

  // …and its ON-state counterpart: once the feature is on, the price is spent,
  // so the row states coverage instead — the same two real counts as a
  // fraction. Never rendered in the off state (G10).
  // ponytail: the counts refresh on toggle and on the group's Refresh button,
  // not on a timer — indexing progress polls if a static meter proves annoying.
  const coverage = $derived(semanticCoverage(systemFacts.value));
</script>

<SettingGroup
  id="settings-section-semanticSearch"
  title="Semantic Search Model"
  hint="Meaning-based search runs fully on-device. Pick a supported model and Mnema embeds your captures in the background; nothing downloads until you choose one."
>
  {#snippet actions()}
    <ReloadButton
      onclick={() => {
        void loadSemanticSearchModelStatus();
        void systemFacts.refresh();
      }}
      busy={loadingSemanticSearchModelStatus}
      title="Refresh"
      label="Refresh semantic search model status"
    />
  {/snippet}

  <SettingRow
    label="Enable semantic search"
    description="Fuse meaning-based results with keyword search. Inert until a model below is installed."
    full
  >
    {#snippet aside()}
      <Switch
        ariaLabel="Enable semantic search"
        checked={rec.draftSemanticSearchEnabled}
        onCheckedChange={(value) => void setSemanticSearchEnabled(value)}
      />
    {/snippet}
    {#snippet control()}
      <div class="ss-stack">
        <p class="group-hint group-hint--warn">
          Stays on as a background indexer — ongoing CPU/GPU and battery while it catches up, and switching models re-indexes every existing capture.
        </p>
        {#if !rec.draftSemanticSearchEnabled}
          {#if semanticPrice}
            <!-- G10 price-before-enable: what switching it on costs, computed
                 from this machine's real pending-capture count. -->
            <p class="inst-cost">{semanticPrice}</p>
          {/if}
        {:else if coverage}
          <div class="download-progress" aria-live="polite">
            <div
              class="download-progress__bar meter"
              role="progressbar"
              aria-label="Semantic index coverage"
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={coverage.percent}
              aria-valuetext={coverage.phrase}
            >
              {#if coverage.percent > 0}
                <span style={`width: ${coverage.percent}%`}></span>
              {/if}
            </div>
            <p class="group-hint">{coverage.phrase}</p>
          </div>
        {/if}
      </div>
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
              <div class="row-actions row-actions--between">
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
                      type="button"
                      class="btn btn--ghost btn--sm"
                      onclick={() => void cancelSemanticSearchModelDownload()}
                      disabled={cancellingSemanticSearchDownload}
                      aria-busy={cancellingSemanticSearchDownload}
                    >
                      {#if cancellingSemanticSearchDownload}<ButtonSpinner />Cancelling{:else}Cancel{/if}
                    </button>
                  {:else if !installed}
                    <!-- Step 1: download. Mnema never auto-downloads (ADR 0036). -->
                    <button
                      type="button"
                      class="btn btn--primary btn--sm"
                      onclick={() => void startSemanticSearchPickedDownload(picked)}
                      disabled={!picked.provider || startingSemanticSearchDownload}
                      aria-busy={startingSemanticSearchDownload}
                    >
                      {#if startingSemanticSearchDownload}
                        <ButtonSpinner />Starting
                      {:else}
                        {picked.approxDownloadBytes != null
                          ? `Download (${formatBytes(picked.approxDownloadBytes)})`
                          : "Download"}
                      {/if}
                    </button>
                  {:else if !selected}
                    <!-- Step 2: use (installed, not yet active). -->
                    <button
                      type="button"
                      class="btn btn--primary btn--sm"
                      onclick={() => void chooseSemanticSearchPickedModel(picked)}
                      disabled={semanticSearchReindexing}
                      aria-busy={semanticSearchReindexing}
                    >
                      {#if semanticSearchReindexing}<ButtonSpinner />Re-indexing{:else}Use this model{/if}
                    </button>
                    <!-- Installed but not active = unused: allow reclaiming its disk. -->
                    <button
                      type="button"
                      class="btn btn--ghost btn--sm"
                      onclick={() => void deleteSemanticSearchPickedModel(picked)}
                      disabled={deletingSemanticSearchModel}
                      aria-busy={deletingSemanticSearchModel}
                    >
                      {#if deletingSemanticSearchModel}<ButtonSpinner />Deleting{:else}Delete{/if}
                    </button>
                  {/if}
                  {#if !installed && !downloading}
                    <span class="action-hint">Step 1: download · Step 2: use this model</span>
                  {/if}
                </div>
              {/if}
            </div>
          {/if}

          <ModelFootprintHint byteSize={semanticSearchPickedModel?.approxDownloadBytes ?? null} />
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

  /* Picked-model header lays its label/description column opposite the status
     badge; override the shared .row-actions (which packs to the end). */
  .row-actions.row-actions--between {
    justify-content: space-between;
    align-items: flex-start;
  }

</style>
