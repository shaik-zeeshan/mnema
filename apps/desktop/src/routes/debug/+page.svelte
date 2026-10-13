<script lang="ts">
  // Developer window shell (docs/mockups/redesign/developer.html): strip,
  // verdict, five tabs and one scrolling body — or the mini view. This file
  // only builds the controller, runs the health poll (both modes) and routes
  // Esc inside drill-ins. Each tab starts its own polls via `controller.poll`.
  //
  // Gating is unchanged: `+layout.svelte` gates direct visits to /debug behind
  // developer options, and closes the window on a top-level Esc.

  import "$lib/debug/developer.css";
  import Segmented from "$lib/components/Segmented.svelte";
  import DevStrip from "$lib/debug/DevStrip.svelte";
  import Verdict from "$lib/debug/Verdict.svelte";
  import MiniView from "$lib/debug/MiniView.svelte";
  import OverviewTab from "$lib/debug/tabs/OverviewTab.svelte";
  import PipelineTab from "$lib/debug/tabs/PipelineTab.svelte";
  import CaptureTab from "$lib/debug/tabs/CaptureTab.svelte";
  import AiTab from "$lib/debug/tabs/AiTab.svelte";
  import LogsTab from "$lib/debug/tabs/LogsTab.svelte";
  import { createDebugController, setDebugController, type DevTab } from "$lib/debug/state/controller.svelte";

  const controller = createDebugController();
  setDebugController(controller);
  const { nav, health } = controller;

  const TABS: { value: DevTab; label: string }[] = [
    { value: "overview", label: "Overview" },
    { value: "pipeline", label: "Pipeline" },
    { value: "capture", label: "Capture" },
    { value: "ai", label: "AI" },
    { value: "logs", label: "Logs" },
  ];

  // The verdict (full) and the dots (mini) both read it, so it runs in both modes.
  $effect(() => health.startPolling());

  // Esc in a drill-in goes back one level. Capture phase, so it runs before the
  // layout's close-on-Escape, which ignores a `defaultPrevented` event.
  $effect(() => {
    const onKeydown = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
      if (nav.processor == null && nav.jobId == null) return;
      event.preventDefault();
      controller.back();
    };
    window.addEventListener("keydown", onKeydown, true);
    return () => window.removeEventListener("keydown", onKeydown, true);
  });
</script>

{#if controller.mode === "mini"}
  <DevStrip compact><MiniView /></DevStrip>
{:else}
  <div class="dv-win" data-at={nav.tab}>
    <DevStrip />
    <Verdict />
    <div class="dv-tabs">
      <Segmented
        options={TABS}
        value={nav.tab}
        ariaLabel="Section"
        onValueChange={(tab) => controller.go(tab as DevTab)}
      />
    </div>
    <div class="dv-body">
      {#if nav.tab === "overview"}
        <OverviewTab />
      {:else if nav.tab === "pipeline"}
        <PipelineTab />
      {:else if nav.tab === "capture"}
        <CaptureTab />
      {:else if nav.tab === "ai"}
        <AiTab />
      {:else}
        <LogsTab />
      {/if}
    </div>
  </div>
{/if}
