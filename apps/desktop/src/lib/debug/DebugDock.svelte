<script lang="ts">
  // Floating icon dock — left, vertically centered, frosted pill (mockup A).
  //
  // Icon-only navigation: one health dot per icon, hover tooltips via the shared
  // `tip` action (NOT the mockup's ::after/data-tip, which can't escape the
  // dock's stacking context and wouldn't match the app's tooltip styling), a
  // separator between feature groups, and a live-poll pulse at the bottom.
  //
  // A click scrolls the summary section into view; `activeId` is fed by the
  // shell's scroll-spy so the dock tracks where the user actually is.

  import { tip } from "$lib/components/tooltip";
  import { DEBUG_SECTIONS, type DebugSectionId } from "./sections";
  import type { HealthStore } from "./state/health.svelte";
  import type { DebugSeverity } from "$lib/types";

  interface Props {
    health: HealthStore;
    activeId: DebugSectionId;
    onselect: (id: DebugSectionId) => void;
  }

  let { health, activeId, onselect }: Props = $props();

  /** Health severity → dot modifier. `null` (no rollup for this section, or not
      loaded yet) reads as idle — see sections.ts on the 9-vs-11 mismatch. */
  function dotClass(severity: DebugSeverity | null): string {
    if (severity === "ok") return "health-dot health-dot--ok";
    if (severity === "warn") return "health-dot health-dot--warn";
    if (severity === "error") return "health-dot health-dot--err";
    return "health-dot health-dot--idle";
  }

  /** Tooltip text: the section label, plus the backend's plain-language reason. */
  function tipText(label: string, reason: string | null): string {
    return reason ? `${label} — ${reason}` : label;
  }
</script>

<nav class="debug-dock" aria-label="Debug sections">
  {#each DEBUG_SECTIONS as section, i (section.id)}
    {#if i > 0 && DEBUG_SECTIONS[i - 1].group !== section.group}
      <div class="debug-dock__sep" aria-hidden="true"></div>
    {/if}
    {@const Icon = section.icon}
    {@const severity = health.severityFor(section.healthFeature)}
    <button
      type="button"
      class="debug-dock__item"
      class:debug-dock__item--active={activeId === section.id}
      aria-label={section.label}
      aria-current={activeId === section.id ? "true" : undefined}
      use:tip={tipText(section.label, health.reasonFor(section.healthFeature))}
      onclick={() => onselect(section.id)}
    >
      <Icon />
      <span class={dotClass(severity)} aria-hidden="true"></span>
    </button>
  {/each}

  <div class="debug-dock__sep" aria-hidden="true"></div>
  <div class="debug-dock__live" use:tip={health.error ? `Live poll failing — ${health.error}` : "Live · 1s poll"}>
    <span class="debug-dock__live-dot" class:debug-dock__live-dot--err={health.error != null}></span>
  </div>
</nav>
