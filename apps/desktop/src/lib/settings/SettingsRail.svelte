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
  import Icon from "./Icon.svelte";
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
              <Icon name={section.id} />
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
