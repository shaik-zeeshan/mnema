<script lang="ts">
  // AI Runtime — the Reasoning Engine's provider, default model, connection
  // state and last Ask AI turn's token usage (mockup A).
  //
  // No stat grid: this card is all config rows, exactly as the mockup has it.
  // `get_ai_runtime_status` pings a local engine endpoint, so it loads on mount
  // and on ↻ rather than on the 1s tick (see state/features.svelte.ts).
  //
  // The mockup's "MCP connectors" row is deliberately absent: no command
  // reports connector/oauth state to this page, and inventing "2 servers · oauth
  // ok" would be a lie.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import { formatCount, severityBadgeClass, severityCardClass, severityLabel } from "../format";

  const { capture, features, health } = getDebugController();

  const severity = $derived(health.severityFor("aiRuntime"));
  const status = $derived(features.aiStatus);
  const usage = $derived(features.askAiUsage);
  const providers = $derived(capture.recordingSettings?.aiRuntime.providers ?? []);

  /**
   * `configured` is "the static config is complete"; `available` additionally
   * requires the local reachability ping. The split matters here: "configured
   * but currently offline" is a different diagnosis from "never set up".
   */
  const connection = $derived.by(() => {
    if (!status) return { word: "unknown", cls: "badge badge--neutral", why: "engine status not read yet" };
    if (status.available) return { word: "available", cls: "badge badge--ok", why: "configured and reachable" };
    if (status.configured) return { word: "offline", cls: "badge badge--warn", why: "configured, but the engine did not answer its reachability ping" };
    return { word: "not configured", cls: "badge badge--err", why: "no provider is set up yet" };
  });
</script>

<SettingGroup
  title="AI Runtime"
  hint="reasoning engine · ask ai"
  hintInline
  id={anchor("aiRuntime")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("aiRuntime") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
    <span class="new-chip">new</span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={features.loadAiStatus}
      disabled={features.loadingAiStatus}
      aria-label="Refresh AI runtime status"
      use:tip={"Refresh engine status"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={features.loadingAiStatus} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Engine</div>
      <div class="row__desc">{connection.why}</div>
    </div>
    <div class="row__value">
      <span class={connection.cls}>{connection.word}</span>
      {#if status && !status.enabled}
        <span class="badge badge--neutral">disabled</span>
      {/if}
      <button class="btn btn--ghost btn--sm" onclick={features.testAiConnection} disabled={features.testingAi}>
        {features.testingAi ? "…" : "test connection"}
      </button>
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Default model</div>
      <div class="row__desc row__desc--mono">
        {status?.defaultModel ? `${status.defaultModel.provider} · ${status.defaultModel.model}` : "none selected"}
      </div>
    </div>
    <div class="row__value">
      <span class={status?.defaultModel ? "badge badge--ok" : "badge badge--warn"}>
        {status?.defaultModel ? "configured" : "unset"}
      </span>
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Providers</div>
      <div class="row__desc row__desc--mono">
        {providers.length === 0 ? "none connected" : providers.map((p) => p.id).join(" · ")}
      </div>
    </div>
    <div class="row__value">
      <span class={providers.length > 0 ? "badge badge--ok" : "badge badge--neutral"}>
        {providers.length} connected
      </span>
    </div>
  </div>

  <div class="row">
    <!-- In-memory last-turn cell: `null` until a turn runs, and a restart
         clears it. Not persisted (PLAN: no conversation migration). -->
    <div class="row__main">
      <div class="row__label">Last Ask AI turn <span class="new-chip">new</span></div>
      <div class="row__desc">token usage of the most recent turn, since app start</div>
    </div>
    <div class="row__value">
      {#if usage}
        in {formatCount(usage.inputTokens)} <span class="dim">tok</span> · out {formatCount(usage.outputTokens)} <span class="dim">tok</span>
      {:else}
        <span class="dim" use:tip={"No Ask AI turn has run since the app started."}>no turns yet</span>
      {/if}
    </div>
  </div>

  {#if status?.reason}
    <p class="debug-warnline" role="status" aria-live="polite">{status.reason}</p>
  {/if}
  {#if features.aiStatusError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.aiStatusError}</p>
  {/if}
  {#if features.aiTestResult}
    <p class={features.aiTestResult.ok ? "debug-note" : "debug-errline"} role="status" aria-live="polite">
      {features.aiTestResult.message}
    </p>
  {/if}
</SettingGroup>
