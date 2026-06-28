<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import Combobox from "$lib/components/Combobox.svelte";
  import { formatBytes } from "./onboarding-mapping";

  let { controller }: { controller: OnboardingController } = $props();

  // Mirrors OcrBody: the Combobox selects the model (draft-only — committed at
  // finish, no live reindex), the card below reflects the resolved
  // `selectedSemanticSearchModel` and drives the in-app download. The status
  // pill reuses the mockup's `.pill` family (granted/pending/denied).
  const model = $derived(controller.selectedSemanticSearchModel);

  const pillLabel = $derived.by(() => {
    if (!model) return "Not installed";
    if (model.available) return "Installed";
    if (controller.selectedSemanticSearchDownloadRunning) return "Downloading";
    return "Not installed";
  });

  const pillClass = $derived.by(() => {
    if (!model) return "pending";
    if (model.available) return "granted";
    if (controller.selectedSemanticSearchDownloadRunning) return "pending";
    return "denied";
  });
</script>

<div class="group">
  <div class="group-title">Model</div>
  <div class="ctl stack-field">
    <div class="ctl-label">
      <div class="name">Model</div>
      <div class="desc">
        Meaning-based search runs fully on-device. Inert until a model is installed — nothing is
        downloaded until you pick one. Recommended tiers are listed first.
      </div>
    </div>
    <div class="ctl-field" style="width: 100%">
      <Combobox
        value={controller.draftSemanticSearchModelId ?? ""}
        onValueChange={(v) => controller.chooseSemanticSearchModel(v)}
        placeholder="Select a model — recommended tiers first…"
        searchPlaceholder="Search models…"
        options={controller.semanticSearchModelOptions.length > 0
          ? controller.semanticSearchModelOptions
          : [{ value: controller.draftSemanticSearchModelId ?? "", label: "Loading model options" }]}
      />
    </div>
  </div>

  {#if controller.semanticSearchModelError}
    <div class="note">
      <div>Failed to load search model status: {controller.semanticSearchModelError}</div>
      <div class="dl-meta">
        <span>This is a fetch error, not a missing model — retry to recheck.</span>
        <button
          type="button"
          class="btn sm"
          disabled={controller.loadingSemanticSearchModelStatus}
          onclick={() => controller.loadSemanticSearchModelStatus()}
        >
          {controller.loadingSemanticSearchModelStatus ? "Retrying…" : "Retry"}
        </button>
      </div>
    </div>
  {:else if controller.semanticSearchSupportedModelsError}
    <div class="note">
      <div>Custom model list failed: {controller.semanticSearchSupportedModelsError}</div>
      <div class="dl-meta">
        <span>Retry to reload the custom model catalog.</span>
        <button
          type="button"
          class="btn sm"
          disabled={controller.loadingSemanticSearchSupportedModels}
          onclick={() => controller.loadSemanticSearchSupportedModels()}
        >
          {controller.loadingSemanticSearchSupportedModels ? "Retrying…" : "Retry"}
        </button>
      </div>
    </div>
  {:else if model}
    <div class="model-card">
      <div class="model-top">
        <div class="model-id">
          {model.displayName}
          <div class="meta">{model.metaLine}{model.description ? ` · ${model.description}` : ""}</div>
        </div>
        <span class="pill {pillClass}">
          <span class="d"></span>{pillLabel}
        </span>
      </div>

      {#if controller.selectedSemanticSearchDownloadRunning}
        <div class="dl">
          <div class="dl-track">
            <div
              class="dl-fill"
              class:dl-fill--indeterminate={controller.selectedSemanticSearchDownloadPercent === null}
              style={controller.selectedSemanticSearchDownloadPercent === null
                ? undefined
                : `width: ${controller.selectedSemanticSearchDownloadPercent}%`}
            ></div>
          </div>
          <div class="dl-meta">
            <span>
              <b>{controller.selectedSemanticSearchDownloadPercent === null
                  ? "…"
                  : `${controller.selectedSemanticSearchDownloadPercent}%`}</b>
              · {controller.selectedSemanticSearchDownloadProgress?.status ?? "downloading"}
            </span>
            <button
              type="button"
              class="btn sm danger"
              disabled={controller.cancellingSemanticSearchDownload}
              onclick={() => controller.cancelSelectedSemanticSearchModelDownload()}
            >
              {controller.cancellingSemanticSearchDownload ? "Cancelling…" : "Cancel"}
            </button>
          </div>
        </div>
      {:else}
        <div class="dl-meta">
          <span>
            {model.available ? "Downloaded" : "Not downloaded"}{model.approxDownloadBytes != null
              ? ` · ${formatBytes(model.approxDownloadBytes)}`
              : ""}
          </span>
          <button
            type="button"
            class="btn sm accent"
            disabled={controller.startingSemanticSearchDownload || model.available || !model.provider}
            onclick={() => controller.startSelectedSemanticSearchModelDownload()}
          >
            {controller.startingSemanticSearchDownload ? "Starting…" : "Download"}
          </button>
        </div>
        {#if !model.available && !model.provider}
          <!-- Explain the otherwise-silent disabled Download: this model has no
               source to fetch from, so point the user at another one. -->
          <div class="note muted">No download source for this model — pick another above.</div>
        {/if}
      {/if}
      {#if controller.semanticSearchDownloadError}
        <div class="note">Download failed: {controller.semanticSearchDownloadError}</div>
      {/if}
    </div>
  {:else if controller.loadingSemanticSearchModelStatus}
    <div class="note muted">Checking installed search models…</div>
  {:else}
    <div class="note">Pick a model above to activate semantic search.</div>
  {/if}

  <div class="note muted">
    Mnema embeds your captures in the background once a model is installed — it keeps re-embedding
    new captures (ongoing CPU/GPU while it catches up). You can change or download a model later in
    Settings.
  </div>
</div>
