<script lang="ts">
  // The feature-detail template (level 2) — ONE component, instantiated for the
  // four features in `specs.ts`. Breadcrumb, status hero with the backend's
  // plain-language diagnosis + a 5-stat strip, `Segmented` sub-tabs
  // (Overview · Jobs · Config · Log tail), and the jobs table + inspector.
  //
  // It is pushed *within* /debug, not routed: the shell renders this instead of
  // the summary scroll while `detail.feature` is set, so back = clear that.
  //
  // Where the mockup and the app disagree, the app wins:
  //   • "Reprocess all failed" is absent — there is no bulk reprocess command.
  //   • The hero's "Retrying" stat is "Failed" — the lane aggregate counts
  //     statuses, and retrying is derived per row (see jobs.ts).
  //   • Chips/segments are the real `log-chip` / `Segmented` components.

  import { untrack } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import Segmented from "$lib/components/Segmented.svelte";
  import IconArrowLeft from "~icons/lucide/arrow-left";
  import StatGrid from "../sections/StatGrid.svelte";
  import LogTail from "../LogTail.svelte";
  import JobsTable from "./JobsTable.svelte";
  import JobInspector from "./JobInspector.svelte";
  import { DEBUG_SECTIONS } from "../sections";
  import { getDebugController } from "../state/controller.svelte";
  import { formatCount, severityBadgeClass, severityLabel, type DebugStat } from "../format";
  import { DETAIL_SPECS, type DetailFeatureId } from "./specs";
  import type { DetailTab } from "../state/detail.svelte";

  interface Props {
    /** The pushed feature — the shell only renders this when one is open. */
    feature: DetailFeatureId;
    /** Breadcrumb + Esc. The shell restores the summary scroll position. */
    onback: () => void;
  }

  let { feature, onback }: Props = $props();

  const { capture, detail, health } = getDebugController();

  const spec = $derived(DETAIL_SPECS[feature]);
  // A DetailFeatureId is also a DebugSectionId and a DebugFeature, so the label,
  // the health dot and the log chip all come from the existing registries.
  const label = $derived(DEBUG_SECTIONS.find((section) => section.id === feature)?.label ?? feature);
  const severity = $derived(health.severityFor(feature));
  const settings = $derived(spec.config(capture.recordingSettings ?? null));

  const TABS: { value: DetailTab; label: string }[] = [
    { value: "overview", label: "Overview" },
    { value: "jobs", label: "Jobs" },
    { value: "config", label: "Config" },
    { value: "log", label: "Log tail" },
  ];
  // A feature with no job lane has no jobs tab to offer (Embeddings): disable it
  // rather than render an empty table that reads as "nothing is queued".
  const disabledTabs = $derived(spec.processor ? [] : ["jobs"]);

  const lane = $derived(detail.lane);
  const index = $derived(detail.semanticIndex);

  /**
   * The hero strip. Two shapes, because the two kinds of feature genuinely
   * measure different things: a job lane counts jobs, the semantic index counts
   * vectors. Everything else on the page is shared.
   */
  const stats = $derived.by<DebugStat[]>(() => {
    if (spec.processor) {
      return [
        { key: "queued", label: "Queued", value: formatCount(lane?.queued) },
        { key: "running", label: "Running", value: formatCount(lane?.running), tone: (lane?.running ?? 0) > 0 ? "ok" : undefined },
        { key: "failed", label: "Failed", value: formatCount(lane?.failed), sub: "terminal", tone: (lane?.failed ?? 0) > 0 ? "err" : undefined },
        { key: "failed24h", label: "Failed 24h", value: formatCount(lane?.failedLast24h), tone: (lane?.failedLast24h ?? 0) > 0 ? "warn" : undefined },
        {
          key: "completed",
          label: "Completed",
          value: formatCount(lane?.completed),
          sub: lane?.averageCompletedSecondsLast24h != null ? `avg ${lane.averageCompletedSecondsLast24h.toFixed(1)}s (24h)` : null,
        },
      ];
    }
    return [
      { key: "vectors", label: "Vectors", value: formatCount(index?.vectorCount) },
      { key: "backlog", label: "Backlog", value: formatCount(index?.backlogCount), sub: "anchors w/o vector", tone: (index?.backlogCount ?? 0) > 0 ? "warn" : undefined },
      { key: "quarantined", label: "Quarantined", value: formatCount(index?.quarantinedCount ?? 0), sub: "since app start", tone: (index?.quarantinedCount ?? 0) > 0 ? "warn" : undefined },
      { key: "dimension", label: "Dimension", value: index?.liveDimension ?? "—", sub: index && index.liveDimension == null ? "index table absent" : null, tone: index && index.liveDimension == null ? "err" : undefined },
      { key: "loadFailures", label: "Load failures", value: formatCount(index?.consecutiveLoadFailures ?? 0), sub: "consecutive", tone: (index?.consecutiveLoadFailures ?? 0) > 0 ? "err" : undefined },
    ];
  });

  /** The Overview rows — the facts behind the hero's one-line diagnosis. */
  const facts = $derived.by<{ k: string; v: string }[]>(() => {
    if (spec.processor) {
      return [
        { k: "processor", v: spec.processor },
        { k: "subject type", v: spec.subjectType ?? "—" },
        { k: "avg duration (24h)", v: lane?.averageCompletedSecondsLast24h != null ? `${lane.averageCompletedSecondsLast24h.toFixed(1)}s` : "nothing completed in the window" },
        { k: "reprocess command", v: spec.reprocess?.command ?? "none" },
      ];
    }
    return [
      { k: "worker", v: index?.modelLoaded ? "embedder loaded" : "embedder idle (unloaded after the grace period — normal)" },
      { k: "index dimension", v: index?.liveDimension != null ? String(index.liveDimension) : "index table absent" },
      { k: "job lane", v: "none — the semantic index is swept by a worker, not queued as processing jobs" },
    ];
  });

  /** The feature's last error, whichever surface it came from. */
  const lastError = $derived(spec.processor ? (lane?.lastError ?? null) : (index?.lastLoadError ?? null));

  // The detail's own 1s poll, for this feature only. The shell stops the
  // summary's pollers while this view is open.
  //
  // `untrack`: startPolling kicks a poll synchronously, which reads the feature
  // / page / filter state it fetches for. Tracked, this effect would tear the
  // interval down and rebuild it on every page or chip change — and double-fetch
  // alongside the setter that already repolls. The store owns those repolls.
  $effect(() => untrack(() => detail.startPolling()));

  // Esc → back.
  //
  // BUBBLE phase on window, deliberately not capture: capture would run before
  // anything layered above this view (a popover, a menu) could handle its own
  // Esc, and this would swallow it. On the bubble phase, whatever is on top
  // handles Esc first and stops/defaults it; only an unclaimed Esc reaches here.
  // A window listener (not element `onkeydown`) because this WKWebView doesn't
  // focus a <button> on click — there is no reliable focused element to hang it
  // off. Native plugin-dialog prompts never deliver keys to the webview at all.
  $effect(() => {
    const onKeydown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      onback();
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });
</script>

