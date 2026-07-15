<script lang="ts">
  // Transcription — provider, auth state, the `audio_transcription` job lane and
  // its last error (mockup A).
  //
  // The card title pushes the feature detail (jobs table + per-job inspector,
  // where the per-segment retry lives). There is still no bulk "reprocess failed
  // segments" command (only per-segment `reprocess_audio_segment_transcription`),
  // so that mockup button is deliberately absent rather than faked.

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

  const severity = $derived(health.severityFor("transcription"));
  const settings = $derived(capture.recordingSettings?.transcription ?? null);
  const lane = $derived(features.lane("audio_transcription"));
  const isDeepgram = $derived(settings?.provider === "deepgram");

  /** The selected model's install state, from the provider→models status tree. */
  const selectedModel = $derived.by(() => {
    const providers = features.transcriptionModels?.providers;
    if (!providers || !settings) return null;
    const provider = providers.find((p) => p.provider === settings.provider);
    if (!provider) return null;
    // A provider whose models carry no id (Apple Speech, Deepgram) has exactly
    // one entry standing for the provider itself.
    return provider.models.find((m) => m.modelId === settings.modelId) ?? provider.models[0] ?? null;
  });

  const providerDesc = $derived.by(() => {
    if (!settings) return null;
    const parts = [settings.provider, isDeepgram ? "cloud · audio leaves the device" : "on-device"];
    return parts.join(" · ");
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
  title="Transcription"
  hint="processor: audio_transcription"
  hintInline
  id={anchor("transcription")}
  onTitleClick={() => detail.open("transcription")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("transcription") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
    <span class="new-chip">new</span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={features.loadConfig}
      disabled={features.loadingModels}
      aria-label="Refresh transcription config"
      use:tip={"Refresh provider + model state"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={features.loadingModels} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Provider</div>
      <div class="row__desc row__desc--mono">{providerDesc ?? "—"}</div>
    </div>
    <div class="row__value">
      {#if isDeepgram}
        <span class="badge badge--running" use:tip={"Cloud provider — audio leaves the device (ADR 0047)."}>cloud</span>
      {:else if settings}
        <span class="badge badge--ok">on-device</span>
      {/if}
      {#if settings && !settings.enabled}
        <span class="badge badge--neutral">disabled</span>
      {/if}
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Model</div>
      <div class="row__desc row__desc--mono">{settings?.modelId ?? selectedModel?.displayName ?? "—"}</div>
    </div>
    <div class="row__value">
      {#if selectedModel}
        <span class={selectedModel.available ? "badge badge--ok" : "badge badge--warn"}>
          {selectedModel.status.replace(/_/g, " ")}
        </span>
      {/if}
    </div>
  </div>

  {#if isDeepgram}
    <div class="row">
      <div class="row__main">
        <div class="row__label">API key</div>
        <div class="row__desc">stored in the OS keychain under <code>transcription.deepgram</code></div>
      </div>
      <div class="row__value">
        <span class={features.deepgramKeyPresent ? "badge badge--ok" : "badge badge--err"}>
          {features.deepgramKeyPresent ? "in keychain" : "missing"}
        </span>
      </div>
    </div>
    <div class="row">
      <div class="row__main">
        <div class="row__label">API auth</div>
        <div class="row__desc row__desc--mono">{features.deepgramAuthStatus ?? "not probed"}</div>
      </div>
      <div class="row__value">
        <button class="btn btn--ghost btn--sm" onclick={features.testDeepgram} disabled={features.testingDeepgram}>
          {features.testingDeepgram ? "…" : "test connection"}
        </button>
      </div>
    </div>
  {/if}

  {#if features.deepgramError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.deepgramError}</p>
  {/if}
  {#if features.deepgramTestResult}
    <p class={features.deepgramTestResult.ok ? "debug-note" : "debug-errline"} role="status" aria-live="polite">
      {features.deepgramTestResult.message}
    </p>
  {/if}
  {#if features.modelsError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.modelsError}</p>
  {/if}

  {#if features.lanesError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.lanesError}</p>
  {:else if lane.lastError}
    <!-- A warnline, not an errline: Deepgram connect/auth failures requeue
         without burning an attempt — transient liveness, not a job failure
         (ADR 0048), so it must not be painted as a hard error. -->
    <p class="debug-warnline" role="status" aria-live="polite">{lane.lastError}</p>
  {/if}

  <StatGrid {stats} />
</SettingGroup>
