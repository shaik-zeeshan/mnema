<script lang="ts">
  // AI tab (docs/mockups/redesign/developer.html ?state=ai): engine status,
  // the last Ask AI turn, recent User Context runs and recent CLI requests.
  // Config (engine, User Context, CLI grants) loads once per mount; the three
  // live reads poll at 5 s. The engine status is never polled: it pings a local
  // model server for Ollama/Llamafile.

  import { getDebugController } from "$lib/debug/state/controller.svelte";
  import { cliRows, digestLine, durationStat, engineView, providerName, runRow, tokenStat } from "$lib/debug/ai-view";
  import { formatClock } from "$lib/time-format";
  import { tip } from "$lib/components/tooltip";
  import IconPlug from "~icons/lucide/plug";
  import IconRefresh from "~icons/lucide/refresh-cw";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInbox from "~icons/lucide/inbox";
  import { untrack } from "svelte";

  const controller = getDebugController();
  const features = controller.features;

  $effect(() => {
    untrack(() => void features.loadAiConfig());
    return controller.poll(() => features.fetchAiLive(), 5000);
  });

  const engine = $derived(engineView(features.aiStatus, features.aiStatusError));
  const usage = $derived(features.askAiUsage);
  const stats = $derived(
    usage
      ? [
          { label: "Read", v: tokenStat(usage.inputTokens) },
          { label: "Wrote", v: tokenStat(usage.outputTokens) },
          { label: "Tool calls", v: [String(usage.toolCalls), ""] as [string, string] },
          { label: "Took", v: durationStat(usage.durationMs) },
        ]
      : [],
  );
  const runs = $derived(features.derivationRuns.map(runRow));
  const cli = $derived(cliRows(features.cliHistory));
  const allowedTools = $derived(features.cliGrants.filter((g) => !g.blocked).length);
  const ucStatus = $derived(features.userContextStatus);
  const ucLine = $derived(
    ucStatus?.reason === "user_context_disabled" ? "turned off" : ucStatus ? digestLine(ucStatus.lastDayDigest) : "",
  );
  const test = $derived(features.aiTestResult);
</script>

