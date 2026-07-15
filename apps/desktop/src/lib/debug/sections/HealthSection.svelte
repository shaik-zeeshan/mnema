<script lang="ts">
  // Health — the summary scroll's first card, and mockup A's Health card:
  // a diagnosis strip over a 4-stat grid, with the session controls in the
  // card's action foot.
  //
  // ── The strip ─────────────────────────────────────────────────────────────
  // `get_debug_health` already returns 9 × { feature, severity, reason } where
  // `reason` is a plain-language sentence the BACKEND wrote. So the strip is a
  // renderer, not a composer: one line per non-ok feature (dot = its severity,
  // text = its reason), then one rolled-up "…healthy" line naming the rest.
  // `View →` drills into the feature — but only for the four features that HAVE
  // a detail view (`DETAIL_SPECS`); a dead link for the other five would be a
  // worse lie than no link.
  //
  // It also carries the probes the rollup does not cover — denied permissions,
  // an unsupported platform, a privacy filter that would not apply. Those are
  // capture-local conditions `get_debug_health` never sees, and the only other
  // place they surface is a collapsed <details> in Capture Sources, so dropping
  // them here would hide a genuinely broken install.

  import { tip } from "$lib/components/tooltip";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import StatGrid from "./StatGrid.svelte";
  import { anchor, DEBUG_SECTIONS } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import { DETAIL_SPECS, type DetailFeatureId } from "../detail/specs";
  import { formatCount, formatPermission, formatTimestamp, severityCardClass, type DebugStat } from "../format";
  import type { DebugFeature, DebugSeverity, PermissionStatus } from "$lib/types";

  const { capture, detail, features, health, pipeline } = getDebugController();

  /** One strip line. `drillTo` is set only when that feature has a detail view. */
  type StripLine = {
    id: string;
    severity: DebugSeverity;
    text: string;
    drillTo?: DetailFeatureId;
  };

  const PRIVACY_SUSPENSION_REASONS = new Set([
    "privacy_filter_apply_failed",
    "privacy_recovery_restart_required",
  ]);

  function hasDetail(feature: DebugFeature): feature is DetailFeatureId {
    return feature in DETAIL_SPECS;
  }

  /** The section label for a health feature — one registry, one spelling. */
  function featureLabel(feature: DebugFeature): string {
    return DEBUG_SECTIONS.find((s) => s.healthFeature === feature)?.label ?? feature;
  }

  /**
   * Capture-local conditions the rollup cannot see. Same shape as a rollup line
   * so the strip renders both without caring which is which.
   */
  const localLines = $derived.by<StripLine[]>(() => {
    const out: StripLine[] = [];

    if (capture.support && !capture.support.nativeCaptureSupported) {
      out.push({ id: "unsupported", severity: "error", text: "Native capture is not supported on this platform." });
    }
    // Note: `migrationsRan` is not a health signal — it only reports whether
    // pending migrations were applied at this startup, so it is `false` on any
    // already-current database. A genuine migration failure errors out before
    // the app reaches a running state, so there is nothing to warn about here.

    const permissions = capture.permissions;
    if (permissions) {
      const denied = (s: PermissionStatus | undefined) => s === "denied" || s === "restricted";
      if (denied(permissions.screen)) out.push({ id: "perm-screen", severity: "error", text: `Screen recording permission ${formatPermission(permissions.screen)} — nothing can be captured.` });
      if (denied(permissions.microphone)) out.push({ id: "perm-mic", severity: "warn", text: `Microphone permission ${formatPermission(permissions.microphone)}.` });
      if (denied(permissions.systemAudio)) out.push({ id: "perm-sys", severity: "warn", text: `System audio permission ${formatPermission(permissions.systemAudio)}.` });
    }

    // Only trust runtime-source health while capturing: idleDebug is not cleared
    // on stop, so a prior privacy_filter_apply_failed state would otherwise keep
    // surfacing a stale "sources suspended" warning while idle (mirrors sources).
    const rs = capture.isCapturing ? capture.idleDebug?.runtimeSources : null;
    if (rs) {
      const suspended = (["screen", "microphone", "systemAudio"] as const).some((key) => {
        const reason = rs[key].reason;
        return rs[key].requested && reason != null && PRIVACY_SUSPENSION_REASONS.has(reason);
      });
      if (suspended) out.push({ id: "privacy", severity: "error", text: "The privacy filter could not be applied, so some capture sources are suspended." });
    }

    if (capture.isInactivityPaused) {
      out.push({ id: "idle", severity: "warn", text: "Recording is paused on the inactivity timeout, waiting for activity." });
    }
    // `get_app_infra_status` counts the app-jobs table, which is a different
    // queue from the `processing_jobs` lanes the rollup reads — so this is not
    // a duplicate of the Jobs & Storage line.
    if (pipeline.infraStatus && pipeline.infraStatus.jobCounts.failed > 0) {
      const n = pipeline.infraStatus.jobCounts.failed;
      out.push({ id: "jobs", severity: "warn", text: `${n} background app job${n === 1 ? "" : "s"} failed.` });
    }

    return out;
  });

  /** The non-ok rollup entries, worst first, each drilling in where it can. */
  const rollupLines = $derived.by<StripLine[]>(() =>
    health.entries
      .filter((entry) => entry.severity !== "ok")
      .map((entry) => ({
        id: `health-${entry.feature}`,
        severity: entry.severity,
        text: entry.reason,
        drillTo: hasDetail(entry.feature) ? entry.feature : undefined,
      }))
  );

  // Errors above warnings; Array.sort is stable, so insertion order (roughly
  // most-blocking first) survives within each severity.
  const lines = $derived.by<StripLine[]>(() =>
    [...rollupLines, ...localLines].sort((a, b) =>
      a.severity === b.severity ? 0 : a.severity === "error" ? -1 : 1
    )
  );

  /** Mockup A's third strip line: "Capture, OCR, … healthy." */
  const healthyLabels = $derived(
    health.entries.filter((entry) => entry.severity === "ok").map((entry) => featureLabel(entry.feature))
  );

  const worstSeverity = $derived.by<DebugSeverity | null>(() => {
    if (lines.some((line) => line.severity === "error")) return "error";
    if (lines.some((line) => line.severity === "warn")) return "warn";
    return health.loaded ? "ok" : null;
  });

  // ─── Stats ────────────────────────────────────────────────────────────────
  // All four are live reads the page already polls: the pipeline lanes and the
  // semantic index (features store), and the capture session.

  const laneTotals = $derived.by(() => {
    let queued = 0;
    let running = 0;
    let failed24h = 0;
    const failedBy: string[] = [];
    for (const lane of features.lanes) {
      queued += lane.queued;
      running += lane.running;
      failed24h += lane.failedLast24h;
      if (lane.failedLast24h > 0) failedBy.push(`${lane.failedLast24h} ${lane.processor.replace(/_/g, " ")}`);
    }
    return { queued, running, failed24h, failedBy };
  });

  /** The earliest source-session start — "recording since". */
  const recordingSince = $derived.by(() => {
    const session = capture.session;
    if (!capture.isCapturing || !session) return null;
    const starts = (["screen", "microphone", "systemAudio"] as const)
      .map((source) => capture.getSourceSessionStartedAt(session, source))
      .filter((ms): ms is number => ms != null);
    return starts.length === 0 ? null : Math.min(...starts);
  });

  /** "ON" / "PAUSED" / "OFF", with the sub line saying since when or why. */
  const recordingStat = $derived.by<DebugStat>(() => {
    if (!capture.isCapturing) {
      return { key: "recording", label: "Recording", value: "OFF", sub: capture.session?.isRunning === false ? "stopped" : "idle" };
    }
    if (capture.session?.isLowDiskSuspended) {
      return { key: "recording", label: "Recording", value: "PAUSED", tone: "warn", sub: "suspended · low disk" };
    }
    if (capture.session?.isUserPaused) {
      return { key: "recording", label: "Recording", value: "PAUSED", tone: "warn", sub: "paused by you" };
    }
    if (capture.isInactivityPaused) {
      return { key: "recording", label: "Recording", value: "PAUSED", tone: "warn", sub: "inactivity timeout" };
    }
    return {
      key: "recording",
      label: "Recording",
      value: "ON",
      tone: "ok",
      sub: recordingSince == null ? null : `since ${new Date(recordingSince).toLocaleTimeString()}`,
    };
  });

  const stats = $derived.by<DebugStat[]>(() => [
    recordingStat,
    {
      key: "queue",
      label: "Pipeline queue",
      value: formatCount(laneTotals.queued),
      sub: `${laneTotals.running} running`,
      tone: laneTotals.queued > 0 ? "warn" : undefined,
    },
    {
      key: "backlog",
      label: "Index backlog",
      value: formatCount(features.semanticIndex?.backlogCount),
      sub: "anchors w/o vector",
      tone: (features.semanticIndex?.backlogCount ?? 0) > 0 ? "warn" : undefined,
      isNew: true,
    },
    {
      key: "failures",
      label: "Failures 24h",
      value: formatCount(laneTotals.failed24h),
      sub: laneTotals.failedBy.length > 0 ? laneTotals.failedBy.join(" · ") : "across every processor",
      tone: laneTotals.failed24h > 0 ? "warn" : undefined,
    },
  ]);

  const refreshing = $derived(capture.loadingSupport || capture.loadingPermissions || pipeline.loadingInfraStatus);


  // The all-clear line must not appear before the probes that feed the strip
  // have resolved — otherwise first open (and any failed probe that leaves its
  // state null) would falsely report everything healthy.
  const probesLoaded = $derived(
    health.loaded && capture.support != null && capture.permissions != null && pipeline.infraStatus != null
  );

  async function refresh(): Promise<void> {
    await Promise.all([capture.loadSupport(), capture.loadPermissions(), pipeline.fetchInfraStatus(), health.fetch()]);
  }

  // The mockup's hint for this card is the rollup's freshness — not a
  // description of it. Before the first poll lands there is no time to show,
  // so say so rather than print a placeholder clock.
  const healthHint = $derived(
    health.fetchedAtMs != null ? `${formatTimestamp(health.fetchedAtMs)} · auto-refresh` : "auto-refresh"
  );
