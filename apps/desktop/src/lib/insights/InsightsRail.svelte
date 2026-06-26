<script lang="ts">
  // InsightsRail — the persistent left rail of the Insights surface (Insights-
  // rail refactor, Slices 2/3). It replaces the old horizontal `.subnav` and is
  // always present across every sub-surface (Overview / Subjects / Context /
  // Chat). Top→bottom it carries: the sub-surface nav (overview / subjects /
  // context), a "new chat" action, the chat search + time-grouped history
  // (<RailHistory/>), and the engine/model footer (<RailFooter/>). The active
  // sub-surface renders in the column to the RIGHT of this rail (the shell owns
  // that). Chat is reached via "new chat" or a history row — it is NOT a primary
  // nav item, but the "New chat" row doubles as the Chat group anchor and carries
  // the active treatment when `view === "chat"`, so the rail always shows a stable
  // "you are here" landmark.
  //
  // The aesthetic is the approved "minimal / quiet" sidebar: hairline dividers +
  // whitespace + a single green accent. Active state combines accent label +
  // a faint accent tint + an inset accent bar (so "you are here" survives the
  // squint test). Title-case nav/action labels to match the app's sidebar
  // typography (date-group headers stay uppercase eyebrows). ~240px wide by
  // default (drag-resizable), token-driven.
  import RailHistory from "$lib/insights/RailHistory.svelte";
  import RailFooter from "$lib/insights/RailFooter.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";

  type InsightsTab = "overview" | "subjects" | "context" | "chat";

  interface Props {
    view: InsightsTab;
    onOpenTab: (tab: InsightsTab) => void;
    engineOn: boolean;
    modelLabel: string;
    statusLoaded: boolean;
    onEnable: () => void;
    // Slice 6 — rail collapse. When `collapsed` the rail renders nothing (the
    // shell shows a floating expand button instead). The in-rail chevron calls
    // `onToggleCollapse` to hide it; the shell owns the persisted state.
    collapsed: boolean;
    onToggleCollapse: () => void;
    // Drag-resizable width (px). The shell owns the persisted value + clamping;
    // the rail just renders to it. The neighbouring <RailResizer/> (in the shell)
    // is the divider/grab handle.
    width: number;
  }

  let {
    view,
    onOpenTab,
    engineOn,
    modelLabel,
    statusLoaded,
    onEnable,
    collapsed,
    onToggleCollapse,
    width,
  }: Props = $props();

  // The nav is the three persistent sub-surfaces only — Chat is reached via
  // new-chat / a history row, never a nav item.
  const NAV: { id: Exclude<InsightsTab, "chat">; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "subjects", label: "Subjects" },
    { id: "context", label: "Context" },
  ];
</script>

