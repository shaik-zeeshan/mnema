<script lang="ts">
  // Insights — the second top-level surface of Main (insights.html). A sticky
  // `.in-bar` of four underline tabs (Overview · Journal · Subjects · Context)
  // switched via local state, with the active view's controls on the same row
  // (views move them there with `use:inBar`), plus a Subject drill-in. Engine
  // trouble is one `mx-notice` over the view — never a wall, never a steer.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { openSettings } from "$lib/surface-windows";
  import type {
    AiRuntimeSettings,
    AiRuntimeStatus,
    AuthoredContext,
    UserContextStatus,
    RecordingSettings,
  } from "$lib/types/recording";
  import { engineState } from "$lib/insights/engine-state";
  import { aiProviderInstanceLabel, aiProviderKindLabel } from "$lib/settings/state/ai-providers";
  import Overview from "$lib/insights/Overview.svelte";
  import DayTimeline from "$lib/insights/DayTimeline.svelte";
  import Subjects from "$lib/insights/Subjects.svelte";
  import SubjectDetail from "$lib/insights/SubjectDetail.svelte";
  import Context from "$lib/insights/Context.svelte";
  import IconOff from "~icons/lucide/circle-off";
  import IconSparkle from "~icons/lucide/sparkle";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconRetry from "~icons/lucide/rotate-ccw";

  type InsightsTab = "overview" | "journal" | "subjects" | "context";

  let view = $state<InsightsTab>("overview");
  // Subject drill-in over the Subjects tab (null = the index).
  let selectedSubject = $state<string | null>(null);
  let main = $state<HTMLElement>();

  function openTab(tab: InsightsTab): void {
    view = tab;
    contextOpenDismissed = false;
    if (tab !== "subjects") selectedSubject = null;
    if (main) main.scrollTop = 0;
  }

  function openSubject(subject: string): void {
    view = "subjects";
    selectedSubject = subject;
    if (main) main.scrollTop = 0;
  }

  // Subjects' "View dismissed": switch to Context with the archive open.
  let contextOpenDismissed = $state(false);
  function viewDismissed(): void {
    openTab("context");
    contextOpenDismissed = true;
  }

  // ── Engine status → one notice (IMPLEMENTATION.md §4 Shell) ──────────
  let aiStatus = $state<AiRuntimeStatus | null>(null);
  let aiSettings = $state<AiRuntimeSettings | null>(null);
  let ctxStatus = $state<UserContextStatus | null>(null);
  let authoredCount = $state<number | undefined>(undefined);
  // Nothing flashes a notice before the first status read lands.
  let statusLoaded = $state(false);

  const engine = $derived(engineState(statusLoaded, aiStatus, aiSettings));

  async function loadEngineStatus(): Promise<void> {
    const [ai, ctx, settings, authored] = await Promise.all([
      invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null),
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
      invoke<RecordingSettings>("get_recording_settings").catch(() => null),
      invoke<AuthoredContext[]>("list_user_context_authored").catch(() => null),
    ]);
    aiStatus = ai;
    ctxStatus = ctx;
    authoredCount = authored?.length;
    aiSettings = settings?.aiRuntime ?? null;
    statusLoaded = true;
  }

  function providerLabel(id: string): string {
    const provider = aiSettings?.providers.find((p) => p.id === id);
    return provider ? aiProviderInstanceLabel(provider) : aiProviderKindLabel(id);
  }

  type Notice = {
    tone: "off" | "warn" | "danger" | "";
    icon: typeof IconOff;
    title: string;
    /** Per-view body copy, or one line for every view. */
    text: Record<InsightsTab, string> | string;
    action: string;
    ghost: boolean;
    onAction: () => void;
  };

  const toEngine = () => void openSettings("intelligence");

  // Reason-keyed copy (insights.html ENGINE). `has_providers` picks "not set up"
  // over "off"; off-by-choice is never a warning, unreachable never says sign in.
  const notice = $derived.by((): Notice | null => {
    if (engine.kind === "pitch") {
      return {
        tone: "",
        icon: IconSparkle,
        title: "AI isn't set up yet",
        text: {
          overview: "Connect a provider — local (Ollama, Llamafile) or your own API key — to get the read, categories and focus.",
          journal: "The journal is written by your AI provider. Until one is connected, the Timeline has everything captured.",
          subjects: "Subjects are views your AI provider forms from your activity.",
          context: "You can write context now — it's saved and used once AI is set up.",
        },
        action: "Set up AI",
        ghost: false,
        onAction: toEngine,
      };
    }
    if (engine.kind === "off") {
      return {
        tone: "off",
        icon: IconOff,
        title: "AI features are off",
        text: {
          overview: "You're seeing capture-only numbers. The read, categories and focus come back when AI features are on.",
          journal: "Nothing new is written up while they're off. Days already written stay as they are.",
          subjects: "Subjects stop updating while AI features are off. Nothing here is lost.",
          context: "What you write is saved, and steers the dossier again once AI features are back on.",
        },
        action: "Turn on in Settings →",
        ghost: true,
        onAction: toEngine,
      };
    }
    if (engine.kind === "unreachable") {
      return {
        tone: "warn",
        icon: IconCloudOff,
        // "Can't reach ChatGPT right now — your sign-in is fine." → the head.
        title: engine.text.split(" — ")[0],
        text: "New write-ups resume on their own once it's reachable. Everything already here stays.",
        action: "Try again",
        ghost: false,
        onAction: () => void loadEngineStatus(),
      };
    }
    if (engine.kind === "fix") {
      const id = engine.reconnectProviderId;
      return {
        tone: "danger",
        icon: IconAlert,
        title: id ? `${providerLabel(id)} needs you to sign in again` : engine.text,
        text: id
          ? "OpenAI rejected the saved sign-in, so new write-ups are paused. Everything already here stays."
          : "New write-ups are paused. Everything already here stays.",
        action: id ? "Reconnect" : "Open settings",
        ghost: false,
        onAction: toEngine,
      };
    }
    if (engine.kind === "on" && ctxStatus?.reason === "user_context_disabled") {
      return {
        tone: "off",
        icon: IconOff,
        title: "Continuous derivation is off",
        text: {
          overview: "No new activities, read or focus while it's off — the numbers below are capture-only.",
          journal: "New capture isn't written up while it's off. Days already written stay as they are.",
          subjects: "No new evidence reaches your subjects while it's off. Confidence keeps fading on its own.",
          context: "What you write is saved, and steers the dossier once derivation is back on.",
        },
        action: "Turn on in Settings →",
        ghost: true,
        onAction: () => void openSettings("userContext"),
      };
    }
    return null;
  });

  // Tab counts: Subjects from the context status, Context from the authored
  // list (both re-read on `user_context_changed`). Journal carries no number
  // by ruling (B7).
  const TABS: { id: InsightsTab; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "journal", label: "Journal" },
    { id: "subjects", label: "Subjects" },
    { id: "context", label: "Context" },
  ];
  const counts = $derived<Partial<Record<InsightsTab, number>>>({
    subjects: ctxStatus?.subjectCount,
    context: authoredCount,
  });

  $effect(() => {
    void untrack(() => loadEngineStatus());
    const unlistens: UnlistenFn[] = [];
    let disposed = false;
    const keep = (fn: UnlistenFn) => (disposed ? fn() : unlistens.push(fn));
    // A context change, a Settings save (engine on/off, default model) and a
    // ChatGPT sign-in all change which notice shows.
    for (const name of ["user_context_changed", "recording_settings_changed", "chatgpt_login_update"]) {
      void listen(name, () => void loadEngineStatus()).then(keep);
    }
    return () => {
      disposed = true;
      for (const fn of unlistens) fn();
    };
  });
