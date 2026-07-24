<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // InsightsShell — the shared rail+main shell of the story-first surfaces
  // (Warm Paper redesign, Slice 2). Extracted from routes/insights/+page.svelte
  // so BOTH the /insights route (Today / Meetings / Subjects / Chat) and the
  // /triggers route render inside the SAME rail + shell layout instead of
  // /triggers carrying its own page shell.
  //
  // The shell owns: the engine-status load for the rail footer, the optional
  // whole-surface engine gate, the derivation-off steering, rail collapse /
  // expand, and rail drag-resize persistence. The owning route passes the
  // active `view` + an `onOpenTab` handler and renders its content as
  // `children` in the main column.
  import { untrack, type Snippet } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettings } from "$lib/surface-windows";
  import type {
    AiRuntimeStatus,
    UserContextStatus,
    RecordingSettings,
  } from "$lib/types/recording";
  import InsightsRail, { type RailTab } from "$lib/insights/InsightsRail.svelte";
  import RailResizer from "$lib/insights/RailResizer.svelte";
  import ActivityReceipt from "$lib/insights/ActivityReceipt.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import { receiptDrawer } from "$lib/insights/receiptDrawer.svelte";

  interface Props {
    view: RailTab;
    onOpenTab: (tab: RailTab) => void;
    // Whole-surface engine gate. The /insights route passes true (every
    // sub-surface there is engine-derived); /triggers passes false — triggers
    // render their own provider-not-ready states and must stay reachable.
    gate?: boolean;
    // Edge-to-edge main column (Chat and Triggers own their scroll + padding).
    bare?: boolean;
    children: Snippet;
  }

  let { view, onOpenTab, gate = false, bare = false, children }: Props = $props();

  // ── Engine status ────────────────────────────────────────────────────
  // Passed down to the rail's footer (<RailFooter> via <InsightsRail>), which
  // renders "engine · <model>" when the Reasoning Engine is on/available, or
  // "engine off · Enable" otherwise.
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let modelLabel = $state<string>("");
  // Distinguishes "still loading the status calls" from "loaded → engine off".
  let statusLoaded = $state(false);

  const engineOn = $derived(
    Boolean(aiStatus?.enabled && aiStatus?.available) ||
      Boolean(ctxStatus?.engineAvailable),
  );

  // Whole-page gate: keyed on the user's SETUP state (enabled && configured),
  // NOT on `available` — a configured engine that is momentarily unreachable
  // keeps the page and its per-surface error states. Only asserted after
  // `statusLoaded` so the page never flashes the gate while loading.
  const engineGated = $derived(
    statusLoaded && !(aiStatus?.enabled && aiStatus?.configured),
  );

  // Continuous-derivation lock: the runtime is set up but the User Context
  // opt-in is off. Today / Subjects are rendered FROM derivation output, so
  // the rail locks them (tooltip + click-through to the derivation setting).
  // Chat, Meetings, and Triggers stay live.
  const derivationOff = $derived(
    statusLoaded && ctxStatus?.reason === "user_context_disabled",
  );

  // While derivation is off the locked tabs are unreachable via the rail, but
  // `view` can still point at one (default "today", or derivation turned off
  // while on a locked tab) — steer to Chat via the owner's handler.
  $effect(() => {
    if (derivationOff && (view === "today" || view === "subjects")) {
      onOpenTab("chat");
    }
  });

  function openDerivationSettings(): void {
    void openSettings("userContext");
  }

  function shortModel(model: string): string {
    const trimmed = model.trim();
    if (!trimmed) return "engine";
    // Drop a leading "provider:" prefix and any path, keep the model id tail.
    const afterProvider = trimmed.includes(":") ? trimmed.split(":").pop()! : trimmed;
    const tail = afterProvider.split("/").pop() ?? afterProvider;
    return tail.length > 28 ? `${tail.slice(0, 27)}…` : tail;
  }

  async function loadEngineStatus(): Promise<void> {
    try {
      const [ai, ctx, settings] = await Promise.all([
        invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
        invoke<UserContextStatus>("get_user_context_status").catch(() => null),
        invoke<RecordingSettings>("get_recording_settings").catch(() => null),
      ]);
      aiStatus = ai;
      ctxStatus = ctx;
      if (settings?.aiRuntime) {
        modelLabel = shortModel(settings.aiRuntime.defaultModel?.model ?? "");
      }
    } catch {
      // Best-effort: leave the pill in its "engine off" default on error.
    } finally {
      statusLoaded = true;
    }
  }

  function enableEngine(): void {
    void openSettings("intelligence");
  }

  // ── Rail collapse / expand ───────────────────────────────────────────────
  // Two independent inputs decide the EFFECTIVE collapsed state:
  //   • userCollapsed — the user's EXPLICIT preference, persisted.
  //   • windowNarrow  — a TRANSIENT, automatic collapse on narrow windows.
  // Effective = userCollapsed || windowNarrow, so an auto-collapse never
  // clobbers the user's saved choice.
  const RAIL_COLLAPSED_KEY = "mnema.insights.rail-collapsed";
  const NARROW_PX = 760;

  function readPersistedCollapsed(): boolean {
    try {
      return localStorage.getItem(RAIL_COLLAPSED_KEY) === "1";
    } catch {
      return false;
    }
  }

  let userCollapsed = $state(readPersistedCollapsed());
  let windowNarrow = $state(false);
  const railCollapsed = $derived(userCollapsed || windowNarrow);

  function toggleRailCollapsed(): void {
    userCollapsed = !railCollapsed;
    try {
      localStorage.setItem(RAIL_COLLAPSED_KEY, userCollapsed ? "1" : "0");
    } catch {
      // Best-effort persistence.
    }
  }

  // ── Rail width (drag-resize) ─────────────────────────────────────────────
  // The shell is the single owner that clamps + persists, so storage never
  // holds an out-of-range value.
  const RAIL_WIDTH_KEY = "mnema.insights.rail-width";
  const RAIL_MIN_WIDTH = 180;
  const RAIL_MAX_WIDTH = 400;
  const RAIL_DEFAULT_WIDTH = 240;

  function clampRailWidth(px: number): number {
    return Math.min(RAIL_MAX_WIDTH, Math.max(RAIL_MIN_WIDTH, Math.round(px)));
  }

  function readPersistedWidth(): number {
    try {
      const raw = localStorage.getItem(RAIL_WIDTH_KEY);
      if (raw === null) return RAIL_DEFAULT_WIDTH;
      const parsed = Number.parseInt(raw, 10);
      return Number.isNaN(parsed) ? RAIL_DEFAULT_WIDTH : clampRailWidth(parsed);
    } catch {
      return RAIL_DEFAULT_WIDTH;
    }
  }

  let railWidth = $state(readPersistedWidth());

  function setRailWidth(px: number): void {
    railWidth = clampRailWidth(px);
    try {
      localStorage.setItem(RAIL_WIDTH_KEY, String(railWidth));
    } catch {
      // Best-effort persistence.
    }
  }

  function resetRailWidth(): void {
    setRailWidth(RAIL_DEFAULT_WIDTH);
  }

  // Track the narrow-window condition with a matchMedia listener.
  $effect(() => {
    if (typeof window === "undefined" || !window.matchMedia) return;
    const mql = window.matchMedia(`(max-width: ${NARROW_PX - 1}px)`);
    const apply = () => {
      windowNarrow = mql.matches;
    };
    apply();
    mql.addEventListener("change", apply);
    return () => mql.removeEventListener("change", apply);
  });

  $effect(() => {
    void untrack(() => loadEngineStatus());
    // Kick the shared store's first history fetch so the rail populates on
    // whichever shell route mounts first (idempotent).
    void conversationStore.ensureStarted();

    let unlisten: UnlistenFn | undefined;
    let unlistenSettings: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // Settings saves (default model / engine on-off) emit this, not
    // `user_context_changed`; refresh the engine pill so it doesn't stay stale.
    void listen("recording_settings_changed", () => {
      void loadEngineStatus();
    }).then((fn) => {
      if (disposed) fn();
      else unlistenSettings = fn;
    });

    return () => {
      disposed = true;
      unlisten?.();
      unlistenSettings?.();
    };
  });
