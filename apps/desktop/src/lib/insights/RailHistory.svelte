<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // RailHistory — the chat search field + time-grouped conversation history that
  // sits in the persistent Insights rail (Insights-rail refactor, Slices 2/3).
  // It renders the shared `conversationStore`: a debounced search over the list,
  // newest-first rows grouped under quiet date headers (Today / Yesterday / This
  // week / earlier months), with per-row inline rename + delete revealed on
  // hover / focus-within. Restyled to the rail's "minimal / quiet" aesthetic
  // (hairline dividers, whitespace, a single green accent for the active row).
  //
  // A row click routes through the store's selection BUS (`requestOpen`); the
  // owning shell switches to the Chat sub-surface when the bus fires. The rename
  // input wiring (a focus/select Svelte action + an Enter/Escape keydown router)
  // is ported from Chat's own rail — Tauri's WKWebView doesn't hand focus around
  // on click, so the input focuses/selects itself once mounted; keydown is
  // attached on the input for the same reason.
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import { slide } from "svelte/transition";
  import { cubicOut } from "svelte/easing";
  import { listTriggers, type ConditionType } from "$lib/triggers/api";
  import { CONDITION_ICON } from "$lib/triggers/condition-icons";
  import {
    conversationStore,
    relativeTime,
    type OriginFilter,
  } from "$lib/insights/conversationStore.svelte";

  // triggerId → condition type, for the origin badge's condition icon.
  // Resolved client-side once per mount (best-effort; a deleted trigger just
  // misses the map and the badge renders without an icon).
  let conditionByTriggerId = $state<Record<string, ConditionType>>({});
  void listTriggers()
    .then((triggers) => {
      conditionByTriggerId = Object.fromEntries(
        triggers.map((t) => [t.id, t.condition.type]),
      );
    })
    .catch(() => {});

  // The rail's origin filter (issue #179) — All / Chats / Triggers, per the
  // final Triggers design (docs/triggers/mockups/final/DESIGN.md). Applied
  // client-side in the store so it composes with backend text search.
  const ORIGIN_FILTERS: { id: OriginFilter; label: string }[] = [
    { id: "all", label: "all" },
    { id: "chats", label: "chats" },
    { id: "triggers", label: "triggers" },
  ];

  const emptyMessage = $derived.by((): string => {
    if (conversationStore.searchQuery.trim().length > 0)
      return "No conversations match.";
    if (conversationStore.conversations.length > 0)
      return conversationStore.originFilter === "triggers"
        ? "No trigger runs yet."
        : "No chats yet.";
    return "No conversations yet.";
  });

  function autofocusSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  function onRenameKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      void conversationStore.commitRename();
    } else if (event.key === "Escape") {
      event.stopPropagation();
      conversationStore.cancelRename();
    }
  }
</script>

<!-- origin filter — rail-sized segmented control (All / Chats / Triggers),
     per the final Triggers design. Narrows the list to trigger runs (or plain
     chats) and back; text search keeps working inside a filtered view. -->
