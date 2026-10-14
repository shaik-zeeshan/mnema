<script lang="ts">
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ModelDownloadRow from "$lib/settings/ui/ModelDownloadRow.svelte";
  import { downloadMeta } from "$lib/settings/ui/model-download";
  import IconInfo from "~icons/lucide/info";
  import IconAlert from "~icons/lucide/triangle-alert";

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

  // Controller derived selectors.
  const semanticSearchModelOptions = $derived(c.semanticSearchModelOptions);
  const picked = $derived(c.semanticSearchPickedModel);
  const progress = $derived(c.semanticSearchPickedProgress);
  const selected = $derived(!!picked && rec.semanticSearchSelectedModelId === picked.modelId);
  const downloading = $derived(
    !!progress && ["starting", "downloading", "installing"].includes(progress.status),
  );
  const failed = $derived(!!semanticSearchDownloadError || progress?.status === "failed");
  const hasNotes = $derived(
    osIsNonEnglish
      || !!semanticSearchSupportedModelsError
      || (loadingSemanticSearchSupportedModels && semanticSearchSupportedModels.length === 0),
  );
  const reindexing = $derived(selected && (semanticSearchReindexing || !!semanticSearchReindexMessage));

  // Controller / store action methods.
  const loadSemanticSearchModelStatus = () => c.loadSemanticSearchModelStatus();
  const setSemanticSearchEnabled = (value: boolean) => c.setSemanticSearchEnabled(value);
  const cancelSemanticSearchModelDownload = () => c.cancelSemanticSearchModelDownload();
</script>

<SettingGroup
  id="settings-section-semanticSearch"
  title="Semantic Search"
  hint="Meaning-based search, fully on-device."
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
    description="Fuse meaning-based results with keyword search. Inert until a model below is installed; indexes in the background, which costs CPU and battery while it catches up."
  >
    {#snippet control()}
      <Switch
        ariaLabel="Enable semantic search"
        checked={rec.draftSemanticSearchEnabled}
        onCheckedChange={(value) => void setSemanticSearchEnabled(value)}
      />
    {/snippet}
  </SettingRow>

  {#if !semanticSearchModelStatus}
    <ModelDownloadRow
      label="Model"
      description={loadingSemanticSearchModelStatus ? "Checking installed search models…" : undefined}
      error={semanticSearchModelError ? `Model status failed: ${semanticSearchModelError}` : null}
      warning={loadingSemanticSearchModelStatus || semanticSearchModelError ? null : "No search model status is available."}
    />
  {:else if !picked}
    <ModelDownloadRow
      label="Model"
      description="Recommended tiers first. Nothing downloads until you choose one."
      error={semanticSearchModelError ? `Model status failed: ${semanticSearchModelError}` : null}
      controls={picker}
      note={hasNotes ? notes : undefined}
    />
  {:else if downloading && progress}
    <ModelDownloadRow
      label="Model"
      description={picked.displayName}
      progress={downloadMeta(picked.displayName, progress)}
      onCancel={() => void cancelSemanticSearchModelDownload()}
      cancelling={cancellingSemanticSearchDownload}
      cancelLabel="Cancel"
    />
  {:else if reindexing}
    <ModelDownloadRow
      label="Model"
      chip={{ text: "active", tone: "accent" }}
      description={semanticSearchReindexMessage ?? "Re-indexing — clearing existing vectors…"}
      progress={{ title: "Re-indexing", detail: "catching up", percent: null }}
      controls={picker}
    />
  {:else}
    <ModelDownloadRow
      label="Model"
      chip={!picked.available
        ? failed
          ? { text: "failed", tone: "danger" }
          : { text: "not installed" }
        : selected
          ? { text: "active", tone: "accent" }
          : { text: "installed" }}
      description={`${picked.metaLine}.${!picked.available && !failed ? " Step 1: download · Step 2: use this model." : ""}`}
      error={semanticSearchModelError
        ? `Model status failed: ${semanticSearchModelError}`
        : semanticSearchDownloadError
          ? `Download failed: ${semanticSearchDownloadError}`
          : null}
      controls={pickedActions}
      note={hasNotes ? notes : undefined}
      download={!picked.available
        ? {
            bytes: picked.approxDownloadBytes,
            starting: startingSemanticSearchDownload,
            onclick: () => void c.startSemanticSearchPickedDownload(picked),
            disabled: !picked.provider,
          }
        : null}
      folder={{ path: semanticSearchModelStatus.modelsDirectory, reveal: picked.available ? picked.installPath : null }}
    />
  {/if}
</SettingGroup>

{#snippet notes()}
  {#if osIsNonEnglish}
    <span class="mx-inline" data-tone="info"><IconInfo width="13" height="13" />Your system language ({osLocale}) isn’t English — the Multilingual tier is recommended.</span>
  {/if}
  {#if semanticSearchSupportedModelsError}
    <span class="mx-inline" data-tone="warn"><IconAlert width="13" height="13" />Custom model list failed: {semanticSearchSupportedModelsError}</span>
  {:else if loadingSemanticSearchSupportedModels && semanticSearchSupportedModels.length === 0}
    <span class="mx-inline">Loading supported models…</span>
  {/if}
{/snippet}

{#snippet pickedActions()}
  {@render picker()}
  {#if picked && picked.available && !selected}
    <!-- Step 2: use (installed, not yet active). Never automatic (ADR 0036). -->
    <button
      type="button"
      class="mx-btn"
      onclick={() => void c.chooseSemanticSearchPickedModel(picked)}
      disabled={semanticSearchReindexing}
      aria-busy={semanticSearchReindexing}
    >
      {#if semanticSearchReindexing}<ButtonSpinner />Re-indexing{:else}Use this model{/if}
    </button>
    <!-- Installed but not active = unused: allow reclaiming its disk. -->
    <button
      type="button"
      class="mx-btn mx-btn--ghost"
      onclick={() => void c.deleteSemanticSearchPickedModel(picked)}
      disabled={deletingSemanticSearchModel}
      aria-busy={deletingSemanticSearchModel}
    >
      {#if deletingSemanticSearchModel}<ButtonSpinner />Deleting{:else}Delete{/if}
    </button>
  {/if}
{/snippet}

{#snippet picker()}
  <div class="ss-ctl">
    <Combobox
      placeholder="Select a model…"
      searchPlaceholder="Search models…"
      value={c.semanticSearchPickedModelId}
      onValueChange={(v) => (c.semanticSearchPickedModelId = v)}
      options={semanticSearchModelOptions}
    />
  </div>
{/snippet}

<style>
  /* Right-aligned picker keeps a readable width (.st-select). */
  .ss-ctl {
    width: 260px;
  }
</style>
