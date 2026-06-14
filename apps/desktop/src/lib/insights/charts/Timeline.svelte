<script lang="ts">
  // Timeline — a chronological / time-of-day breakdown of the user's day, shown
  // inline as a Chat answer-chart. Renders a vertical timeline rail: a thin spine
  // with a per-row colour dot (category colour), the time range in a quiet
  // tabular monospace, the label as the primary text, and an optional app chip.
  //
  // The caller passes an already-validated, parsed array of intervals (the Chat
  // answer parser does the validation), so this component is presentation-only
  // and defensive about missing end / app / category.
  //
  // Props:
  //   title?: string | null            — small uppercase muted caption at the top.
  //   intervals: {                      — time-ordered rows.
  //     label: string;
  //     start: string;                  — human time-of-day, e.g. "9:30 AM".
  //     end?: string | null;            — optional human time-of-day.
  //     app?: string | null;            — optional app / window context chip.
  //     category?: string | null;       — optional category key for the dot colour.
  //   }[]

  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { CATEGORY_COLOR } from "$lib/insights/activity-helpers";
  import {
    appIconFallback,
    canonicalBundleIdForComparison,
    iconPathForBundleId,
    mergeIconResolutions,
    unresolvedIconBundleIds,
    type AppIconResolution,
  } from "$lib/app-privacy-exclusion";

  interface TimelineInterval {
    label: string;
    start: string;
    end?: string | null;
    app?: string | null;
    category?: string | null;
  }

  interface Props {
    title?: string | null;
    intervals: TimelineInterval[];
  }

  let { title = null, intervals }: Props = $props();

  // Unknown/missing category falls back to the neutral chart grey — mirrors the
  // UNCATEGORIZED_COLOR used elsewhere in the Insights surfaces.
  const FALLBACK_COLOR = "--chart-grey-3";

  function colorVarFor(category?: string | null): string {
    if (!category) return FALLBACK_COLOR;
    return (CATEGORY_COLOR as Record<string, string>)[category] ?? FALLBACK_COLOR;
  }

  function timeRange(interval: TimelineInterval): string {
    if (interval.end) return `${interval.start} – ${interval.end}`;
    return interval.start;
  }

  // The model is told to emit a bare application name in `app`, but it sometimes
  // appends a window title / tab as a trailing parenthetical (e.g. "Zen (Zen)").
  // Strip that so the chip stays to just the app — conservative: only a single
  // trailing `(…)` group, leaving names without one untouched.
  function appLabel(app: string | null | undefined): string | null {
    if (!app) return null;
    const cleaned = app.replace(/\s*\([^()]*\)\s*$/, "").trim();
    return cleaned.length > 0 ? cleaned : app.trim();
  }

  // App icons beside each interval's app label — mirrors the App Privacy
  // Exclusion idiom used by the Chat tool chips and the dashboard timeline
  // tooltip. The interval `app` is usually a human display name; the backend
  // resolves it (by bundle id, else by display name against the installed-app
  // catalog) to a real icon, otherwise the letter fallback shows. Resolutions
  // are id-keyed facts; a null stays in the requested set so it is not re-fetched.
  let appIconPaths = $state<Record<string, string>>({});
  const requestedAppIconIds = new Set<string>();

  async function resolveAppIcons(
    apps: Array<string | null | undefined>,
  ): Promise<void> {
    const unresolved = unresolvedIconBundleIds(
      apps,
      appIconPaths,
      requestedAppIconIds,
    );
    if (unresolved.length === 0) return;
    for (const id of unresolved) {
      requestedAppIconIds.add(canonicalBundleIdForComparison(id));
    }
    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: unresolved },
      });
      const result = mergeIconResolutions(appIconPaths, icons);
      if (result.changed) appIconPaths = result.iconPathsByBundleId;
    } catch {
      // Icons are decorative; the letter fallback keeps working.
    }
  }

  function appIconSrc(app: string | null | undefined): string | null {
    if (!app) return null;
    const iconPath = iconPathForBundleId(app, appIconPaths);
    return iconPath ? convertFileSrc(iconPath) : null;
  }

  $effect(() => {
    void resolveAppIcons(intervals.map((interval) => appLabel(interval.app)));
  });
</script>

{#if intervals.length > 0}
  <div class="timeline">
    {#if title}
      <div class="timeline-title">{title}</div>
    {/if}
    <ol class="rail">
      {#each intervals as interval, i (i)}
        <li class="row">
          <span class="spine" aria-hidden="true">
            <span class="dot" style="color:var({colorVarFor(interval.category)});background:currentColor;"></span>
          </span>
          <span class="body">
            <span class="time">{timeRange(interval)}</span>
            <span class="label">
              <span class="label-text">{interval.label}</span>
              {#if appLabel(interval.app)}
                {@const label = appLabel(interval.app)}
                <span class="app-chip">
                  <span class="app-chip-icon" aria-hidden="true">
                    {#if appIconSrc(label) !== null}
                      <img src={appIconSrc(label)} alt="" />
                    {:else}
                      {appIconFallback(label, label)}
                    {/if}
                  </span>
                  <span class="app-chip-name">{label}</span>
                </span>
              {/if}
            </span>
          </span>
        </li>
      {/each}
    </ol>
  </div>
{/if}

<style>
  .timeline {
    display: flex;
    flex-direction: column;
  }
  .timeline-title {
    font-size: 10.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    margin: 0 0 10px;
  }
  .rail {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }
  .row {
    display: grid;
    grid-template-columns: 16px 1fr;
    gap: 11px;
    align-items: stretch;
  }
  /* The spine is the vertical rail: a thin centred line that the per-row dot
     sits on. ::before draws the connector, the dot caps it for that row. */
  .spine {
    position: relative;
    display: block;
    width: 16px;
  }
  .spine::before {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 1px;
    transform: translateX(-50%);
    background: var(--app-border);
  }
  /* Top of the first row and bottom of the last row taper the spine so it reads
     as a contained rail rather than running off the edges. */
  .row:first-child .spine::before {
    top: 9px;
  }
  .row:last-child .spine::before {
    bottom: calc(100% - 9px);
  }
  .dot {
    position: absolute;
    top: 5px;
    left: 50%;
    width: 8px;
    height: 8px;
    border-radius: 999px;
    transform: translateX(-50%);
    /* Inner surface ring lifts the dot off the spine; the faint outer ring is the
       dot's own (category) colour, set via `currentColor` from the inline color. */
    box-shadow:
      0 0 0 2px var(--app-surface-subtle),
      0 0 0 4px color-mix(in srgb, currentColor 22%, transparent);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding-bottom: 15px;
  }
  .row:last-child .body {
    padding-bottom: 0;
  }
  .time {
    font-size: 10.5px;
    font-weight: 500;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.02em;
  }
  .label {
    display: flex;
    align-items: baseline;
    gap: 7px;
    min-width: 0;
    flex-wrap: wrap;
  }
  .label-text {
    font-size: 12.5px;
    color: var(--app-text);
    line-height: 1.45;
    min-width: 0;
  }
  .app-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
    max-width: 100%;
    min-width: 0;
    font-size: 9.5px;
    color: var(--app-text-muted);
    padding: 1px 6px 1px 2px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    white-space: nowrap;
  }
  .app-chip-icon {
    display: grid;
    width: 13px;
    height: 13px;
    flex: 0 0 13px;
    place-items: center;
    overflow: hidden;
    border-radius: 4px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: 8px;
    font-weight: 800;
    line-height: 1;
  }
  .app-chip-icon img {
    width: 13px;
    height: 13px;
    object-fit: contain;
  }
  .app-chip-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
