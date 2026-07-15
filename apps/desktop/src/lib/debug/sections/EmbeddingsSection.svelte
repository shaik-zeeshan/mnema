<script lang="ts">
  // Embeddings — the "why is semantic search degraded?" card: model state,
  // index coverage, vector count, backlog, quarantine, and the last model-load
  // error (mockup A).
  //
  // The worker half (`modelLoaded`, load failures, quarantine, `lastLoadError`)
  // comes from the shared snapshot the sweep publishes each pass; the counts are
  // live DB reads. Both arrive in one `get_semantic_index_status`.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    formatCount,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    type DebugStat,
  } from "../format";

  const { capture, detail, features, health } = getDebugController();

  const severity = $derived(health.severityFor("embeddings"));
  const settings = $derived(capture.recordingSettings?.semanticSearch ?? null);
  const index = $derived(features.semanticIndex);

  const selectedModel = $derived.by(() => {
    const models = features.semanticModels?.models;
    if (!models || !settings) return null;
    return models.find((m) => m.provider === settings.provider && m.modelId === settings.modelId) ?? null;
  });

  /** The model row's description: only the parts we actually know. */
  const modelDesc = $derived.by(() => {
    const parts: string[] = [];
    if (settings?.provider) parts.push(settings.provider);
    if (index?.liveDimension != null) parts.push(`${index.liveDimension}-dim`);
    return parts.length === 0 ? null : parts.join(" · ");
  });

  /**
   * `modelLoaded: false` is NOT a fault — the worker drops the embedder after an
   * idle grace period and reloads it on the next anchor. Only a load *failure*
   * is a fault, so the two are reported as distinct states.
   */
  const workerState = $derived.by(() => {
    if (!index) return { word: "unknown", cls: "badge badge--neutral", why: "worker snapshot not read yet" };
    if ((index.consecutiveLoadFailures ?? 0) > 0) {
      return {
        word: "stalled",
        cls: "badge badge--err",
        why: `${index.consecutiveLoadFailures} consecutive load failures — backfill cannot make progress`,
      };
    }
    if (index.modelLoaded) return { word: "loaded", cls: "badge badge--ok", why: "the embedder is warm" };
    return { word: "idle", cls: "badge badge--neutral", why: "embedder unloaded after the idle grace period — normal" };
  });

  /**
   * Indexed share of everything that wants a vector. `null` on a fresh install
   * (0 vectors, 0 backlog) — there is nothing to be a percentage *of*, and
   * `0/0` would render `NaN%`.
   */
  const coverage = $derived.by(() => {
    if (!index) return null;
    const total = index.vectorCount + index.backlogCount;
    if (total === 0) return null;
    return { total, pct: (index.vectorCount / total) * 100 };
  });

  const stats = $derived.by<DebugStat[]>(() => [
    { key: "vectors", label: "Vectors", value: formatCount(index?.vectorCount) },
    {
      key: "backlog",
      label: "Backlog",
      value: formatCount(index?.backlogCount),
      sub: "anchors w/o vector",
      tone: (index?.backlogCount ?? 0) > 0 ? "warn" : undefined,
      isNew: true,
    },
    {
      key: "quarantined",
      label: "Quarantined",
      value: formatCount(index?.quarantinedCount ?? 0),
      // In-memory only: a restart clears it, so the label must say so.
      sub: "since app start",
      tone: (index?.quarantinedCount ?? 0) > 0 ? "warn" : undefined,
      isNew: true,
    },
    {
      key: "dimension",
      label: "Dimension",
      value: index?.liveDimension ?? "—",
      sub: index && index.liveDimension == null ? "index table absent" : null,
      tone: index && index.liveDimension == null ? "err" : undefined,
    },
  ]);
</script>

<SettingGroup
  title="Embeddings"
  hint="semantic index · backfill worker"
  hintInline
  id={anchor("embeddings")}
  onTitleClick={() => detail.open("embeddings")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("embeddings") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
    <span class="new-chip">new</span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={features.loadModelStatuses}
      disabled={features.loadingModels}
      aria-label="Refresh embedding model state"
      use:tip={"Refresh model state"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={features.loadingModels} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Model</div>
      <div class="row__desc row__desc--mono">
        {selectedModel?.displayName ?? settings?.modelId ?? "none selected"}{#if modelDesc}
          · {modelDesc}{/if}
      </div>
    </div>
    <div class="row__value">
      {#if selectedModel}
        <span class={selectedModel.available ? "badge badge--ok" : "badge badge--warn"}>
          {selectedModel.status.replace(/_/g, " ")}
        </span>
      {/if}
      {#if settings && !settings.enabled}
        <span class="badge badge--neutral">disabled</span>
      {/if}
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Backfill worker <span class="new-chip">new</span></div>
      <div class="row__desc">{workerState.why}</div>
    </div>
    <div class="row__value">
      <span class={workerState.cls}>{workerState.word}</span>
    </div>
  </div>

  <!-- Index coverage. Hidden entirely on a fresh install rather than shown as
       an empty or NaN bar — nothing is indexed and nothing is waiting, so there
       is no progress to report. -->
  {#if coverage}
    <div class="bar-caption">
      <span>index coverage</span>
      <span>{formatCount(index?.vectorCount)} / {formatCount(coverage.total)} · {coverage.pct.toFixed(1)}%</span>
    </div>
    <div class="bar">
      <div class="bar__fill" style="width: {coverage.pct}%"></div>
    </div>
  {/if}

  {#if features.modelsError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.modelsError}</p>
  {/if}
  {#if features.semanticIndexError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.semanticIndexError}</p>
  {:else if index?.lastLoadError}
    <p class="debug-errline" role="alert" aria-live="polite">
      {index.lastLoadError}
      <span class="debug-errline__meta">consecutive load failures: {index.consecutiveLoadFailures ?? 0}</span>
    </p>
  {/if}

  <StatGrid {stats} />
</SettingGroup>
