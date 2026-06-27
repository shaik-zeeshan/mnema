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
  import type { IconComponent } from "$lib/settings/section-icons";
  import IconOverview from "~icons/lucide/layout-dashboard";
  import IconSubjects from "~icons/lucide/lightbulb";
  import IconContext from "~icons/lucide/notebook-text";
  import IconCollapse from "~icons/lucide/chevrons-left";

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
  const NAV: {
    id: Exclude<InsightsTab, "chat">;
    label: string;
    icon: IconComponent;
  }[] = [
    { id: "overview", label: "Overview", icon: IconOverview },
    { id: "subjects", label: "Subjects", icon: IconSubjects },
    { id: "context", label: "Context", icon: IconContext },
  ];

  // "New chat" carries the Chat-group active treatment ONLY when the open thread
  // is an unsaved/new one — i.e. the active conversation id isn't a saved history
  // row. When a SAVED conversation is open that row (in <RailHistory/>) owns the
  // "you are here" highlight, so "New chat" reverts to its quiet state instead of
  // doubly reading active beside the selected row (the section-vs-selection nit).
  const newChatActive = $derived(
    view === "chat" &&
      !conversationStore.conversations.some(
        (c) => c.conversationId === conversationStore.activeConversationId,
      ),
  );
</script>

{#if !collapsed}
<aside class="sidebar" aria-label="Insights" style="width: {width}px;">
  <div class="sidebar-scroll">
    <!-- A quiet collapse chevron in a compact right-aligned header row. It owns
         its own band so it never sits on top of the (full-width) Overview nav
         button. Hides the rail to give the active sub-surface full width; the
         shell shows a matching expand button. -->
    <div class="rail-header">
      <button
        type="button"
        class="rail-collapse"
        aria-label="Collapse sidebar"
        aria-expanded="true"
        title="Collapse sidebar"
        onclick={onToggleCollapse}
      >
        <IconCollapse aria-hidden="true" />
      </button>
    </div>

    <!-- primary nav — title-case text rows with a leading glyph. Active =
         accent label + tint + inset bar. -->
    <nav class="rail-nav" aria-label="Insights sub-surface">
      {#each NAV as item (item.id)}
        {@const Icon = item.icon}
        <button
          type="button"
          class="rail-nav-item"
          class:active={view === item.id}
          aria-current={view === item.id ? "page" : undefined}
          onclick={() => onOpenTab(item.id)}
        >
          <Icon aria-hidden="true" />
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
      class:active={newChatActive}
      aria-current={newChatActive ? "page" : undefined}
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
    padding: 10px 16px 0;
  }

  /* Compact header band — holds only the collapse chevron, right-aligned. Giving
     it its own row keeps the chevron off the (full-width) Overview nav button it
     used to overlap when it was absolutely positioned. */
  .rail-header {
    display: flex;
    justify-content: flex-end;
    align-items: center;
    height: 24px;
    margin-bottom: 4px;
  }

  /* Collapse chevron. */
  .rail-collapse {
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
  .rail-collapse :global(svg) {
    width: 16px;
    height: 16px;
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
    gap: 2px;
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
  /* Leading glyph — inherits the row's color (muted → strong on hover → accent
     when active) via currentColor, and never shrinks below its 16px box. */
  .rail-nav-item :global(svg) {
    flex: none;
    width: 16px;
    height: 16px;
    opacity: 0.85;
  }
  .rail-nav-item.active :global(svg) {
    opacity: 1;
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

  /* new chat — a full row that shares the nav's geometry exactly: same 28px
     height, same 0 8px padding, same 8px gap, and a 16px leading-glyph box. That
     puts the ＋ on the SAME vertical guide as the nav icons and the label on the
     same guide as the nav labels (the previous inline link sat ~15px off, which
     read as the rail's broken alignment). */
  .rail-newchat {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    margin-top: 16px;
    background: transparent;
    border: 0;
    border-radius: 6px;
    padding: 0 8px;
    font: inherit;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    text-align: left;
    cursor: pointer;
    transition:
      color 0.12s ease,
      background 0.12s ease;
  }
  .rail-newchat .plus {
    flex: none;
    width: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 15px;
    line-height: 1;
    color: var(--app-text-subtle);
    transition: color 0.12s ease;
  }
  .rail-newchat:hover {
    color: var(--app-accent);
    background: var(--app-surface-hover);
  }
  .rail-newchat:hover .plus {
    color: var(--app-accent);
  }
  /* Chat group anchor — when the user is in Chat this row carries the same
     multi-signal active treatment as the nav (accent label + tint + inset bar) so
     the rail always shows a stable "you are here" landmark. */
  .rail-newchat.active,
  .rail-newchat.active .plus {
    color: var(--app-accent);
  }
  .rail-newchat.active {
    font-weight: 600;
    background: var(--app-accent-bg);
  }
  .rail-newchat.active::before {
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