{#snippet empty(title: string, failed = false)}
  <div class="mx-empty mx-empty--compact dv-empty" data-tone={failed ? "danger" : undefined}>
    <span class="mx-empty__glyph">{#if failed}<IconAlert width="14" height="14" />{:else}<IconInbox width="14" height="14" />{/if}</span>
    <b class="mx-empty__title">{title}</b>
  </div>
{/snippet}

<section class="dv-sec">
  {#if engine}
    <div class="dv-engine" data-tone={engine.tone}>
      <span class="dv-dot"></span>
      <div>
        <b>{engine.title}</b>
        {#if engine.provider || engine.model}
          <span>{engine.provider ?? ""}{#if engine.provider && engine.model}{" · "}{/if}{#if engine.model}<code>{engine.model}</code>{/if}</span>
        {/if}
      </div>
      <button
        class="mx-btn mx-btn--sm"
        aria-busy={features.testingAi}
        disabled={features.testingAi}
        onclick={() => features.testAiConnection()}
      >
        {#if features.testingAi}<span class="mx-spin mx-spin--sm"></span>Testing…{:else}<IconPlug width="13" height="13" />Test connection{/if}
      </button>
    </div>
  {:else}
    <div class="dv-engine" aria-busy="true">
      <span class="dv-dot"></span>
      <div><span>Checking the AI engine…</span></div>
    </div>
  {/if}
  {#if test && !features.testingAi}
    <p class="mx-inline dv-test" data-tone={test.ok ? "ok" : "danger"}>
      {#if test.ok}
        <IconCheck width="13" height="13" />Reached {providerName(test.provider)} · <code>{test.model}</code>
      {:else}
        <IconAlert width="13" height="13" />{test.message}
      {/if}
    </p>
  {/if}
</section>

<section class="dv-sec">
  <h3 class="dv-h">
    Last Ask AI turn<span class="mx-spacer"></span>{#if usage}<span class="num">{formatClock(usage.startedAtMs)}</span>{/if}
  </h3>
  {#if usage}
    <div class="dv-stats">
      {#each stats as stat (stat.label)}
        <div class="mx-stat">
          <span class="mx-stat__label">{stat.label}</span>
          <span class="mx-stat__value">{stat.v[0]}{#if stat.v[1]}<small>{stat.v[1]}</small>{/if}</span>
        </div>
      {/each}
    </div>
  {:else}
    {@render empty("No Ask AI turn since the app started")}
  {/if}
</section>

<section class="dv-sec">
  <h3 class="dv-h">
    User Context{#if ucLine}<span class="num">{ucLine}</span>{/if}<span class="mx-spacer"></span>
    <button
      class="mx-btn mx-btn--ghost mx-btn--sm"
      aria-busy={features.runningDerivation}
      disabled={features.runningDerivation}
      onclick={() => features.runDerivationNow()}
    >
      {#if features.runningDerivation}<span class="mx-spin mx-spin--sm"></span>Deriving…{:else}<IconRefresh width="13" height="13" />Derive latest now{/if}
    </button>
  </h3>
  {#if features.derivationRunsError}
    {@render empty("Couldn’t load recent runs", true)}
  {:else if runs.length === 0}
    {@render empty("No User Context runs yet")}
  {:else}
    <ul class="dv-log" aria-label="Recent User Context runs">
      {#each runs as run, i (features.derivationRuns[i].id)}
        <li>
          <span><b>{run.kind}</b>{#if run.window}{" · "}{run.window}{/if}</span>
          <span class="mx-chip" class:mx-chip--danger={run.chip.tone === "danger"} class:mx-chip--info={run.chip.tone === "info"}>{run.chip.label}</span>
          {#if features.derivationRuns[i].error}
            <span use:tip={features.derivationRuns[i].error}>{run.result}</span>
          {:else}
            <span>{run.result}</span>
          {/if}
          <span class="num">{run.meta}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<section class="dv-sec">
  <h3 class="dv-h">
    CLI access<span class="num">{allowedTools} {allowedTools === 1 ? "tool" : "tools"} allowed</span>
  </h3>
  {#if features.cliHistoryError}
    {@render empty("Couldn’t read CLI requests", true)}
  {:else if cli.length === 0}
    {@render empty("No CLI requests yet")}
  {:else}
    <ul class="dv-log" aria-label="Recent CLI requests">
      {#each cli as row, i (i)}
        <li>
          <b>{row.tool}</b><code>{row.command}</code>
          <span class="mx-chip" class:mx-chip--warn={!row.allowed}>{row.allowed ? "allowed" : "refused"}</span>
          <span class="num">{row.time}</span>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .dv-engine { display: grid; grid-template-columns: 7px minmax(0, 1fr) auto; align-items: center; gap: 10px; padding: 10px 12px;
    border: 1px solid var(--app-border); border-radius: var(--r-md); background: var(--app-surface-subtle); }
  .dv-engine b { display: block; font: 500 var(--text-md)/1.3 var(--font-sans); color: var(--app-text-strong); }
  .dv-engine span:not(.dv-dot):not(.mx-spin) { font: 400 var(--text-base)/1.4 var(--font-sans); color: var(--app-text-subtle); }
  .dv-engine code, .dv-test code { font: 450 var(--text-sm)/1 var(--font-mono); color: var(--app-text-muted); }
  .dv-test { margin: 0; padding: 0 2px; color: var(--app-text-muted); }
  .dv-test :global(svg) { color: var(--tone); }
  .dv-empty { padding: 10px 0; }
  .dv-empty .mx-empty__glyph { grid-row: auto; }
  .dv-stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); border: 1px solid var(--app-border); border-radius: var(--r-md); }
  .dv-stats .mx-stat { padding: 10px 12px; gap: 6px; }
  .dv-stats .mx-stat + .mx-stat { border-left: 1px solid var(--mx-hairline); }
  .dv-stats .mx-stat__value { font-size: var(--text-lg); letter-spacing: -0.03em; }
  .dv-stats .mx-stat__value small { font-size: var(--text-sm); margin-left: 2px; }
  .dv-stats .mx-stat__label { font: 500 var(--text-sm)/1 var(--font-sans); letter-spacing: 0; text-transform: none; }
</style>
