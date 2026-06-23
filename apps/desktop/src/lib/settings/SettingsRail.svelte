<script lang="ts">
  // Settings left rail — Codex-style (Slice 3).
  //
  // Renders the 5 navigation groups from `groups.ts` (the source of truth) as
  // category headers (`.nav-cat`) each followed by their sub-sections as bare
  // `.nav-item`s. The nav is modeled as ONE flattened WAI-ARIA tablist of
  // sub-section tabs with roving tabindex + arrow/Home/End keyboard nav; the
  // active sub-section is driven by the shell via `activeSection`, and clicks /
  // keyboard activation call `onNavigate(section)`.
  //
  // The rail is always expanded (the collapse-to-icons feature was dropped).
  // A fixed top zone holds the "← Back to app" link + a search field; a pinned
  // footer shows the live auto-save status, styled per the mockup.

  import { goto } from "$app/navigation";
  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
    type SettingsSection,
    type SettingsSectionId,
  } from "./groups";
  import { filterGroups, flattenSections } from "./rail-filter";
  import { getSettingsController } from "./state/controller.svelte";

  interface Props {
    /** The active group (the one group panel currently mounted). */
    activeGroup: SettingsGroupId;
    /** The active sub-section — drives the rail's active item. */
    activeSection: SettingsSectionId;
    /** Called on click / keyboard activation of a sub-section item. */
    onNavigate: (section: SettingsSectionId) => void;
  }

  let { activeGroup, activeSection, onNavigate }: Props = $props();

  const c = getSettingsController();
  const rec = c.rec;
  const keyboard = c.keyboard;
  const audio = c.audio;

  // Slice 4: the search field narrows the nav as you type. The (pure) filter
  // helper lives in `rail-filter.ts`; here we only bind state + render.
  let searchQuery = $state("");
  let searchInput = $state<HTMLInputElement | null>(null);

  // The visible (filtered) groups — what the nav actually renders. An empty or
  // whitespace query is a pass-through (all groups). A no-match query yields [].
  const visibleGroups = $derived(filterGroups(SETTINGS_GROUPS, searchQuery));

  // The flattened sub-section order (rail order) — the keyboard roving model.
  // Derived from the VISIBLE groups so Arrow/Home/End only traverse the items
  // currently shown (if the active section is filtered out, nav still works over
  // whatever is visible, and is a no-op on an empty list).
  const flatItems = $derived<SettingsSection[]>(flattenSections(visibleGroups));

  // Clearing the query on blur must NOT eat a click on a nav item: a click that
  // moves focus out of the input fires `blur` BEFORE the item's `click`. If we
  // cleared synchronously here, the item would unmount before its click landed.
  // Defer the clear to a macrotask so the pending click + navigation fire first.
  function clearSearch() {
    searchQuery = "";
  }
  function onSearchBlur() {
    setTimeout(clearSearch, 0);
  }
  function onSearchKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      searchQuery = "";
      searchInput?.blur();
    }
  }

  // "← Back to app" — Settings is the `/settings` route inside the Main window,
  // so leaving it is a plain in-window navigation back to the app root.
  function backToApp() {
    void goto("/");
  }

  // ─── Flattened tablist keyboard nav (roving tabindex) ─────────────────────
  function handleNavKeydown(event: KeyboardEvent) {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[role="tab"]')
      : null;
    const focusedSection = focusedTab?.dataset.section as SettingsSectionId | undefined;
    const focusedIndex = focusedSection
      ? flatItems.findIndex((s) => s.id === focusedSection)
      : -1;
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : flatItems.findIndex((s) => s.id === activeSection);
    if (currentIndex === -1) return;

    let nextIndex: number | null = null;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") {
      nextIndex = (currentIndex + 1) % flatItems.length;
    } else if (event.key === "ArrowUp" || event.key === "ArrowLeft") {
      nextIndex = (currentIndex - 1 + flatItems.length) % flatItems.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = flatItems.length - 1;
    }
    if (nextIndex === null) return;

    event.preventDefault();
    event.stopPropagation();
    const next = flatItems[nextIndex];
    // Moving the roving focus activates the item (focus + navigate).
    onNavigate(next.id);
    document.getElementById(`settings-tab-${next.id}`)?.focus();
  }

  // Group a section belongs to controls which panel id its tab `aria-controls`.
  function groupOf(section: SettingsSectionId): SettingsGroupId {
    for (const g of SETTINGS_GROUPS) {
      if (g.sections.some((s) => s.id === section)) return g.id;
    }
    return activeGroup;
  }