</script>

{#if gate && engineGated}
  <!-- Engine never set up — the whole workspace is engine-derived, so pitch it.
       `recording_settings_changed` re-runs loadEngineStatus, so finishing setup
       in Settings unlocks this page live, no reload needed. -->
  <div class="gate">
    <div class="gate-panel">
      <p class="gate-eyebrow">
        <span class="diamond" aria-hidden="true">◆</span>
        Insights
      </p>
      <h1 class="gate-title">Turn on the Reasoning Engine to unlock Insights.</h1>
      <p class="gate-detail">
        Insights is what the engine writes about your days — everything on this
        surface is derived from it:
      </p>
      <ul class="gate-list">
        <li><strong>Today</strong> — your day reconstructed as a readable journal.</li>
        <li><strong>Meetings</strong> — every detected meeting with its recap and transcript.</li>
        <li><strong>Subjects</strong> — the views it forms about you, with confidence trajectories.</li>
        <li><strong>Chat</strong> — ask questions over your own history.</li>
      </ul>
      <button type="button" class="gate-cta" onclick={enableEngine}>
        Open engine settings
      </button>
      <p class="gate-note">
        Runs on your own provider — local (Ollama, Llamafile) or your cloud API key.
      </p>
    </div>
  </div>
{:else}
<div class="insights" class:insights--collapsed={railCollapsed}>
  <InsightsRail
    {view}
    {onOpenTab}
    {derivationOff}
    onOpenDerivationSettings={openDerivationSettings}
    {engineOn}
    {modelLabel}
    {statusLoaded}
    onEnable={enableEngine}
    collapsed={railCollapsed}
    onToggleCollapse={toggleRailCollapsed}
    width={railWidth}
  />

  <!-- Drag handle between the rail and the active sub-surface. Only present when
       the rail is (so there is a boundary to drag). -->
  {#if !railCollapsed}
    <RailResizer
      width={railWidth}
      min={RAIL_MIN_WIDTH}
      max={RAIL_MAX_WIDTH}
      onWidth={setRailWidth}
      onReset={resetRailWidth}
    />
  {/if}

  <main class="insights-main" class:insights-main--bare={bare}>
    <!-- When the rail is collapsed, a quiet floating button (top-left, with a
         subtle backdrop so it reads above sub-surface content) brings it back. -->
    {#if railCollapsed}
      <button
        type="button"
        class="rail-expand-float"
        aria-label="Expand sidebar"
        aria-expanded="false"
        use:tip={"Expand sidebar"}
        onclick={toggleRailCollapsed}
      >
        <span aria-hidden="true">»</span>
      </button>
    {/if}
    {@render children()}
  </main>
</div>
{/if}

<!-- Span-scoped receipt drawer host (Slice 4). Any surface under the shell
     opens it via receiptDrawer.open(activity); the drawer renders as a fixed
     right-side panel over rail + main, so it lives here rather than in each
     surface. Esc / scrim / ✕ dismissal and playback teardown are owned by
     <ActivityReceipt> itself. -->
{#if receiptDrawer.current}
  <ActivityReceipt activity={receiptDrawer.current} onClose={() => receiptDrawer.close()} />
{/if}

<style>
  /* Shared rail+main shell — token-driven. A persistent left rail
     (<InsightsRail>) sits beside the `.insights-main` scroll column. */
  .insights {
    display: flex;
    flex-direction: row;
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
  }

  /* ── Engine gate — full-surface pitch shown until the engine is set up ── */
  .gate {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    overflow-y: auto;
    padding: 28px 20px;
  }
  .gate-panel {
    /* Auto margins center when there's room but keep the top reachable when the
       panel is taller than the viewport (flex centering would clip it). */
    margin: auto;
    max-width: 460px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 26px 28px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 11px;
  }
  .gate-eyebrow {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: var(--text-xs);
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .gate-eyebrow .diamond {
    color: var(--app-accent);
    letter-spacing: 0;
  }
  .gate-title {
    margin: 0;
    font-size: var(--text-lg);
    line-height: 1.35;
    color: var(--app-text-strong);
  }
  .gate-detail {
    margin: 0;
    font-size: var(--text-md);
    line-height: 1.6;
    color: var(--app-text-muted);
  }
  .gate-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--text-md);
    line-height: 1.55;
    color: var(--app-text-muted);
  }
  /* Hanging indent — wrapped lines align under the text, not the ◆ marker. */
  .gate-list li {
    position: relative;
    padding-left: 16px;
  }
  .gate-list li::before {
    content: "◆";
    position: absolute;
    left: 0;
    font-size: 8px;
    color: var(--app-accent);
    vertical-align: 1px;
  }
  .gate-list strong {
    color: var(--app-text-strong);
    font-weight: 600;
  }
  .gate-cta {
    align-self: flex-start;
    margin-top: 8px;
    font: inherit;
    font-size: var(--text-md);
    padding: 7px 15px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .gate-cta:hover {
    border-color: var(--app-accent);
  }
  .gate-cta:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .gate-cta:active {
    transform: translateY(1px);
  }
  .gate-note {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--app-text-faint);
  }

  .insights-main {
    flex: 1 1 auto;
    min-width: 0;
    /* Position context for the floating expand button (collapsed state). */
    position: relative;
    overflow-y: auto;
    /* Reading surfaces never scroll sideways; a stray wide element (long
       unwrapped token, 1px rounding) must not summon a horizontal scrollbar. */
    overflow-x: hidden;
    padding: 18px 20px 28px;
  }
  /* When the rail is collapsed, the padded sub-surfaces reserve a little extra
     top-left room so the floating expand button never sits on top of their
     content. Bare surfaces (Chat, Triggers) own their own padding, so they
     keep the edge-to-edge layout (the button's backdrop separates it). */
  .insights--collapsed .insights-main:not(.insights-main--bare) {
    padding-top: 46px;
  }

  /* Floating expand affordance — only rendered when the rail is collapsed. */
  .rail-expand-float {
    position: absolute;
    top: 12px;
    left: 12px;
    z-index: 5;
    width: 26px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .rail-expand-float:hover {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-surface-hover);
  }
  .rail-expand-float:focus-visible {
    outline: none;
    color: var(--app-accent);
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }
  /* Bare surfaces (Chat, Triggers) own their full-height, edge-to-edge layout
     and internal scrolling, so the shell main drops its padding and outer
     scroll. Flex column so the surface fills via flex-grow — WKWebView does
     not reliably resolve a child's `height: 100%` against a flex-stretched
     parent. */
  .insights-main--bare {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
</style>
