// Semantic-search picker view for the settings controller.
//
// The page picker's draft selection + its derivations (guided/custom model
// options, the picked-model view, in-flight download progress) and the
// load/choose/enable actions live here so the main `controller.svelte.ts`
// stays under the 800-line file cap. Like `controller-processing.svelte.ts`,
// this is a behavior-preserving re-home: a FACTORY (not a class) so the
// `rec`/`models` references are closure variables defined before any `$derived`
// (class `$derived` field initializers run before the constructor body could
// assign the store refs, tripping "used before initialization"). The controller
// composes one instance and re-exposes its members so panel markup references
// stay flat (`c.semanticSearchPickedModel`, etc.) and verbatim.

import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { errorText, formatBytes } from "./format";
import { semanticSearchTierLabel } from "./models-format";
import type { RecordingStore } from "./recording.svelte";
import type { createModelStatusStore } from "./model-status.svelte";
import type {
  RecordingSettingsDomainUpdateResponse,
  SemanticSearchModelStatus,
  SemanticSearchModelDownloadProgress,
} from "$lib/types";

type ModelStatusStore = ReturnType<typeof createModelStatusStore>;

export interface SemanticSearchPickedView {
  modelId: string;
  provider: string | null;
  displayName: string;
  description: string;
  metaLine: string;
  available: boolean;
  approxDownloadBytes: number | null;
}