</script>

<!-- Per-section nav glyphs — one per sub-section, drawn on a 24 viewBox with a
     1.7 stroke so the rail reads as one icon family (mockup-2b). -->
{#snippet navIcon(section: SettingsSectionId)}
  <svg viewBox="0 0 24 24" aria-hidden="true">
    {#if section === "appearance"}
      <path d="M12 2a10 10 0 1 0 0 20 2.5 2.5 0 0 0 2-4 2.5 2.5 0 0 1 2-4h2A4 4 0 0 0 22 10 10 10 0 0 0 12 2z" />
      <circle cx="7.5" cy="10.5" r="1" />
      <circle cx="12" cy="7.5" r="1" />
      <circle cx="16.5" cy="10.5" r="1" />
    {:else if section === "startup"}
      <path d="M12 2v8" />
      <path d="M18.4 6.6a9 9 0 1 1-12.8 0" />
    {:else if section === "shortcuts"}
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h0M10 10h0M14 10h0M18 10h0M6 14h0M18 14h0M9 14h6" />
    {:else if section === "capture"}
      <rect x="2" y="3" width="20" height="14" rx="2" />
      <path d="M8 21h8M12 17v4" />
    {:else if section === "video"}
      <rect x="2" y="6" width="20" height="13" rx="2" />
      <path d="M2 9h20M6 6V4M10 6V4M14 6V4M18 6V4" />
    {:else if section === "audio"}
      <path d="M3 12h2l2-6 4 14 3-9 2 4h5" />
    {:else if section === "privacy"}
      <path d="M12 2l8 3v6c0 5-3.5 8-8 11-4.5-3-8-6-8-11V5l8-3z" />
    {:else if section === "intelligence"}
      <path d="M9 2v6M15 2v6M8 8h8v3a4 4 0 0 1-8 0V8zM12 15v3a3 3 0 0 0 3 3h2" />
    {:else if section === "askAi"}
      <path d="M12 3l1.8 4.2L18 9l-4.2 1.8L12 15l-1.8-4.2L6 9l4.2-1.8L12 3z" />
      <path d="M18 14l.9 2.1L21 17l-2.1.9L18 20l-.9-2.1L15 17l2.1-.9L18 14z" />
    {:else if section === "userContext"}
      <circle cx="12" cy="8" r="4" />
      <path d="M4 21a8 8 0 0 1 16 0" />
    {:else if section === "ocr"}
      <path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2" />
      <path d="M8 10h0M12 10v4M16 10h0M9 16h6" />
    {:else if section === "transcription"}
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
    {:else if section === "speakers"}
      <circle cx="9" cy="8" r="3" />
      <path d="M3 20a6 6 0 0 1 12 0" />
      <path d="M16 5.5a3 3 0 0 1 0 5M18 14a6 6 0 0 1 3 5" />
    {:else if section === "semanticSearch"}
      <circle cx="11" cy="11" r="7" />
      <path d="M21 21l-4.3-4.3" />
      <circle cx="11" cy="11" r="1.5" fill="currentColor" stroke="none" />
    {:else if section === "storage"}
      <ellipse cx="12" cy="5" rx="8" ry="3" />
      <path d="M4 5v6c0 1.66 3.58 3 8 3s8-1.34 8-3V5" />
      <path d="M4 11v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
    {:else if section === "access"}
      <circle cx="8" cy="8" r="4" />
      <path d="M11 11l8 8M16 16l2-2M18 18l2-2" />
    {:else if section === "about"}
      <circle cx="12" cy="12" r="9" />
      <path d="M12 16v-4M12 8h.01" />
    {:else if section === "developer"}
      <path d="M8 9l-4 3 4 3M16 9l4 3-4 3M13 6l-2 12" />
    {/if}
  </svg>
{/snippet}

<aside id="settings-sidebar" class="settings-sidebar settings-rail">
  <!-- Fixed top zone: back link + search -->
  <div class="rail-top">
    <button class="rail-back" type="button" onclick={backToApp}>
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M15 18l-6-6 6-6" /></svg>
      Back to app
    </button>

    <div class="rail-search">
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="11" cy="11" r="7" />
        <path d="M21 21l-4.3-4.3" />
      </svg>
      <input
        bind:this={searchInput}
        type="text"
        placeholder="Search settings…"
        aria-label="Search settings"
        bind:value={searchQuery}
        onkeydown={onSearchKeydown}
        onblur={onSearchBlur}
      />
    </div>
  </div>

  <!-- Scrolling nav: flattened tablist of sub-section tabs. -->
  <nav class="settings-nav rail-nav" aria-label="Settings sections">
    <div class="rail-nav__list" role="tablist" aria-orientation="vertical" tabindex="-1" onkeydown={handleNavKeydown}>
      {#each visibleGroups as group (group.id)}
        <div class="nav-group">
          <p class="nav-cat" role="presentation">{group.label}</p>
          {#each group.sections as section (section.id)}
            <button
              class="nav-item"
              class:nav-item--active={activeSection === section.id}
              type="button"
              role="tab"
              id="settings-tab-{section.id}"
              data-section={section.id}
              aria-selected={activeSection === section.id}
              aria-controls="settings-panel-{groupOf(section.id)}"
              tabindex={activeSection === section.id ? 0 : -1}
              onclick={() => onNavigate(section.id)}
            >
              {@render navIcon(section.id)}
              <span>{section.label}</span>
            </button>
          {/each}
        </div>
      {/each}
      {#if visibleGroups.length === 0}
        <p class="rail-empty" role="status">No settings match</p>
      {/if}
    </div>
  </nav>

  <!-- Pinned footer: live auto-save status (relocated from the old rail head). -->
  <div class="rail-foot" aria-live="polite">
    {#if rec.recError || keyboard.keyboardBindingsError || audio.micError}
      <span class="rail-foot__dot rail-foot__dot--error"></span>
      <span class="rail-foot__label">save failed</span>
    {:else if c.recSaveBlocked || audio.micApplyBlocked}
      <span class="rail-foot__dot rail-foot__dot--blocked"></span>
      <span class="rail-foot__label">resolve issues</span>
    {:else if c.savingRecSettings || keyboard.savingKeyboardBindings || audio.savingMicSettings}
      <span class="rail-foot__dot rail-foot__dot--saving"></span>
      <span class="rail-foot__label">saving</span>
    {:else if rec.recSaved || keyboard.keyboardBindingsSaved || audio.micSaved}
      <span class="rail-foot__dot rail-foot__dot--ok"></span>
      <span class="rail-foot__label">saved</span>
    {:else}
      <span class="rail-foot__dot"></span>
      <span class="rail-foot__label">auto-save on</span>
    {/if}
  </div>
</aside>

<style>
  /* Slice 4: quiet empty state when the search query matches no sections.
     Namespaced with the rail's `rail-`/`nav-` family; tokens-only (muted text
     to match `.nav-cat`). Component-scoped so it never touches the shared
     `.settings-shell` cascade in settings-layout.css. */
  .rail-empty {
    margin: 4px 8px;
    padding: 6px 10px;
    font-size: 12px;
    line-height: 1.4;
    color: var(--app-text-subtle);
  }
</style>