<div class="debug-detail">
  <div class="debug-detail__crumb">
    <button class="debug-detail__back" onclick={onback}>
      <IconArrowLeft />
      Debug
    </button>
    <span class="debug-detail__crumb-sep" aria-hidden="true">/</span>
    <b class="debug-detail__crumb-here">{label}</b>
    <span class="debug-detail__esc">esc to go back</span>
  </div>

  <div class="debug-detail__hero" class:debug-detail__hero--warn={severity === "warn"} class:debug-detail__hero--err={severity === "error"}>
    <div class="debug-detail__hero-top">
      <span class="debug-detail__hero-title">{label}</span>
      <span class={severityBadgeClass(severity)}>{severityLabel(severity)}</span>
      <span class="debug-detail__spacer"></span>
      {#if settings}
        <span class="badge badge--neutral badge--sm" use:tip={"The provider + model this feature is configured to use"}>
          {settings.provider}{settings.modelId ? ` · ${settings.modelId}` : ""}
        </span>
      {/if}
    </div>
    <!-- The plain-language diagnosis is the health rollup's own `reason` — the
         same sentence the dock tooltip shows, sourced once, backend-side. -->
    <p class="debug-detail__hero-desc">{health.reasonFor(feature) ?? "no diagnosis yet — the health rollup has not been read"}</p>
    <StatGrid {stats} columns={5} />
  </div>

  <div class="debug-detail__tabs">
    <Segmented
      options={TABS}
      value={detail.tab}
      onValueChange={(value) => (detail.tab = value as DetailTab)}
      disabledValues={disabledTabs}
      ariaLabel="{label} detail section"
    />
  </div>

  {#if detail.error}
    <p class="debug-err" role="alert" aria-live="polite">{detail.error}</p>
  {/if}

  {#if detail.tab === "overview"}
    <div class="debug-detail__card debug-detail__pane">
      <ul class="kv-list">
        {#each facts as fact (fact.k)}
          <li>
            <span class="kv-key kv-key--wide">{fact.k}</span>
            <span class="kv-val kv-val--mono">{fact.v}</span>
          </li>
        {/each}
      </ul>
      {#if lastError}
        <p class="debug-errline" role="status" aria-live="polite">
          {lastError}
          <span class="debug-errline__meta">newest error on this lane</span>
        </p>
      {:else}
        <p class="empty">no errors recorded</p>
      {/if}
    </div>
  {:else if detail.tab === "jobs" && spec.processor}
    <JobsTable />
    <JobInspector {feature} />
  {:else if detail.tab === "config"}
    <div class="debug-detail__card debug-detail__pane">
      <!-- The settings slice exactly as the backend holds it — on a debug page
           the raw record beats a prettied-up restatement of it. -->
      {#if settings}
        <pre class="debug-json">{JSON.stringify(settings, null, 2)}</pre>
      {:else}
        <p class="empty">recording settings have not loaded yet</p>
      {/if}
    </div>
  {:else if detail.tab === "log"}
    <div class="debug-detail__card debug-detail__pane">
      <!-- Slice 8's viewer, seeded to this feature's chip — and, when the
           inspector's "filter log to this job" routed here, to that job id. -->
      <LogTail {feature} needle={detail.logNeedle ?? ""} />
    </div>
  {/if}
</div>
