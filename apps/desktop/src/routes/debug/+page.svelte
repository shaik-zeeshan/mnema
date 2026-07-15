<script lang="ts">
  // Debug shell — slice-5 restructure.
  //
  // The five `{#if activeTab === ...}` panels (Overview / Capture / Inactivity /
  // Pipeline / System) and the top tab strip are gone. The page is now a summary
  // scroll: one card per feature in dock order, with a floating icon dock on the
  // left whose health dots come from `get_debug_health` and whose clicks scroll
  // to a section. See lib/debug/sections.ts for the registry that drives both.
  //
  // This shell is thin: it builds the single DebugController (shared with every
  // section via context), runs the mount/polling effects, tracks which section
  // is on screen for the dock's active state, and renders the sections. All
  // state, loaders and helpers live in lib/debug/state/* and lib/debug/format.ts.
  //
  // Slice 7 gave it a second thing to render: a feature detail is a view PUSHED
  // within this page (not a route), so the scroll is swapped for FeatureDetail
  // while `detail.feature` is set — and every summary poller below is gated on
  // that, since none of their cards are on screen. The dock is the exception: it
  // stays visible in both levels, so its health poll keeps running.
  //
  // Gating is unchanged: `+layout.svelte` gates direct visits to /debug behind
  // developer options.

  import { onDestroy, tick, untrack } from "svelte";
  import { DEBUG_SECTIONS, anchor, type DebugSectionId } from "$lib/debug/sections";
  import { createDebugController, setDebugController } from "$lib/debug/state/controller.svelte";
  // Shared `.debug-shell` styles, split per concern (≤800 lines each), imported
  // in SOURCE ORDER (cascade-critical). Map: debug-layout.css.
  import "$lib/debug/debug-layout.css";
  import "$lib/debug/debug-controls.css";
  import "$lib/debug/debug-capture.css";
  import "$lib/debug/debug-logs.css";
  import "$lib/debug/debug-features.css";
  import "$lib/debug/debug-cards.css";
  import "$lib/debug/debug-detail.css";
  import DebugDock from "$lib/debug/DebugDock.svelte";
  import FeatureDetail from "$lib/debug/detail/FeatureDetail.svelte";
  import HealthSection from "$lib/debug/sections/HealthSection.svelte";
  import CaptureSourcesSection from "$lib/debug/sections/CaptureSourcesSection.svelte";
  import PrivacyInactivitySection from "$lib/debug/sections/PrivacyInactivitySection.svelte";
  import OcrSection from "$lib/debug/sections/OcrSection.svelte";
  import TranscriptionSection from "$lib/debug/sections/TranscriptionSection.svelte";
  import DiarizationSection from "$lib/debug/sections/DiarizationSection.svelte";
  import EmbeddingsSection from "$lib/debug/sections/EmbeddingsSection.svelte";
  import AiRuntimeSection from "$lib/debug/sections/AiRuntimeSection.svelte";
  import UserContextSection from "$lib/debug/sections/UserContextSection.svelte";
  import JobsStorageSection from "$lib/debug/sections/JobsStorageSection.svelte";
  import LogsSection from "$lib/debug/sections/LogsSection.svelte";

  const controller = createDebugController();
  setDebugController(controller);
  const { capture, pipeline, health, features, detail } = controller;

  let scrollRegion = $state<HTMLDivElement | null>(null);
  let activeId = $state<DebugSectionId>("health");

  async function scrollTo(id: DebugSectionId) {
    // Set eagerly so the dock responds to the click even before the smooth
    // scroll settles and the observer catches up.
    activeId = id;
    // The dock is live in a detail view too, so a click there is also "back".
    // The summary sections don't exist until it re-renders — hence the tick.
    if (detail.isOpen) {
      detail.close();
      await tick();
    }
    document.getElementById(anchor(id))?.scrollIntoView({ behavior: "smooth", block: "start" });
  }

  /** Breadcrumb / Esc: pop the detail and land back on the card it came from. */
  async function backToSummary() {
    const id = detail.feature;
    detail.close();
    if (!id) return;
    activeId = id;
    await tick();
    document.getElementById(anchor(id))?.scrollIntoView({ block: "start" });
  }

  // ─── Mount ────────────────────────────────────────────────────────────────
  // The loaders write state that they also read back, so the whole block is
  // untracked — otherwise this effect would re-run on its own writes and
  // clobber in-flight state (see settings' mount-untrack precedent).
  $effect(() => {
    untrack(() => {
      capture.loadSettings();
      capture.loadPermissions();
      // The Health board reads capture support on open (platform + per-source
      // capability), so load it eagerly instead of waiting for the manual
      // System-probe "Query" button.
      capture.loadSupport();
      pipeline.fetchInfraStatus();
      pipeline.fetchJobs();
      // The feature cards' config half (engine status, model install state,
      // Deepgram auth). Deliberately not on the 1s tick — those reads ping the
      // local engine endpoint and stat model files. See state/features.svelte.ts.
      features.loadConfig();
    });
    return capture.initListeners();
  });

  // ─── Polling ──────────────────────────────────────────────────────────────
  // Each poller lives in its own $effect so that its reactive deps never force
  // a re-run of the mount effect above. Every one stops when the document is
  // hidden (checked inside the store's fetch) and is cleaned up on unmount.
  //
  // The summary's pollers are additionally gated on `!detail.isOpen`: a pushed
  // detail view renders none of these cards and polls its own feature itself
  // (state/detail.svelte.ts), so leaving them running would be a dozen
  // round-trips a second feeding nothing. Reading `detail.isOpen` re-runs each
  // effect on the push, which tears its interval down.

  // The dock — the one surface that survives the push, so this poll does too.
  $effect(() => health.startPolling());

  $effect(() => {
    if (detail.isOpen) return;
    return capture.startIdlePolling();
  });
  $effect(() => {
    if (detail.isOpen) return;
    return pipeline.startOcrBudgetPolling();
  });
  // One tick for all five feature cards' live half (job lanes, semantic index,
  // derivation runs, Ask AI usage) — they share a data set, so one poll keeps
  // them coherent with each other.
  $effect(() => {
    if (detail.isOpen) return;
    return features.startPolling();
  });

  // Session reconciliation: only poll while the UI thinks we're recording.
  $effect(() => {
    if (!capture.isCapturing || detail.isOpen) return;
    return capture.startReconcilePolling();
  });

  // Keep the dock pointing at the pushed feature — the scroll spy can't, since
  // the sections it observes are unmounted while a detail view is open.
  $effect(() => {
    const pushed = detail.feature;
    if (pushed) activeId = pushed;
  });

  $effect(() => capture.startWakeResync());

  // Clamp the jobs/admission/execution pagers when their lists shrink. Lives
  // here rather than in a section because the pagers it guards are split across
  // two of them (OCR and Jobs & Storage).
  $effect(() => pipeline.clampPages());

  // ─── Scroll spy ───────────────────────────────────────────────────────────
  // Drives the dock's active state from what is actually on screen. The
  // top-most intersecting section wins, so scrolling past a short card doesn't
  // leave the dock pointing at the previous one.
  $effect(() => {
    const root = scrollRegion;
    if (!root || typeof IntersectionObserver === "undefined") return;
    const observer = new IntersectionObserver(
      (entries) => {
        const visible = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top)[0];
        if (!visible) return;
        const id = visible.target.id.replace(/^debug-section-/, "");
        activeId = id as DebugSectionId;
      },
      { root, rootMargin: "0px 0px -60% 0px", threshold: 0 }
    );
    for (const section of DEBUG_SECTIONS) {
      const el = document.getElementById(anchor(section.id));
      if (el) observer.observe(el);
    }
    return () => observer.disconnect();
  });

  // Clean up any in-flight post-submit poll when the page is destroyed.
  onDestroy(() => pipeline.stopPostSubmitPolling());
