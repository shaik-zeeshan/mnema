<script lang="ts">
  // InsightsRail — the persistent left rail of the Insights surface (Insights-
  // rail refactor, Slices 2/3). It replaces the old horizontal `.subnav` and is
  // always present across every sub-surface (Overview / Subjects / Context /
  // Chat). Top→bottom it carries: the sub-surface nav (overview / subjects /
  // context), a "new chat" action, the chat search + time-grouped history
  // (<RailHistory/>), and the engine/model footer (<RailFooter/>). The active
  // sub-surface renders in the column to the RIGHT of this rail (the shell owns
  // that). Chat is reached via "new chat" or a history row — it is NOT a nav
  // item, so when `view === "chat"` no nav item is active.
  //
  // The aesthetic is the approved "minimal / quiet" sidebar: hairline dividers +
  // whitespace + a single green accent. Active state is accent TEXT only (no
  // dot, pill, or box). Lowercase labels, 200px wide, token-driven.
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
  }: Props = $props();

  // The nav is the three persistent sub-surfaces only — Chat is reached via
  // new-chat / a history row, never a nav item.
  const NAV: { id: Exclude<InsightsTab, "chat">; label: string }[] = [
    { id: "overview", label: "overview" },
    { id: "subjects", label: "subjects" },
    { id: "context", label: "context" },
  ];
</script>

{#if !collapsed}
<aside class="sidebar" aria-label="Insights">
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
    <!-- primary nav — plain lowercase text rows. Active = accent text only. -->
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

    <!-- new chat — quiet borderless text link. -->
    <button
      type="button"
      class="rail-newchat"
      onclick={() => conversationStore.requestNewChat()}
    >
      <span class="plus" aria-hidden="true">＋</span> new chat
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
     (accent TEXT only — no leading dot/marker). Even 16px L/R gutters,
     lowercase labels. Mirrors the approved mockup's `.sidebar`. */
  .sidebar {
    position: relative;
    flex: 0 0 200px;
    width: 200px;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--app-surface-subtle);
    border-right: 1px solid var(--app-border);
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
    width: 20px;
    height: 20px;
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
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  /* primary nav — plain lowercase text rows, generous spacing. */
  .rail-nav {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .rail-nav-item {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    font: inherit;
    font-size: 12px;
    text-transform: lowercase;
    color: var(--app-text-muted);
    transition: color 0.12s ease;
    background: transparent;
    border: 0;
    padding: 0;
    text-align: left;
    cursor: pointer;
  }
  .rail-nav-item:hover {
    color: var(--app-text-strong);
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
  /* active = accent text only (no leading marker). */
  .rail-nav-item.active {
    color: var(--app-accent-strong);
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
    font-size: 12px;
    text-transform: lowercase;
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
