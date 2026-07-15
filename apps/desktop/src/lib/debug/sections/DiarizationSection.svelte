<script lang="ts">
  // Diarization — speaker-analysis provider, model install state, and the
  // `speaker_analysis` job lane (mockup A).
  //
  // ponytail: the mockup's "clusters / voice prints" stats need counts no
  // command exposes today, so the grid carries the job lane instead — which is
  // what the section is for ("why is this segment not diarized?").

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    type DebugStat,
  } from "../format";

  const { capture, detail, features, health } = getDebugController();

  const severity = $derived(health.severityFor("diarization"));
  const settings = $derived(capture.recordingSettings?.speakerAnalysis ?? null);
  const lane = $derived(features.lane("speaker_analysis"));

  const selectedModel = $derived.by(() => {
    const providers = features.speakerModels?.providers;
    if (!providers || !settings) return null;
    const provider = providers.find((p) => p.provider === settings.provider);
    if (!provider) return null;
    return provider.models.find((m) => m.modelId === settings.modelId) ?? provider.models[0] ?? null;
  });

  const stats = $derived.by<DebugStat[]>(() => [
    { key: "queued", label: "Queued", value: lane.queued, isNew: true },
    { key: "running", label: "Running", value: lane.running, tone: lane.running > 0 ? "ok" : undefined },
    { key: "failed", label: "Failed", value: lane.failed, tone: lane.failed > 0 ? "err" : undefined },
    {
      key: "failed24h",
      label: "Failed (24h)",
      value: lane.failedLast24h,
      tone: lane.failedLast24h > 0 ? "warn" : undefined,
      sub: lane.averageCompletedSecondsLast24h != null
        ? `avg ${lane.averageCompletedSecondsLast24h.toFixed(1)}s`
        : null,
    },
  ]);
</script>

<SettingGroup
  title="Diarization"
  hint="processor: speaker_analysis"
  hintInline
  id={anchor("diarization")}
  onTitleClick={() => detail.open("diarization")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("diarization") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
    <span class="new-chip">new</span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={features.loadModelStatuses}
      disabled={features.loadingModels}
      aria-label="Refresh diarization model state"
      use:tip={"Refresh model state"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={features.loadingModels} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Provider</div>
      <div class="row__desc row__desc--mono">{settings?.provider ?? "—"}</div>
    </div>
    <div class="row__value">
      {#if settings && !settings.separateSpeakers}
        <span class="badge badge--neutral">disabled</span>
      {/if}
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Model</div>
      <div class="row__desc row__desc--mono">{selectedModel?.displayName ?? settings?.modelId ?? "—"}</div>
    </div>
    <div class="row__value">
      {#if selectedModel}
        <span class={selectedModel.available ? "badge badge--ok" : "badge badge--warn"}>
          {selectedModel.status.replace(/_/g, " ")}
        </span>
      {/if}
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Recognize saved people</div>
      <div class="row__desc">match clusters against enrolled voice prints</div>
    </div>
    <div class="row__value">
      <span class={settings?.recognizeSavedPeople ? "badge badge--ok" : "badge badge--neutral"}>
        {settings?.recognizeSavedPeople ? "on" : "off"}
      </span>
    </div>
  </div>

  {#if selectedModel?.failureMessage}
    <p class="debug-errline" role="status" aria-live="polite">{selectedModel.failureMessage}</p>
  {/if}
  {#if features.modelsError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.modelsError}</p>
  {/if}
  {#if features.lanesError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.lanesError}</p>
  {:else if lane.lastError}
    <p class="debug-errline" role="status" aria-live="polite">{lane.lastError}</p>
  {/if}

  <StatGrid {stats} />
</SettingGroup>
