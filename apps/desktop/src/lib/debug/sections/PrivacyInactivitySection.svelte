<script lang="ts">
  // Privacy & Inactivity — merges the legacy "Capture" tab's Privacy Filter card
  // with the whole "Inactivity" tab. Both read the same two live snapshots
  // (get_capture_privacy_debug / get_idle_debug) and both answer "why is capture
  // not recording what I expect", so they share one section.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import IconMonitor from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import IconVolume2 from "~icons/lucide/volume-2";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    formatActivityMode,
    formatDebugList,
    formatEffectiveSource,
    formatIdleMs,
    formatTimestamp,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    sourceDecisionSummary,
    sourceKindLabel,
  } from "../format";

  const { capture, health } = getDebugController();

  const severity = $derived(health.severityFor("privacy"));
</script>

<SettingGroup
  title="Privacy & Inactivity"
  hint="privacy filter · idle policy"
  hintInline
  id={anchor("privacy")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("privacy") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={capture.refreshPrivacyFilter}
      disabled={capture.loadingPrivacyFilter || !capture.isCapturing}
      aria-label="Refresh privacy filter"
      use:tip={capture.isCapturing ? "Refresh privacy filter" : "No active capture session — start recording to refresh"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={capture.loadingPrivacyFilter} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <!-- ── Privacy filter ────────────────────────────────────────────────── -->
  {#if capture.privacyDebug}
    {@const privacyDebug = capture.privacyDebug}
    <div class="row">
      <div class="row__main">
        <div class="row__label">Capture filter</div>
        <div class="row__desc">last successfully applied ScreenCaptureKit exclusions</div>
      </div>
      <div class="row__value">
        <span class={privacyDebug.privacyDebug.privacyFilterApplied ? "badge badge--warn" : "badge badge--ok"}>
          {privacyDebug.privacyDebug.privacyFilterApplied ? "active" : "empty"}
        </span>
      </div>
    </div>
    <div class="row">
      <div class="row__main">
        <div class="row__label">Excluded apps</div>
        <div class="row__desc row__desc--mono">{formatDebugList(privacyDebug.privacyDebug.currentlyExcludedBundleIds)}</div>
      </div>
      <div class="row__value">
        <span class="badge badge--neutral">{privacyDebug.privacyDebug.currentlyExcludedBundleIds.length}</span>
      </div>
    </div>
    <div class="row">
      <div class="row__main">
        <div class="row__label">Metadata</div>
        <div class="row__desc row__desc--mono">
          URL {privacyDebug.browserUrlMode} · {privacyDebug.browserUrlMetadataSource}
        </div>
      </div>
      <div class="row__value">
        <span class={privacyDebug.metadataEnabled ? "badge badge--ok" : "badge badge--neutral"}>
          {privacyDebug.metadataEnabled ? "enabled" : "disabled"}
        </span>
      </div>
    </div>
  {:else}
    <!-- No snapshot: the filter row still states its own absence, calmly, rather
         than an italic aside floating in an otherwise empty body. -->
    <div class="row">
      <div class="row__main">
        <div class="row__label">Capture filter</div>
        <div class="row__desc">last successfully applied ScreenCaptureKit exclusions</div>
      </div>
      <div class="row__value">
        <span class="dim">{capture.isCapturing ? "not loaded" : "session inactive"}</span>
      </div>
    </div>
  {/if}

  {#if capture.privacyDebugError}
    <p class="debug-errline" role="alert" aria-live="polite">{capture.privacyDebugError}</p>
  {:else if capture.privacyDebug}
    {@const privacyDebug = capture.privacyDebug}
    <details class="disclosure">
      <summary class="row row--head">
        <div class="row__main">
          <div class="row__label">Raw decisions &amp; snapshot</div>
          <div class="row__desc">evaluated · applied · latest metadata</div>
        </div>
        <div class="row__value"><span class="disclosure__chevron" aria-hidden="true">›</span></div>
      </summary>
      <div class="debug-body">
        <div class="privacy-debug-grid">
          <div>
            <div class="idle-section-label">evaluated decision</div>
            <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestDecision, null, 2)}</p>
          </div>
          <div>
            <div class="idle-section-label">applied decision</div>
            <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestAppliedDecision, null, 2)}</p>
          </div>
        </div>

        <div class="idle-section-label">latest metadata snapshot</div>
        {#if privacyDebug.privacyDebug.latestSnapshot}
          <p class="privacy-debug-json">{JSON.stringify(privacyDebug.privacyDebug.latestSnapshot, null, 2)}</p>
        {:else}
          <p class="empty">no metadata snapshot captured yet</p>
        {/if}
      </div>
    </details>
  {/if}

  <!-- ── Inactivity policy ─────────────────────────────────────────────── -->
  <div class="row row--head">
    <div class="row__main">
      <div class="row__label">Inactivity policy</div>
      <div class="row__desc">idle gating · per-detector pause state</div>
    </div>
    <div class="row__value">
      {#if !capture.idleDebug && !capture.idleDebugError}
        <span class="dim">{capture.isCapturing ? "not loaded" : "session inactive"}</span>
      {/if}
      <button
        class="btn btn--ghost btn--sm"
        onclick={capture.refreshInactivity}
        disabled={capture.loadingInactivity || !capture.isCapturing}
        aria-label="Refresh inactivity policy"
        use:tip={capture.isCapturing ? "Refresh inactivity policy" : "No active capture session — start recording to refresh"}
      >
        <span class="refresh-glyph" class:refresh-glyph--spin={capture.loadingInactivity} aria-hidden="true">↻</span>
      </button>
    </div>
  </div>

  {#if capture.idleDebugError}
    <p class="debug-errline" role="alert" aria-live="polite">{capture.idleDebugError}</p>
  {:else if capture.idleDebug}
    {@const idleDebug = capture.idleDebug}
    <div class="debug-body">
      <!-- ── Status row ──────────────────────────────────── -->
      <div class="idle-policy">
        <span class="idle-policy__item">
          <span class="idle-policy__k">gating</span>
          <span class={idleDebug.inactivityEnabled ? "badge badge--ok badge--sm" : "badge badge--neutral badge--sm"}>
            {idleDebug.inactivityEnabled ? "enabled" : "disabled"}
          </span>
        </span>
        <span class="idle-policy__item">
          <span class="idle-policy__k">any paused</span>
          <span class={idleDebug.isInactivityPaused ? "badge badge--warn badge--sm" : "badge badge--neutral badge--sm"}>
            {idleDebug.isInactivityPaused ? "yes" : "no"}
          </span>
        </span>
        <span class="idle-policy__item">
          <span class="idle-policy__k">timeout</span>
          <span class="badge badge--neutral badge--sm">{idleDebug.inactivityEnabled ? `${idleDebug.idleTimeoutSeconds}s` : "—"}</span>
        </span>
        <span class="idle-policy__item">
          <span class="idle-policy__k">mode</span>
          <span class="badge badge--neutral badge--sm">{formatActivityMode(idleDebug.activityMode)}</span>
        </span>
      </div>

      <!-- ── Signal readings ────────────────────────────── -->
      <div class="idle-section-label">Input signals <span class="idle-note">raw / threshold-qualified readings</span></div>
      <ul class="kv-list">
        <li>
          <span class="kv-key kv-key--wide">system input idle</span>
          <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.systemIdleMs)}</span>
          {#if idleDebug.activityMode !== "system_input_only"}
            <span class="idle-note">keyboard/mouse only</span>
          {/if}
        </li>
        {#if idleDebug.activityMode === "system_input_or_screen" || idleDebug.activityMode === "system_input_or_screen_or_audio"}
          <li>
            <span class="kv-key kv-key--wide">screen activity idle</span>
            <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.screenActivityIdleMs)}</span>
            <span class="idle-note">time since last screen change</span>
          </li>
        {/if}
        {#if idleDebug.activityMode === "system_input_or_screen_or_audio"}
          <li>
            <span class="kv-key kv-key--wide">mic activity idle</span>
            <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.microphoneActivityDecision.idleMs)}</span>
            {#if idleDebug.microphoneActivitySample.level != null}
              <span class="idle-note">level {(idleDebug.microphoneActivitySample.level * 100).toFixed(0)}%</span>
            {/if}
            {#if !idleDebug.microphoneActivityDecision.enabled}
              <span class="badge badge--neutral badge--sm">off</span>
            {/if}
          </li>
          <li>
            <span class="kv-key kv-key--wide">sys audio activity idle</span>
            <span class="kv-val kv-val--mono">{formatIdleMs(idleDebug.systemAudioActivityDecision.idleMs)}</span>
            {#if idleDebug.systemAudioActivitySample.level != null}
              <span class="idle-note">level {(idleDebug.systemAudioActivitySample.level * 100).toFixed(0)}%</span>
            {/if}
            {#if !idleDebug.systemAudioActivityDecision.enabled}
              <span class="badge badge--neutral badge--sm">off</span>
            {/if}
          </li>
        {/if}
      </ul>

      <!-- ── Per-detector pause status ─────────────────── -->
      <div class="idle-section-label">Detector pause status</div>
      <div class="detector-grid">
        <!-- Screen detector -->
        <div class="detector-card detector-card--screen" class:detector-card--paused={idleDebug.screenPaused}>
          <div class="detector-card__header">
            <span class="detector-card__icon" aria-hidden="true"><IconMonitor /></span>
            <span class="detector-card__name">Screen</span>
            {#if idleDebug.screenPaused}
              <span class="badge badge--warn badge--sm">paused</span>
            {:else}
              <span class="badge badge--ok badge--sm">active</span>
            {/if}
          </div>
          <div class="detector-card__body">
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">effective idle</span>
              <span class="detector-card__metric-value">{formatIdleMs(idleDebug.screenEffectiveIdleMs)}</span>
            </div>
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">source</span>
              <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.screenEffectiveActivitySource)}</span>
            </div>
          </div>
        </div>

        <!-- Microphone detector -->
        <div
          class="detector-card detector-card--mic"
          class:detector-card--paused={idleDebug.microphonePaused && idleDebug.microphoneActivityDecision.enabled}
          class:detector-card--off={!idleDebug.microphoneActivityDecision.enabled}
        >
          <div class="detector-card__header">
            <span class="detector-card__icon" aria-hidden="true"><IconMic /></span>
            <span class="detector-card__name">Microphone</span>
            {#if !idleDebug.microphoneActivityDecision.enabled}
              <span class="badge badge--neutral badge--sm">off</span>
            {:else if idleDebug.microphonePaused}
              <span class="badge badge--warn badge--sm">paused</span>
            {:else}
              <span class="badge badge--ok badge--sm">active</span>
            {/if}
          </div>
          <div class="detector-card__body">
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">effective idle</span>
              <span class="detector-card__metric-value">{formatIdleMs(idleDebug.microphoneEffectiveIdleMs)}</span>
            </div>
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">source</span>
              <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.microphoneEffectiveActivitySource)}</span>
            </div>
            {#if idleDebug.microphoneActivitySample.level != null}
              <div class="detector-card__metric">
                <span class="detector-card__metric-label">level</span>
                <span class="detector-card__metric-value">{(idleDebug.microphoneActivitySample.level * 100).toFixed(0)}%</span>
              </div>
            {/if}
            {#if idleDebug.microphoneActivitySensitivity != null}
              <div class="detector-card__metric">
                <span class="detector-card__metric-label">sensitivity</span>
                <span class="detector-card__metric-source">{idleDebug.microphoneActivitySensitivity}%{#if idleDebug.microphoneActivityDecision.activityThreshold != null} (thr {(idleDebug.microphoneActivityDecision.activityThreshold * 100).toFixed(1)}%){/if}</span>
              </div>
            {/if}
          </div>
        </div>

        <!-- System audio detector -->
        <div
          class="detector-card detector-card--sysaudio"
          class:detector-card--paused={idleDebug.systemAudioPaused && idleDebug.systemAudioActivityDecision.enabled}
          class:detector-card--off={!idleDebug.systemAudioActivityDecision.enabled}
        >
          <div class="detector-card__header">
            <span class="detector-card__icon" aria-hidden="true"><IconVolume2 /></span>
            <span class="detector-card__name">System Audio</span>
            {#if !idleDebug.systemAudioActivityDecision.enabled}
              <span class="badge badge--neutral badge--sm">off</span>
            {:else if idleDebug.systemAudioPaused}
              <span class="badge badge--warn badge--sm">paused</span>
            {:else}
              <span class="badge badge--ok badge--sm">active</span>
            {/if}
          </div>
          <div class="detector-card__body">
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">effective idle</span>
              <span class="detector-card__metric-value">{formatIdleMs(idleDebug.systemAudioEffectiveIdleMs)}</span>
            </div>
            <div class="detector-card__metric">
              <span class="detector-card__metric-label">source</span>
              <span class="detector-card__metric-source">{formatEffectiveSource(idleDebug.systemAudioEffectiveActivitySource)}</span>
            </div>
            {#if idleDebug.systemAudioActivitySample.level != null}
              <div class="detector-card__metric">
                <span class="detector-card__metric-label">level</span>
                <span class="detector-card__metric-value">{(idleDebug.systemAudioActivitySample.level * 100).toFixed(0)}%</span>
              </div>
            {/if}
            {#if idleDebug.systemAudioActivitySensitivity != null}
              <div class="detector-card__metric">
                <span class="detector-card__metric-label">sensitivity</span>
                <span class="detector-card__metric-source">{idleDebug.systemAudioActivitySensitivity}%{#if idleDebug.systemAudioActivityDecision.activityThreshold != null} (thr {(idleDebug.systemAudioActivityDecision.activityThreshold * 100).toFixed(1)}%){/if}</span>
              </div>
            {/if}
          </div>
        </div>
      </div>

      <!-- ── Combined effective (subordinate) ────────────── -->
      <div class="effective-idle-summary">
        <span class="effective-idle-summary__label">combined effective idle</span>
        <span class="effective-idle-summary__value">{formatIdleMs(idleDebug.effectiveIdleMs)}</span>
        <span class="effective-idle-summary__source">via {formatEffectiveSource(idleDebug.effectiveActivitySource)}</span>
      </div>
      {#if idleDebug.activityMode === "system_input_or_screen_or_audio"}
        <p class="effective-idle-block__note">
          Audio mode: combined value is min-over-sources; detector pause states are still tracked per family.
        </p>
      {:else if idleDebug.activityMode === "system_input_or_screen"}
        <p class="effective-idle-block__note">
          Hybrid mode: pause requires <em>both</em> input and screen idle ≥ {idleDebug.idleTimeoutSeconds}s.
        </p>
      {:else}
        <p class="effective-idle-block__note">
          Input-only mode: pause when system input idle ≥ {idleDebug.idleTimeoutSeconds}s.
        </p>
      {/if}

      <details class="advanced">
        <summary class="advanced__summary">Raw samples &amp; probe</summary>
        <div class="idle-section-label">Activity sources <span class="idle-note">combined-policy samples</span></div>
        <ul class="kv-list">
          {#each idleDebug.activitySources as source (source.kind)}
            <li>
              <span class="kv-key kv-key--wide">{sourceKindLabel(source.kind)}</span>
              <span class="kv-val kv-val--mono">{formatIdleMs(source.idleMs)}</span>
              {#if source.latestNormalizedLevel != null}
                <span class="idle-note">lvl {(source.latestNormalizedLevel * 100).toFixed(0)}%{source.activityThreshold != null ? ` / thr ${(source.activityThreshold * 100).toFixed(0)}%` : ""}</span>
              {/if}
              <span class={source.selected ? "badge badge--ok badge--sm" : source.available ? "badge badge--neutral badge--sm" : "badge badge--warn badge--sm"}>
                {sourceDecisionSummary(source.available, source.selected, source.enabled)}
              </span>
            </li>
          {/each}
        </ul>

        <!-- ── Probe info ─────────────────────────────────── -->
        <div class="idle-section-label">Probe</div>
        <ul class="kv-list">
          <li>
            <span class="kv-key kv-key--wide">detector source</span>
            <span class="kv-val kv-val--mono">{idleDebug.detectorSource}</span>
          </li>
          {#if idleDebug.screenActivityLastUnixMs != null}
            <li>
              <span class="kv-key kv-key--wide">screen raw sample</span>
              <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.screenActivityLastUnixMs)}</span>
            </li>
          {/if}
          {#if idleDebug.microphoneActivitySensitivity != null}
            <li>
              <span class="kv-key kv-key--wide">mic sensitivity</span>
              <span class="kv-val kv-val--mono">{idleDebug.microphoneActivitySensitivity}%</span>
            </li>
          {/if}
          <li>
            <span class="kv-key kv-key--wide">mic VAD</span>
            <span class="kv-val kv-val--mono">
              {idleDebug.microphoneVad.configuredAdapter} -&gt; {idleDebug.microphoneVad.effectiveAdapter}
            </span>
            {#if idleDebug.microphoneVad.fallbackReason}
              <span class="idle-note">{idleDebug.microphoneVad.fallbackReason}</span>
            {/if}
          </li>
          {#if idleDebug.systemAudioActivitySensitivity != null}
            <li>
              <span class="kv-key kv-key--wide">sys audio sensitivity</span>
              <span class="kv-val kv-val--mono">{idleDebug.systemAudioActivitySensitivity}%</span>
            </li>
          {/if}
          {#if idleDebug.microphoneActivityDecision.activityThreshold != null}
            <li>
              <span class="kv-key kv-key--wide">mic threshold</span>
              <span class="kv-val kv-val--mono">{(idleDebug.microphoneActivityDecision.activityThreshold * 100).toFixed(1)}%</span>
              <span class="idle-note">normalised level; audio above this counts as activity</span>
            </li>
          {/if}
          {#if idleDebug.microphoneActivitySample.lastUnixMs != null}
            <li>
              <span class="kv-key kv-key--wide">mic raw sample</span>
              <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.microphoneActivitySample.lastUnixMs)}</span>
              <span class="idle-note">timestamp, not detector decision</span>
            </li>
          {/if}
          {#if idleDebug.systemAudioActivitySample.lastUnixMs != null}
            <li>
              <span class="kv-key kv-key--wide">sys-audio raw sample</span>
              <span class="kv-val kv-val--mono">{formatTimestamp(idleDebug.systemAudioActivitySample.lastUnixMs)}</span>
              <span class="idle-note">timestamp, not detector decision</span>
            </li>
          {/if}
        </ul>
      </details>
    </div>
  {/if}
</SettingGroup>
