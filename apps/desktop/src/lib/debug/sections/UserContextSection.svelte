<script lang="ts">
  // User Context — distillation runs, gate drops, and the tail of the
  // derivation-run ledger (mockup A).
  //
  // Gate drops come from `UserContextStatus.lastDistillation`, the per-gate
  // withheld counts of the most recent Conclusion pass — the "why is my dossier
  // thin?" readout. The ledger tail below answers "did the pass even run?".

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import {
    formatCount,
    formatOptionalTime,
    formatWindow,
    severityBadgeClass,
    severityCardClass,
    severityLabel,
    truncateDebugText,
    type DebugStat,
  } from "../format";

  const { features, health } = getDebugController();

  const severity = $derived(health.severityFor("userContext"));
  const status = $derived(features.userContextStatus);
  const distillation = $derived(status?.lastDistillation ?? null);

  function runStatusBadgeClass(runStatus: string): string {
    if (runStatus === "completed") return "badge badge--ok badge--sm";
    if (runStatus === "failed") return "badge badge--err badge--sm";
    if (runStatus === "running") return "badge badge--running badge--sm";
    // `skipped` — a gated low-signal window, which is a normal outcome.
    return "badge badge--neutral badge--sm";
  }

  /** Every gate that withheld something on the last Conclusion pass. */
  const gateDrops = $derived.by(() => {
    if (!distillation) return 0;
    return distillation.ungrounded + distillation.guardrailSuppressed
      + distillation.belowFormationBar + distillation.resurfaceBlocked;
  });

  const gateBreakdown = $derived.by(() => {
    if (!distillation) return null;
    const parts = [
      ["ungrounded", distillation.ungrounded],
      ["guardrail", distillation.guardrailSuppressed],
      ["below bar", distillation.belowFormationBar],
      ["resurface", distillation.resurfaceBlocked],
    ] as const;
    const active = parts.filter(([, n]) => n > 0);
    return active.length === 0 ? null : active.map(([label, n]) => `${label} ${n}`).join(" · ");
  });

  const stats = $derived.by<DebugStat[]>(() => [
    { key: "activities", label: "Activities", value: formatCount(status?.activityCount), sub: "total derived" },
    { key: "conclusions", label: "Conclusions", value: formatCount(status?.conclusionCount), sub: "total derived" },
    {
      key: "gate",
      label: "Gate drops",
      value: distillation ? gateDrops : "—",
      sub: gateBreakdown ?? "last distillation",
      tone: gateDrops > 0 ? "warn" : undefined,
      isNew: true,
    },
    {
      key: "tokens",
      label: "Tokens",
      value: formatCount(status?.tokenUsage.totalTokens),
      sub: status ? `${formatCount(status.tokenUsage.runCount)} runs` : null,
    },
  ]);
</script>

<SettingGroup
  title="User Context"
  hint="activities · conclusions · derivation ledger"
  hintInline
  id={anchor("userContext")}
  cardClass={severityCardClass(severity)}
>
  {#snippet titleExtra()}
    <span class={severityBadgeClass(severity)} use:tip={health.reasonFor("userContext") ?? ""}>
      <span class="b-dot" aria-hidden="true"></span>{severityLabel(severity)}
    </span>
    <span class="new-chip">new</span>
  {/snippet}

  {#snippet actions()}
    <button
      class="btn btn--ghost btn--sm"
      onclick={features.loadUserContextStatus}
      disabled={features.loadingUserContextStatus}
      aria-label="Refresh user context status"
      use:tip={"Refresh status"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={features.loadingUserContextStatus} aria-hidden="true">↻</span>
    </button>
  {/snippet}

  <div class="row">
    <div class="row__main">
      <div class="row__label">Derivation</div>
      <div class="row__desc">
        {status?.engineAvailable ? "an AI model is available to distill with" : "no usable AI model — distillation cannot run"}
      </div>
    </div>
    <div class="row__value">
      <span class={status?.engineAvailable ? "badge badge--ok" : "badge badge--warn"}>
        {status?.engineAvailable ? "available" : "unavailable"}
      </span>
      {#if status?.backfilling}
        <span class="badge badge--running">backfilling</span>
      {/if}
      <button class="btn btn--ghost btn--sm" onclick={features.runDerivationNow} disabled={features.runningDerivation}>
        {features.runningDerivation ? "…" : "run now"}
      </button>
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Last derived</div>
      <div class="row__desc">the most recent pass that produced anything</div>
    </div>
    <div class="row__value">{formatOptionalTime(status?.lastDerivedAtMs)}</div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Covered until</div>
      <div class="row__desc">activity up to here has been read into context</div>
    </div>
    <div class="row__value">{formatOptionalTime(status?.coveredUntilMs)}</div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Budget tier</div>
      <div class="row__desc">how much the distillation passes are allowed to spend</div>
    </div>
    <div class="row__value"><span class="badge badge--neutral">{status?.budgetTier ?? "—"}</span></div>
  </div>

  {#if status?.reason}
    <p class="debug-warnline" role="status" aria-live="polite">{status.reason}</p>
  {/if}
  {#if features.userContextStatusError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.userContextStatusError}</p>
  {/if}
  {#if features.derivationRunMessage}
    <p class="debug-note" role="status" aria-live="polite">{features.derivationRunMessage}</p>
  {/if}

  <StatGrid {stats} />

  {#if features.derivationRunsError}
    <p class="debug-errline" role="alert" aria-live="polite">{features.derivationRunsError}</p>
  {:else if features.derivationRuns.length === 0}
    <div class="jobs"><p class="empty">no derivation runs yet</p></div>
  {:else}
    <div class="jobs">
      <table>
        <thead>
          <tr>
            <th>run</th>
            <th>kind</th>
            <th>window</th>
            <th class="cell-num" use:tip={"activities derived"}>act</th>
            <th class="cell-num" use:tip={"conclusions derived"}>concl</th>
            <th class="cell-num" use:tip={"input / output tokens"}>tok</th>
            <th>result</th>
            <th>error</th>
          </tr>
        </thead>
        <tbody>
          {#each features.derivationRuns as run (run.id)}
            <tr>
              <td class="mono-dim">{formatOptionalTime(run.createdAtMs)}</td>
              <td>{run.kind}</td>
              <td class="mono-dim">{formatWindow(run.windowStartMs, run.windowEndMs)}</td>
              <td class="cell-num">{run.activitiesDerived}</td>
              <td class="cell-num">{run.conclusionsDerived}</td>
              <td class="cell-num">{formatCount(run.inputTokens)} / {formatCount(run.outputTokens)}</td>
              <td><span class={runStatusBadgeClass(run.status)}>{run.status}</span></td>
              <td use:tip={run.error ?? ""}>{truncateDebugText(run.error, 40)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</SettingGroup>