export function createSemanticSearchView(rec: RecordingStore, models: ModelStatusStore) {
  // Semantic-search picked model (page picker draft).
  let semanticSearchPickedModelId = $state<string | null>(null);

  // Seed the page picker from the persisted (sticky) selection, but ONLY while
  // the picker has not been touched (`semanticSearchPickedModelId === null`), so
  // a live user edit is never clobbered. Idempotent — safe to call from every
  // path that might learn the persisted selection (status load, download
  // progress, or a post-settings-load re-seed that fixes the init race where the
  // picker status resolved before recording settings).
  function reseedSemanticSearchPickedModel() {
    if (semanticSearchPickedModelId === null && rec.semanticSearchSelectedModelId !== null) {
      semanticSearchPickedModelId = rec.semanticSearchSelectedModelId;
    }
  }

  async function loadSemanticSearchModelStatus() {
    await models.loadSemanticSearchModelStatus();
    reseedSemanticSearchPickedModel();
  }

  async function handleSemanticSearchDownloadProgress(progress: SemanticSearchModelDownloadProgress) {
    await models.handleSemanticSearchDownloadProgress(progress);
    reseedSemanticSearchPickedModel();
  }

  async function chooseSemanticSearchModel(model: SemanticSearchModelStatus) {
    // In-flight re-entry guard (mirrors `saveRecordingDomain`'s `savingRecDomains`
    // gate). The confirm() dialog below awaits, so without this a second invocation
    // while a `select_semantic_search_model` invoke is in flight would stack a
    // second clear/reindex. Correctness must not depend solely on the UI `disabled`.
    if (models.semanticSearchReindexing) return;
    if (!rec.recordingSettingsLoaded) await rec.loadRecordingSettings();
    if (rec.semanticSearchSelectedModelId === model.modelId) return;

    // Arm the in-flight guard BEFORE the (awaited) confirm dialog — same as
    // `saveRecordingDomain` arms `savingRecDomains[domain]` before its retention
    // preview + confirm. The earlier check at the top is checked while the flag is
    // still false through the whole dialog, so two rapid selections could both pass
    // and stack two clear/reindex passes. Setting it here closes that window; the
    // single `finally` below always clears it on every early-return path (including
    // the cancel path).
    models.semanticSearchReindexing = true;
    try {
      const isFirstSelection = rec.semanticSearchSelectedModelId === null;
      if (!isFirstSelection) {
        const confirmed = await confirm(
          `Switching to “${model.displayName}” re-indexes every recording: all existing meaning vectors are cleared and re-derived under the new model in the background. Your captures are not changed.`,
          {
            title: "Re-index for new search model?",
            kind: "warning",
            okLabel: "Switch & Re-index",
            cancelLabel: "Keep Current Model",
          },
        );
        if (!confirmed) return;
      }

      models.semanticSearchModelError = null;
      models.semanticSearchReindexMessage = null;
      try {
        const cleared = await invoke<number>("select_semantic_search_model", {
          modelId: model.modelId,
        });
        rec.semanticSearchSelectedModelId = model.modelId;
        if (!isFirstSelection) {
          models.semanticSearchReindexMessage =
            cleared > 0
              ? `Cleared ${cleared} vector${cleared === 1 ? "" : "s"}; re-indexing in the background.`
              : "Re-index started in the background.";
        }
        await loadSemanticSearchModelStatus();
      } catch (err) {
        models.semanticSearchModelError = errorText(err);
      }
    } finally {
      models.semanticSearchReindexing = false;
    }
  }

  async function setSemanticSearchEnabled(enabled: boolean) {
    models.semanticSearchModelError = null;
    try {
      await invoke<RecordingSettingsDomainUpdateResponse>("update_semantic_search_settings", {
        request: { enabled },
      });
      rec.draftSemanticSearchEnabled = enabled;
    } catch (err) {
      models.semanticSearchModelError = errorText(err);
      rec.draftSemanticSearchEnabled = !enabled;
    }
  }

  // ─── Semantic-search picker derivations ─────────────────────────────────────
  const semanticSearchGuidedModels = $derived(
    (models.semanticSearchModelStatus?.models ?? []).filter((m) => m.tier !== "custom"),
  );
  const semanticSearchProvider = $derived(
    (models.semanticSearchModelStatus?.models ?? [])[0]?.provider ?? null,
  );
  const semanticSearchGuidedModelIds = $derived(
    new Set(semanticSearchGuidedModels.map((m) => m.modelId)),
  );
  const semanticSearchCustomOptions = $derived(
    models.semanticSearchSupportedModels.filter(
      (m) => !semanticSearchGuidedModelIds.has(m.modelId),
    ),
  );
  const semanticSearchModelOptions = $derived([
    ...semanticSearchGuidedModels.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} · ${m.dimension}d${m.tier === "multilingual" ? " · multilingual" : ""} · recommended`,
    })),
    ...semanticSearchCustomOptions.map((m) => ({
      value: m.modelId,
      label: `${m.displayName} — ${m.dimension}d${m.multilingual ? " · multilingual" : ""}`,
    })),
  ]);

  const semanticSearchPickedModel = $derived.by((): SemanticSearchPickedView | null => {
    const id = semanticSearchPickedModelId;
    if (!id) return null;
    const live = (models.semanticSearchModelStatus?.models ?? []).find((m) => m.modelId === id);
    if (live) {
      return {
        modelId: live.modelId,
        provider: live.provider,
        displayName: live.displayName,
        description: live.description,
        metaLine: `${semanticSearchTierLabel(live.tier)} · ${formatBytes(live.approxDownloadBytes)} on disk · ${live.dimension}-dim · runs on-device${live.licenseLabel ? ` · ${live.licenseLabel}` : ""}`,
        available: live.available,
        approxDownloadBytes: live.approxDownloadBytes,
      };
    }
    const catalog = models.semanticSearchSupportedModels.find((m) => m.modelId === id);
    if (catalog) {
      const size =
        catalog.approxDownloadBytes != null
          ? `${formatBytes(catalog.approxDownloadBytes)} on disk · `
          : "";
      return {
        modelId: catalog.modelId,
        provider: semanticSearchProvider,
        displayName: catalog.displayName,
        description: catalog.description,
        metaLine: `${semanticSearchTierLabel("custom")} · ${size}${catalog.dimension}-dim · runs on-device${catalog.multilingual ? " · multilingual" : ""}`,
        available: false,
        approxDownloadBytes: catalog.approxDownloadBytes,
      };
    }
    return null;
  });

  const semanticSearchPickedProgress = $derived.by(() => {
    const id = semanticSearchPickedModelId;
    const p = models.semanticSearchDownloadProgress;
    return id && p && p.modelId === id ? p : null;
  });

  async function startSemanticSearchPickedDownload(model: SemanticSearchPickedView) {
    if (!model.provider) return;
    await models.startSemanticSearchModelDownload({
      provider: model.provider,
      modelId: model.modelId,
    } as SemanticSearchModelStatus);
  }

  async function chooseSemanticSearchPickedModel(model: SemanticSearchPickedView) {
    await chooseSemanticSearchModel({
      modelId: model.modelId,
      displayName: model.displayName,
    } as SemanticSearchModelStatus);
  }

  return {
    get semanticSearchPickedModelId() {
      return semanticSearchPickedModelId;
    },
    set semanticSearchPickedModelId(value: string | null) {
      semanticSearchPickedModelId = value;
    },
    reseedSemanticSearchPickedModel,
    loadSemanticSearchModelStatus,
    handleSemanticSearchDownloadProgress,
    chooseSemanticSearchModel,
    setSemanticSearchEnabled,
    startSemanticSearchPickedDownload,
    chooseSemanticSearchPickedModel,
    get semanticSearchModelOptions() {
      return semanticSearchModelOptions;
    },
    get semanticSearchPickedModel() {
      return semanticSearchPickedModel;
    },
    get semanticSearchPickedProgress() {
      return semanticSearchPickedProgress;
    },
  };
}

export type SemanticSearchView = ReturnType<typeof createSemanticSearchView>;