</script>

<!-- Layout is mockup A's: a fixed dock, then ONE scroll region whose left
     padding is the dock's gutter, holding ONE centered `.debug-panel` column.
     The page head lives inside that panel so it aligns with the cards rather
     than with the window. Both levels (summary scroll and pushed detail) render
     into the same panel, so the column width never jumps on drill-in. -->
<div class="debug-shell">
  <DebugDock {health} {activeId} onselect={scrollTo} />

  {#if detail.feature}
    <!-- Level 2, pushed in place of the scroll: same page, same dock. -->
    <div class="debug-scroll">
      <div class="debug-panel">
        <header class="debug-head">
          <h1 class="debug-head__title">Debug</h1>
          <span class="debug-head__meta">live · 1s poll</span>
        </header>
        <FeatureDetail feature={detail.feature} onback={backToSummary} />
      </div>
    </div>
  {:else}
  <div class="debug-scroll" bind:this={scrollRegion}>
    <div class="debug-panel">
      <header class="debug-head">
        <h1 class="debug-head__title">Debug</h1>
        <span class="debug-head__meta">live · 1s poll</span>
      </header>

      <HealthSection />
      <CaptureSourcesSection />
      <PrivacyInactivitySection />
      <OcrSection />

      <TranscriptionSection />
      <DiarizationSection />
      <EmbeddingsSection />
      <AiRuntimeSection />
      <UserContextSection />

      <JobsStorageSection />

      <LogsSection />

      <!-- ── Error display ────────────────────────────────────────────── -->
      {#if capture.lastError}
        <section class="debug-error-card" role="alert" aria-live="assertive">
          <h2 class="debug-error-card__title">
            Error
            <button class="btn btn--ghost btn--sm debug-block__action" onclick={() => (capture.lastError = null)}>dismiss</button>
          </h2>
          <pre class="error-pre">{capture.lastError}</pre>
        </section>
      {/if}
    </div>
  </div>
  {/if}
</div>