</script>

<div class="in-main" bind:this={main}>
  <div class="in-bar">
    <div class="in-bar__in">
      <nav class="in-tabs" aria-label="Insights sections">
        {#each TABS as tab (tab.id)}
          <button
            type="button"
            aria-current={view === tab.id ? "true" : "false"}
            onclick={() => openTab(tab.id)}
          >
            {tab.label}
            {#if counts[tab.id] !== undefined}<span class="num">{counts[tab.id]}</span>{/if}
          </button>
        {/each}
      </nav>
      <span class="mx-spacer"></span>
      <!-- The active view moves its own controls in here (`use:inBar`). -->
      <div class="in-bar__ctl"></div>
    </div>
  </div>

  <section class="in-view" aria-label={TABS.find((t) => t.id === view)?.label}>
    {#if notice && !(engine.kind === "pitch" && view === "journal")}
      {@const Icon = notice.icon}
      <div class="in-notices">
        <div class="mx-notice" data-tone={notice.tone || undefined} role="status">
          <span class="mx-notice__icon"><Icon width="15" height="15" aria-hidden="true" /></span>
          <b class="mx-notice__title">{notice.title}</b>
          <p class="mx-notice__text">{typeof notice.text === "string" ? notice.text : notice.text[view]}</p>
          <div class="mx-notice__acts">
            <button
              type="button"
              class="mx-btn mx-btn--sm"
              class:mx-btn--ghost={notice.ghost}
              onclick={notice.onAction}
            >
              {#if notice.icon === IconCloudOff}<IconRetry width="13" height="13" aria-hidden="true" />{/if}
              {notice.action}
            </button>
          </div>
        </div>
      </div>
    {/if}
    {#if view === "overview"}
      <Overview
        onOpenSubject={openSubject}
        onOpenTab={openTab}
      />
    {:else if view === "journal"}
      <DayTimeline />
    {:else if view === "subjects"}
      {#if selectedSubject}
        <SubjectDetail
          subject={selectedSubject}
          onBack={() => {
            selectedSubject = null;
            if (main) main.scrollTop = 0;
          }}
          askDisabled={engine.kind === "pitch" || engine.kind === "off"}
        />
      {/if}
      <!-- The index stays mounted under a subject's page so its filter and
           Conviction/Movement controls keep their place on the bar (mockup). -->
      <div style:display={selectedSubject ? "none" : "contents"}>
        <Subjects onOpenSubject={openSubject} onViewDismissed={viewDismissed} onFilter={() => (selectedSubject = null)} />
      </div>
    {:else}
      <Context
        openDismissed={contextOpenDismissed}
        onOpenSubject={openSubject}
        providerKind={aiStatus?.defaultProviderKind ?? null}
        providerName={aiStatus?.defaultModel ? providerLabel(aiStatus.defaultModel.provider) : null}
      />
    {/if}
  </section>
</div>

<style>
  /* insights.html `.in-main` / `.in-bar` / `.in-tabs` / `.in-view`. */
  .in-main {
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
  }
  .in-bar {
    position: sticky;
    top: 0;
    z-index: 20;
    border-bottom: 1px solid var(--mx-hairline);
  }
  /* Blur on a pseudo layer so popovers inside the bar can still blur the page. */
  .in-bar::before {
    content: "";
    position: absolute;
    inset: 0;
    z-index: -1;
    background: color-mix(in srgb, var(--app-bg) 84%, transparent);
    -webkit-backdrop-filter: blur(14px) saturate(1.2);
    backdrop-filter: blur(14px) saturate(1.2);
  }
  .in-bar__in {
    max-width: 1280px;
    margin: 0 auto;
    padding: 0 var(--s-7);
    height: 56px;
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  .in-tabs {
    display: flex;
    align-self: stretch;
    gap: var(--s-5);
  }
  .in-tabs button {
    position: relative;
    display: flex;
    align-items: center;
    gap: 7px;
    padding: 0;
    border: 0;
    background: none;
    cursor: pointer;
    font: 500 var(--text-lg) / 1 var(--font-sans);
    letter-spacing: -0.012em;
    color: var(--app-text-subtle);
    transition: color var(--t-fast) var(--ease-quart);
  }
  .in-tabs button:hover {
    color: var(--app-text);
  }
  .in-tabs button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .in-tabs button[aria-current="true"] {
    color: var(--app-text-strong);
  }
  /* The ink sits on the bar's hairline. */
  .in-tabs button[aria-current="true"]::after {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: -1px;
    height: 2px;
    border-radius: 2px;
    background: var(--app-text-strong);
  }
  .in-tabs .num {
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--app-text-faint);
  }
  .in-tabs button[aria-current="true"] .num {
    color: var(--app-text-subtle);
  }
  .in-bar__ctl {
    display: flex;
    align-items: center;
    gap: var(--s-3);
  }
  /* A view's moved-in controls drop the spacing and wrapping they had in-flow. */
  .in-bar__ctl > :global(*) {
    margin: 0;
    flex-wrap: nowrap;
  }
  .in-view {
    padding: var(--s-6) var(--s-7) var(--s-8);
    max-width: 1280px;
    margin: 0 auto;
  }
  .in-notices {
    display: grid;
    gap: var(--s-2);
    margin: 0 0 var(--s-5);
  }
</style>