{#if !collapsed}
<aside class="sidebar" aria-label="Insights" style="width: {width}px;">
  <!-- A quiet collapse chevron, floated into the empty top-right gutter so it
       never claims a full header band of dead space above the nav. Hides the
       rail to give the active sub-surface full width; the shell shows a matching
       expand button. -->
  <button
    type="button"
    class="rail-collapse"
    aria-label="Collapse sidebar"
    aria-expanded="true"
    title="Collapse sidebar"
    onclick={onToggleCollapse}
  >
    <span aria-hidden="true">«</span>
  </button>

  <div class="sidebar-scroll">
    <!-- primary nav — plain title-case text rows. Active = accent label + tint
         + inset bar. -->
    <nav class="rail-nav" aria-label="Insights sub-surface">
      {#each NAV as item (item.id)}
        <button
          type="button"
          class="rail-nav-item"
          class:active={view === item.id}
          aria-current={view === item.id ? "page" : undefined}
          onclick={() => onOpenTab(item.id)}
        >
          {item.label}
        </button>
      {/each}
    </nav>

    <!-- new chat — quiet borderless text link. Doubles as the Chat group anchor:
         when `view === "chat"` it carries the active treatment so the rail always
         shows a stable "you are here" landmark even with no history row matched. -->
    <button
      type="button"
      class="rail-newchat"
      class:active={view === "chat"}
      aria-current={view === "chat" ? "page" : undefined}
      onclick={() => conversationStore.requestNewChat()}
    >
      <span class="plus" aria-hidden="true">＋</span> New chat
    </button>

    <!-- chat search + time-grouped history (owns its own rename/delete). -->
    <RailHistory />
  </div>

  <!-- pinned engine/model status footer. -->
  <RailFooter {engineOn} {modelLabel} {statusLoaded} {onEnable} />
</aside>
{/if}

<style>
  /* Persistent left navigation rail — the "minimal / quiet" sidebar. No card
     fills, no item borders, no pills; separation comes from 1px hairline
     dividers + whitespace, with a single green accent for the active state
     (accent label + faint accent tint + inset accent bar). Even 16px L/R gutters,
     title-case nav/action labels (matching the app's sidebar typography).
     Mirrors the approved mockup's `.sidebar`. */
  .sidebar {
    position: relative;
    /* Width is driven by the shell's persisted `railWidth` (inline `width`); the
       neighbouring <RailResizer/> renders the divider + drag handle, so the rail
       no longer carries its own border-right. */
    flex: 0 0 auto;
    width: 240px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--app-surface-subtle);
  }
  .sidebar-scroll {
    flex: 1 1 auto;
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 16px 16px 0;
  }

  /* Collapse chevron — floated into the empty top-right gutter (the nav labels
     are short + left-aligned, so the right half of the first row is dead space).
     Floating it keeps the nav starting at the same top padding as the mockup
     instead of pushing everything down behind a header band. */
  .rail-collapse {
    position: absolute;
    top: 12px;
    right: 10px;
    z-index: 2;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text-subtle);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    transition:
      color 0.12s ease,
      background 0.12s ease;
  }
  .rail-collapse:hover {
    color: var(--app-accent);
    background: var(--app-surface-hover);
  }
  .rail-collapse:focus-visible {
    outline: none;
    color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  /* primary nav — plain title-case text rows, generous spacing. */
  .rail-nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rail-nav-item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    font: inherit;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    transition:
      color 0.12s ease,
      background 0.12s ease;
    background: transparent;
    border: 0;
    border-radius: 6px;
    padding: 0 8px;
    text-align: left;
    cursor: pointer;
  }
  .rail-nav-item:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }
  /* Visible keyboard focus — quiet accent text, no box/pill (keeps the minimal
     aesthetic; the focus ring is an underline-style accent rather than a border). */
  .rail-nav-item:focus-visible {
    outline: none;
    color: var(--app-accent);
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
    text-underline-offset: 3px;
  }
  /* active = accent label + tint fill + inset accent bar (combine >=2 signals so
     "you are here" survives the squint test, while hover stays a neutral tint). */
  .rail-nav-item.active {
    color: var(--app-accent);
    font-weight: 600;
    background: var(--app-accent-bg);
  }
  .rail-nav-item.active:hover {
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .rail-nav-item.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 50%;
    transform: translateY(-50%);
    width: 3px;
    height: 16px;
    border-radius: 0 2px 2px 0;
    background: var(--app-accent);
  }

  /* new chat — quiet borderless text link. */
  .rail-newchat {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    margin-top: 18px;
    background: transparent;
    border: 0;
    padding: 0;
    font: inherit;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .rail-newchat .plus {
    color: var(--app-text-subtle);
    transition: color 0.12s ease;
  }
  .rail-newchat:hover,
  .rail-newchat:hover .plus {
    color: var(--app-accent);
  }
  /* Chat group anchor — when the user is in Chat this row stays accent + bold so
     the rail always shows a stable "you are here" landmark. */
  .rail-newchat.active,
  .rail-newchat.active .plus {
    color: var(--app-accent);
  }
  .rail-newchat.active {
    font-weight: 600;
  }
  .rail-newchat:focus-visible {
    outline: none;
    color: var(--app-accent);
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
    text-underline-offset: 3px;
  }
  .rail-newchat:focus-visible .plus {
    color: var(--app-accent);
  }
</style>
