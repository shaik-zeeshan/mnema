<script lang="ts">
  // Settings left rail — Codex-style (Slice 3).
  //
  // Renders the 5 navigation groups from `groups.ts` (the source of truth) as
  // category headers (`.nav-cat`) each followed by their sub-sections as bare
  // `.nav-item`s. The nav is a NAVIGATION landmark (a `role="list"` of grouped
  // section buttons), NOT a tablist: settings is a single scrolling panel with
  // scroll-spy, so the items behave like in-page links — the active one carries
  // `aria-current="page"`. A roving tabindex + arrow/Home/End keyboard nav keeps
  // the whole list reachable with one Tab stop; the active sub-section is driven
  // by the shell via `activeSection`, and clicks / keyboard activation call
  // `onNavigate(section)`.
  //
  // The rail is always expanded (the collapse-to-icons feature was dropped).
  // A fixed top zone holds the "← Back to app" link + a search field; a pinned
  // footer shows the live auto-save status, styled per the mockup.

  import { goto } from "$app/navigation";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSearch from "~icons/lucide/search";
  import { SECTION_ICONS } from "./section-icons";
  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
    type SettingsSection,
    type SettingsSectionId,
  } from "./groups";
  import { filterGroups, flattenSections } from "./rail-filter";
  import { getSettingsController } from "./state/controller.svelte";
  import { getLastMainSurface } from "$lib/surface-windows";

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

  // The single roving-tabindex target: the one visible tab that gets `tabindex=0`
  // (so the tablist is reachable by Tab). It's `activeSection` while that section
  // is still visible; once a search query filters the active section OUT of
  // `flatItems`, fall back to the first visible item so the list never becomes
  // entirely `tabindex=-1` (which would make it unreachable by keyboard).
  const rovingTarget = $derived<SettingsSectionId | undefined>(
    flatItems.some((s) => s.id === activeSection)
      ? activeSection
      : flatItems[0]?.id,
  );

  // Clearing the query on blur must NOT eat a click on a nav item: a click that
  // moves focus out of the input fires `blur` BEFORE the item's `click`. If we
  // cleared synchronously here, the item would unmount before its click landed.
  // Defer the clear to a macrotask so the pending click + navigation fire first.
  function clearSearch() {
    searchQuery = "";
  }
  function onSearchBlur(event: FocusEvent) {
    // Only clear when focus actually leaves the rail. A keyboard user who Tabs
    // from the search field into the filtered results stays inside the rail —
    // clearing then would re-render the full list and drop the survivor they
    // were reaching for. `relatedTarget` is the element gaining focus (null for
    // a plain click into empty space, which the deferred clear still handles).
    const next = event.relatedTarget;
    if (
      next instanceof HTMLElement &&
      next.closest("#settings-sidebar")
    ) {
      return;
    }
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
  // so leaving it is a plain in-window navigation back to the last main surface
  // the user was on (Timeline or Insights), falling back to `/`.
  function backToApp() {
    void goto(getLastMainSurface());
  }

  // ─── Flattened nav keyboard nav (roving tabindex) ─────────────────────────
  // The rail is a NAVIGATION landmark (a single scrolling panel with scroll-spy
  // highlighting), not a tablist — but it keeps the roving-tabindex + arrow/
  // Home/End stepping so the whole list is reachable with one Tab stop.
  function handleNavKeydown(event: KeyboardEvent) {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>(".nav-item")
      : null;
    const focusedSection = focusedTab?.dataset.section as SettingsSectionId | undefined;
    const focusedIndex = focusedSection
      ? flatItems.findIndex((s) => s.id === focusedSection)
      : -1;
    // Anchor nav on the focused tab; else the active section if it's still
    // visible; else the first survivor (a query may have filtered the active
    // section out, but the visible survivors must still be steppable). Bail only
    // when there's genuinely nothing to navigate.
    const activeIndex = flatItems.findIndex((s) => s.id === activeSection);
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : activeIndex >= 0
        ? activeIndex
        : flatItems.length > 0
          ? 0
          : -1;
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
</script>

<aside id="settings-sidebar" class="settings-sidebar settings-rail">
  <!-- Fixed top zone: back link + search -->
  <div class="rail-top">
    <button class="rail-back" type="button" onclick={backToApp}>
      <IconBack aria-hidden="true" />
      Back to app
    </button>

    <div class="rail-search">
      <IconSearch aria-hidden="true" />
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

  <!-- Scrolling nav: a NAVIGATION landmark whose items are buttons that scroll
       the single settings panel to a section (scroll-spy drives the active one).
       Modeled as a list of groups rather than a tablist — every "tab" would have
       pointed at the one mounted panel id, and most targets are unmounted, which
       a real tablist mustn't do. The active section carries aria-current="page".
       The roving-tabindex keyboard stepping (arrow/Home/End) lives on the
       .nav-item buttons themselves — keeping the keydown on the interactive
       elements, not the non-interactive role="list" container. -->
  <nav class="settings-nav rail-nav" aria-label="Settings sections">
    <div class="rail-nav__list" role="list">
      {#each visibleGroups as group (group.id)}
        <div class="nav-group" role="group" aria-labelledby="settings-cat-{group.id}">
          <p class="nav-cat" id="settings-cat-{group.id}">{group.label}</p>
          {#each group.sections as section (section.id)}
            {@const SectionIcon = SECTION_ICONS[section.id]}
            <button
              class="nav-item"
              class:nav-item--active={activeSection === section.id}
              type="button"
              id="settings-tab-{section.id}"
              data-section={section.id}
              aria-current={activeSection === section.id ? "page" : undefined}
              tabindex={rovingTarget === section.id ? 0 : -1}
              onclick={() => onNavigate(section.id)}
              onkeydown={handleNavKeydown}
            >
              <SectionIcon aria-hidden="true" />
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

  <!-- Autosave failure detail: the bare "save failed" dot above discards the
       message, so surface the actual error here. Each of the three error sources
       that light the footer dot (recording, keyboard, microphone) gets its own
       detail line so none is left as just an unexplained dot. role="alert" so the
       failure is announced, not just shown.

       The recording path has a targeted Retry (re-runs the failed domain save,
       bypassing the backoff window) + a Dismiss that reconciles the control back
       to the last-saved value. The keyboard + microphone domains have no
       equivalent manual-retry surface (their autosave engine re-attempts a dirty
       save on its own), so they show the message text + a Dismiss that clears the
       error — consistent with the recError treatment. -->
  {#if rec.recError}
    <div class="rail-foot-error" role="alert">
      <p class="rail-foot-error__msg">{rec.recError}</p>
      <div class="rail-foot-error__actions">
        {#if c.lastFailedSaveDomain}
          <button class="btn btn--ghost btn--sm" type="button" onclick={() => c.retryFailedSave()}>
            Retry
          </button>
        {/if}
        <button class="btn btn--ghost btn--sm" type="button" onclick={() => c.dismissRecError()}>
          Dismiss
        </button>
      </div>
    </div>
  {/if}

  {#if keyboard.keyboardBindingsError}
    <div class="rail-foot-error" role="alert">
      <p class="rail-foot-error__msg">{keyboard.keyboardBindingsError}</p>
      <div class="rail-foot-error__actions">
        <button
          class="btn btn--ghost btn--sm"
          type="button"
          onclick={() => (keyboard.keyboardBindingsError = null)}
        >
          Dismiss
        </button>
      </div>
    </div>
  {/if}

  {#if audio.micError}
    <div class="rail-foot-error" role="alert">
      <p class="rail-foot-error__msg">{audio.micError}</p>
      <div class="rail-foot-error__actions">
        <button
          class="btn btn--ghost btn--sm"
          type="button"
          onclick={() => (audio.micError = null)}
        >
          Dismiss
        </button>
      </div>
    </div>
  {/if}
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

  /* Autosave failure banner pinned under the footer status line. Keeps the real
     error message visible (it scrolls if long) with the Retry/Dismiss actions
     directly beneath it. Tokens-only, namespaced under the rail. */
  .rail-foot-error {
    margin: 0 8px 8px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    border: 1px solid var(--app-danger-border);
    border-radius: 6px;
    background: var(--app-danger-bg);
  }

  .rail-foot-error__msg {
    margin: 0;
    font-size: 12px;
    line-height: 1.4;
    color: var(--app-danger-text);
    max-height: 6.4em;
    overflow-y: auto;
    word-break: break-word;
  }

  .rail-foot-error__actions {
    display: flex;
    gap: 6px;
  }
</style>