<div class="origin-filter" role="group" aria-label="Filter conversations by origin">
  {#each ORIGIN_FILTERS as f (f.id)}
    <button
      type="button"
      class:active={conversationStore.originFilter === f.id}
      aria-pressed={conversationStore.originFilter === f.id}
      onclick={() => (conversationStore.originFilter = f.id)}
    >
      {f.label}
    </button>
  {/each}
</div>

<!-- search — borderless; a clear magnifier glyph carries the "this is search"
     signal (the app's own search SVG). Focus brightens the glyph to the accent
     (no box/line). -->
<div class="rail-search">
  <svg
    class="icon"
    width="13"
    height="13"
    viewBox="0 0 14 14"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    aria-hidden="true"
  >
    <circle cx="6" cy="6" r="4.5" />
    <path d="M9.5 9.5 13 13" />
  </svg>
  <input
    type="search"
    placeholder="Search chats…"
    aria-label="Search chats"
    autocomplete="off"
    spellcheck="false"
    bind:value={conversationStore.searchQuery}
    oninput={() => conversationStore.onSearchInput()}
  />
</div>

<!-- chat history — ultra-compact single-line rows, no chrome. -->
<div class="rail-history" role="list" aria-label="Conversation history">
  {#if !conversationStore.historyLoaded}
    <div class="rail-history-skeleton">
      {#each Array(6) as _, i (i)}
        <div class="sk-row">
          <Skeleton width="68%" height="9px" radius="4px" muted />
        </div>
      {/each}
    </div>
  {:else if conversationStore.filteredConversations.length === 0}
    <p class="rail-empty">{emptyMessage}</p>
  {:else}
    {#each conversationStore.historyGroups as group (group.label)}
      <div class="rail-group" role="presentation">{group.label}</div>
      {#each group.items as c (c.conversationId)}
        <!-- Deleting a row just makes it vanish; a short local slide+fade makes
             the removal (and post-rename re-sort) perceptible. `|local` keeps it
             from firing on the initial list mount. -->
        <div
          class="rail-chat-row"
          class:active={c.conversationId ===
            conversationStore.activeConversationId}
          role="listitem"
          transition:slide|local={{ duration: 150, easing: cubicOut }}
        >
          {#if conversationStore.renamingId === c.conversationId}
            <!-- Inline rename: Enter commits, Escape cancels, blur
                 commits-if-changed (else cancels). Focus/select is programmatic
                 (WKWebView focus quirk). -->
            <input
              type="text"
              class="rail-rename-input"
              aria-label="Rename conversation"
              spellcheck="false"
              autocomplete="off"
              bind:value={conversationStore.renameDraft}
              use:autofocusSelect
              onkeydown={onRenameKeydown}
              onblur={() => void conversationStore.commitRename()}
            />
          {:else}
            <button
              type="button"
              class="rail-chat"
              onclick={() => conversationStore.requestOpen(c.conversationId)}
              aria-current={c.conversationId ===
              conversationStore.activeConversationId
                ? "true"
                : undefined}
            >
              <span class="row1">
                <span class="t" use:tip={c.title || c.preview}>
                  {c.title || c.preview || "Untitled chat"}
                </span>
                <span class="when">{relativeTime(c.updatedAtMs)}</span>
              </span>
              {#if c.origin === "trigger"}
                <!-- trigger-origin badge: accent-bordered chip with the firing
                     trigger's display name (final Triggers design). -->
                <span
                  class="origin-badge"
                  use:tip={`Run by trigger: ${c.triggerName || "unknown"}`}
                >
                  {#if c.triggerId && conditionByTriggerId[c.triggerId]}
                    {@const CondIcon = CONDITION_ICON[conditionByTriggerId[c.triggerId]]}
                    <span class="badge-glyph" aria-hidden="true"><CondIcon /></span>
                  {/if}
                  {c.triggerName || "trigger run"}
                </span>
              {/if}
            </button>
            <!-- Quiet row actions: hidden until the row is hovered or holds
                 keyboard focus (`:focus-within`) — pure hover would lock
                 keyboard users out. -->
            <div class="rail-actions">
              <button
                type="button"
                class="rail-action"
                aria-label="Rename conversation"
                use:tip={"Rename conversation"}
                onclick={(e) => {
                  e.stopPropagation();
                  conversationStore.startRename(c);
                }}
              >
                ✎
              </button>
              <button
                type="button"
                class="rail-action rail-action--delete"
                aria-label="Delete conversation"
                use:tip={"Delete conversation"}
                onclick={(e) => {
                  e.stopPropagation();
                  void conversationStore.deleteConversation(c);
                }}
              >
                ✕
              </button>
            </div>
          {/if}
        </div>
      {/each}
    {/each}
  {/if}
</div>

<style>
  /* search — a quiet borderless row, consistent with the nav / new-chat rows.
     No box, fill, or underline; a clear magnifier glyph (the app's own search
     SVG) does the "this is search" signalling that the old thin ⌕ couldn't. A
     focus bottom-hairline read as a stray, lopsided green line, so it's gone —
     on focus the only cue is the glyph brightening to the accent (plus the
     caret), matching the nav's box-free focus idiom. Token-driven,
     sentence-case placeholder. */
  /* search — shares the nav / new-chat row geometry exactly (28px tall, 0 8px
     padding, 8px gap, 16px leading-glyph box) so the magnifier lands on the nav
     icon guide and the input text on the nav label guide. Sits tight under New
     chat as one "actions" cluster, then the history list. */
  .rail-search {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    margin-top: 8px;
    padding: 0 8px;
    border-radius: 6px;
    transition: background 0.12s ease;
  }
  /* Focus the search → glyph brightens to the accent + a quiet tint fill, matching
     the nav/new-chat hover idiom (no box/line). */
  .rail-search:focus-within {
    background: var(--app-surface-hover);
  }
  .rail-search:focus-within .icon {
    color: var(--app-accent);
  }
  .rail-search .icon {
    flex: 0 0 16px;
    width: 16px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--app-text-subtle);
    transition: color 0.12s ease;
  }
  .rail-search input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    margin: 0;
    padding: 0;
    /* Strip WebKit's native search-field chrome. Its intrinsic box grows once
       the field holds a value, which pushed the whole row — and everything below
       it — down (the reported layout shift). With appearance:none the row height
       is purely our fixed 30px, empty or filled. */
    appearance: none;
    -webkit-appearance: none;
    background: transparent;
    border: 0;
    outline: 0;
    color: var(--app-text);
    font-family: inherit;
    font-size: 12.5px;
    line-height: 1;
  }
  .rail-search input::placeholder {
    color: var(--app-text-subtle);
  }
  /* Hide the native search clear affordance for a consistent terminal look. */
  .rail-search input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  /* origin filter — the mockup's rail-sized segmented control (All / Chats /
     Triggers). Same quiet chrome as the rest of the rail: 1px border, tint-on-
     active, lowercase labels. */
  .origin-filter {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    margin-top: 14px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .origin-filter button {
    flex: 1 1 0;
    font: inherit;
    font-size: 10.5px;
    line-height: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 20px;
    padding: 0 4px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    text-transform: lowercase;
    transition:
      color 0.12s ease,
      background 0.12s ease;
  }
  .origin-filter button:hover {
    color: var(--app-text-strong);
  }
  .origin-filter button:focus-visible {
    outline: none;
    color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }
  .origin-filter button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  /* trigger-origin badge — accent-bordered chip with the firing trigger's
     display name, sitting under the row title (final Triggers design). */
  .origin-badge {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    /* Vertical inset comes from the row's own padding now. */
    margin-bottom: 1px;
    font-size: 9px;
    letter-spacing: 0.03em;
    line-height: 14px;
    padding: 0 5px;
    border-radius: 4px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .badge-glyph {
    display: inline-flex;
    flex: 0 0 auto;
  }
  .badge-glyph :global(svg) {
    width: 9px;
    height: 9px;
  }

  /* chat history — ultra-compact single-line rows, no chrome. */
  .rail-history {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding-bottom: 16px;
    /* Scrollable, but the scrollbar chrome is hidden (Firefox). */
    scrollbar-width: none;
  }
  /* Hide the WebKit scrollbar — the list still scrolls, just without the rail. */
  .rail-history::-webkit-scrollbar {
    width: 0;
    height: 0;
    display: none;
  }

  .rail-history-skeleton {
    display: flex;
    flex-direction: column;
    gap: 14px;
    margin-top: 14px;
  }
  .sk-row {
    display: flex;
    padding: 0 8px;
  }
  .rail-empty {
    font-size: 11px;
    color: var(--app-text-subtle);
    margin-top: 16px;
    padding: 0 8px;
    line-height: 1.5;
  }

  /* group label — tiny, faint, uppercase eyebrow (matching the app's section
     markers); hairline above + top spacing. */
  .rail-group {
    font-size: 9px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    margin-top: 14px;
    /* 8px horizontal inset puts the eyebrow on the same 24px content guide as
       the row titles / nav labels (it used to sit flush at the gutter edge,
       reading as "touching the border"); the hairline still spans full width. */
    padding: 11px 8px 8px;
    border-top: 1px solid var(--app-border);
  }
  /* first group: sits clean below the search's bottom hairline — no double
     line. */
  .rail-group:first-child {
    border-top: none;
    margin-top: 12px;
    padding-top: 0;
  }

  /* A row holds the chat link + its quiet hover actions on one baseline line.
     `position: relative` anchors the absolutely-placed `.rail-actions` (see
     below) so the hidden actions never reserve width. */
  .rail-chat-row {
    position: relative;
    display: flex;
    align-items: center;
    /* Same 8px horizontal inset as the nav rows (keeps title/badge on the 24px
       content guide and stops the timestamp touching the tint's right edge),
       plus a small vertical inset so the active tint never hugs the content.
       The left 8px also reserves room for the active row's 3px inset bar so
       toggling active never shifts the title horizontally. */
    padding: 3px 8px;
    border-radius: 5px;
    /* The inner `.rail-chat` is a fixed 24px tall, so the row height matches it
       without an explicit `min-height` — and dropping the min-height lets the
       removal slide collapse smoothly to 0 instead of snapping at 24px. */
  }
  .rail-chat {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: stretch;
    min-height: 24px;
    background: transparent;
    border: 0;
    padding: 0;
    width: 100%;
    cursor: pointer;
    text-align: left;
    font: inherit;
  }
  /* First line — the old single-line row geometry (title + timestamp); a
     trigger row grows a second badge line below it. */
  .rail-chat .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 24px;
    min-width: 0;
  }
  .rail-chat .t {
    flex: 1 1 auto;
    min-width: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    transition: color 0.12s ease;
  }
  .rail-chat-row:hover .t {
    color: var(--app-text-strong);
  }
  /* Keyboard focus on the row link — quiet accent title, no box. */
  .rail-chat:focus-visible {
    outline: none;
  }
  .rail-chat:focus-visible .t {
    color: var(--app-accent);
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
    text-underline-offset: 3px;
  }
  .rail-chat .when {
    font-size: 9.5px;
    color: var(--app-text-faint);
    flex: 0 0 auto;
    transition: opacity 0.12s ease;
  }
  /* The timestamp yields to the hover actions so the two never overlap. */
  .rail-chat-row:hover .when,
  .rail-chat-row:focus-within .when {
    opacity: 0;
  }
  /* active row — accent title PLUS a tinted background and a 3px inset accent
     bar, so the selection never relies on text colour alone (matches the
     primary nav's multi-signal active treatment). */
  .rail-chat-row.active {
    background: var(--app-accent-bg);
  }
  .rail-chat-row.active::before {
    content: "";
    position: absolute;
    left: 0;
    top: 3px;
    bottom: 3px;
    width: 3px;
    border-radius: 0 2px 2px 0;
    background: var(--app-accent-strong);
  }
  .rail-chat-row.active .t {
    color: var(--app-accent-strong);
  }

  /* Row actions (rename + delete): hidden until the row is hovered or holds
     keyboard focus. Absolutely anchored to the row's right edge so they NEVER
     reserve width when hidden — previously they sat in flow at `opacity: 0`,
     stealing ~44px from every title (forcing truncation) and leaving a dead
     right gutter. A short gradient masks the title/timestamp they overlay. */
  .rail-actions {
    position: absolute;
    top: 0;
    /* Matches the row's own right inset so the ✕ never touches the tint edge. */
    right: 4px;
    bottom: 0;
    display: flex;
    align-items: center;
    gap: 2px;
    padding-left: 16px;
    background: linear-gradient(
      to right,
      transparent,
      var(--app-surface-subtle) 45%
    );
    opacity: 0;
    pointer-events: none;
    transition: opacity 0.12s ease;
  }
  .rail-chat-row:hover .rail-actions,
  .rail-chat-row:focus-within .rail-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .rail-action {
    width: 18px;
    height: 18px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: 0;
    background: transparent;
    color: var(--app-text-subtle);
    font-size: 11px;
    line-height: 1;
    cursor: pointer;
    transition: color 0.12s ease;
  }
  .rail-action:hover {
    color: var(--app-text-strong);
  }
  .rail-action--delete:hover {
    color: var(--app-danger);
  }
  /* Keyboard focus on a row action — small accent ring, stays visible (the row
     reveals its actions on :focus-within so a focused action is always shown). */
  .rail-action:focus-visible {
    outline: none;
    color: var(--app-text-strong);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
    border-radius: 5px;
  }
  .rail-action--delete:focus-visible {
    color: var(--app-danger);
    box-shadow: 0 0 0 2px var(--app-danger-bg);
  }

  /* Inline rename input: replaces the row content while editing. Quiet, sits
     flush within the rail's gutter. */
  .rail-rename-input {
    flex: 1 1 auto;
    width: 100%;
    min-width: 0;
    height: 22px;
    margin: 1px 0;
    padding: 2px 6px;
    font: inherit;
    font-size: 11.5px;
    border: 1px solid var(--app-accent-border);
    border-radius: 5px;
    background: var(--app-surface);
    color: var(--app-text);
    outline: none;
  }
  .rail-rename-input:focus {
    border-color: var(--app-accent);
  }
</style>