</script>

<SettingGroup
  title="Health"
  hint={healthHint}
  hintInline
  id={anchor("health")}
  cardClass={severityCardClass(worstSeverity)}
>
  <div class="strip">
    {#each lines as line (line.id)}
      <div class="strip__item">
        <span class="health-dot health-dot--{line.severity === 'error' ? 'err' : 'warn'}"></span>
        <span>
          {line.text}
          {#if line.drillTo}
            {@const target = line.drillTo}
            <button class="strip__link" type="button" onclick={() => detail.open(target)}>View →</button>
          {/if}
        </span>
      </div>
    {/each}

    {#if healthyLabels.length > 0}
      <div class="strip__item">
        <span class="health-dot health-dot--ok"></span>
        <span>{healthyLabels.join(", ")} healthy.</span>
      </div>
    {/if}

    {#if health.error}
      <div class="strip__item" role="alert" aria-live="polite">
        <span class="health-dot health-dot--idle"></span>
        <span>Health rollup unavailable — {health.error}</span>
      </div>
    {:else if !probesLoaded && lines.length === 0 && healthyLabels.length === 0}
      <div class="strip__item">
        <span class="health-dot health-dot--idle"></span>
        <span>{refreshing || !health.loaded ? "Checking health…" : "Health state unavailable."}</span>
      </div>
    {/if}
  </div>

  <StatGrid {stats} />

  <div class="actions">
    <button class="btn btn--primary" onclick={capture.startCapture} disabled={capture.isCapturing || capture.loadingStart || capture.loadingSettings}>
      {capture.loadingStart ? "Starting…" : "Start"}
    </button>
    <button class="btn btn--danger" onclick={capture.stopCapture} disabled={!capture.isCapturing || capture.loadingStop}>
      {capture.loadingStop ? "Stopping…" : "Stop"}
    </button>
    <button
      class="btn btn--ghost"
      onclick={refresh}
      disabled={refreshing}
      aria-label="Refresh health"
      use:tip={"Refresh health"}
    >
      <span class="refresh-glyph" class:refresh-glyph--spin={refreshing} aria-hidden="true">↻</span>
    </button>
    {#if capture.reconcileStale}
      <span class="session-stale" role="status" aria-live="polite" use:tip={"The backend stopped responding to status checks; this readout may be out of date."}>status may be stale</span>
    {/if}
    {#if capture.lifecycleError}
      <span class="lifecycle-error" role="alert" aria-live="assertive" use:tip={capture.lifecycleError}>
        <span class="lifecycle-error__tag" aria-hidden="true">✕</span>
        <span class="lifecycle-error__text">{capture.lifecycleError}</span>
      </span>
    {/if}
  </div>
</SettingGroup>
