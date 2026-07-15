<script lang="ts">
  // User Context — distillation runs, the window gate, digest freshness, and
  // the tail of the derivation-run ledger (mockup A).
  //
  // "Distillation gate" here is the WINDOW gate: low-signal windows the
  // scheduler recorded `skipped` before any LLM call (24h count from
  // `UserContextStatus.skippedWindows24h`). The per-draft persist gates
  // (ungrounded / guardrail / formation bar / resurface) of the last
  // Conclusion pass survive as the row's tooltip. The ledger tail below
  // answers "did the pass even run?".

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

  /** Newest ledger row of any kind — the "Last derivation run" headline. */
  const lastRun = $derived(features.derivationRuns[0] ?? null);
  /** Newest Activity-deriving run (forward window or backfill) for the stat. */
  const lastActivityRun = $derived(
    features.derivationRuns.find((run) => run.kind === "activity" || run.kind === "backfill") ?? null,
  );

  function runStatusBadgeClass(runStatus: string): string {
    if (runStatus === "completed") return "badge badge--ok badge--sm";
    if (runStatus === "failed") return "badge badge--err badge--sm";
    if (runStatus === "running") return "badge badge--running badge--sm";
    // `skipped` — a gated low-signal window, which is a normal outcome.
    return "badge badge--neutral badge--sm";
  }

  /** Per-draft persist-gate breakdown of the last Conclusion pass (tooltip). */
  const gateBreakdown = $derived.by(() => {
    if (!distillation) return null;
    const parts = [
      ["ungrounded", distillation.ungrounded],
      ["guardrail", distillation.guardrailSuppressed],
      ["below bar", distillation.belowFormationBar],
      ["resurface", distillation.resurfaceBlocked],
    ] as const;
    const active = parts.filter(([, n]) => n > 0);
    if (active.length === 0) return null;
    return `last pass also withheld drafts: ${active.map(([label, n]) => `${label} ${n}`).join(" · ")}`;
  });

  /** `+05:30` from frontend-stamped offset minutes. */
  function formatOffset(minutes: number): string {
    const sign = minutes < 0 ? "-" : "+";
    const abs = Math.abs(minutes);
    const hh = String(Math.floor(abs / 60)).padStart(2, "0");
    const mm = String(abs % 60).padStart(2, "0");
    return `${sign}${hh}:${mm}`;
  }

  function isToday(ms: number): boolean {
    const d = new Date(ms);
    const now = new Date();
    return d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  }

  const digest = $derived(status?.lastDayDigest ?? null);
  /** Fresh = the newest day digest was generated today (local clock). */
  const digestFresh = $derived(digest != null && isToday(digest.generatedAtMs));
  const digestDesc = $derived.by(() => {
    if (!digest) return "no daily digest generated yet";
    const at = digestFresh
      ? `today ${formatOptionalTime(digest.generatedAtMs)}`
      : new Date(digest.generatedAtMs).toLocaleString();
    const offset = status?.localOffsetMinutes;
    return `last generated ${at}${offset != null ? ` · local offset ${formatOffset(offset)}` : ""}`;
  });

  const stats = $derived.by<DebugStat[]>(() => [
    {
      key: "activities-run",
      label: "Activities (run)",
      value: lastActivityRun ? formatCount(lastActivityRun.activitiesDerived) : "—",
      sub: "derived last window",
    },
    {
      key: "conclusions-run",
      label: "Conclusions (run)",
      value: distillation ? formatCount(distillation.conclusionsDerived) : "—",
      sub: "upserted last pass",
    },
    {
      key: "subjects",
      label: "Subjects",
      value: formatCount(status?.subjectCount),
      sub: status ? `${formatCount(status.conclusionCount)} conclusions total` : null,
    },
    {
      key: "dismissed",
      label: "Dismissed",
      value: formatCount(status?.dismissedCount),
      sub: "excluded from digest",
    },
  ]);
</script>

<SettingGroup
  title="User Context"
  hint="activities · conclusions · digest"
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
      <div class="row__label">Last derivation run</div>
      <div class="row__desc">
        {#if lastRun}
          {formatOptionalTime(lastRun.createdAtMs)} · window {formatWindow(lastRun.windowStartMs, lastRun.windowEndMs)} · model {lastRun.model ?? "—"}
        {:else}
          no derivation runs yet
        {/if}
      </div>
    </div>
    <div class="row__value">
      {#if status && !status.engineAvailable}
        <span class="badge badge--warn">engine unavailable</span>
      {/if}
      {#if status?.backfilling}
        <span class="badge badge--running">backfilling</span>
      {/if}
      {#if lastRun}
        <span class={runStatusBadgeClass(lastRun.status)}>{lastRun.status}</span>
      {/if}
      <button class="btn btn--ghost btn--sm" onclick={features.runDerivationNow} disabled={features.runningDerivation}>
        {features.runningDerivation ? "…" : "run now"}
      </button>
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Distillation gate <span class="new-chip">new</span></div>
      <div class="row__desc">low-signal windows dropped before the LLM call</div>
    </div>
    <div class="row__value" use:tip={gateBreakdown ?? ""}>
      {formatCount(status?.skippedWindows24h)} <span class="dim">windows dropped 24h</span>
    </div>
  </div>

  <div class="row">
    <div class="row__main">
      <div class="row__label">Daily digest</div>
      <div class="row__desc">{digestDesc}</div>
    </div>
    <div class="row__value">
      {#if digest}
        <span class={digestFresh ? "badge badge--ok" : "badge badge--neutral"}>{digestFresh ? "fresh" : "stale"}</span>
      {:else}
        <span class="badge badge--neutral">none</span>
      {/if}
      <button class="btn btn--ghost btn--sm" onclick={features.regenerateDailyDigest} disabled={features.regeneratingDigest}>
        {features.regeneratingDigest ? "…" : "regenerate"}
      </button>
    </div>
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
  {#if features.digestMessage}
    <p class="debug-note" role="status" aria-live="polite">{features.digestMessage}</p>
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
