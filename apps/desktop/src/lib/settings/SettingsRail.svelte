<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // Settings left rail — page 11's corrected structure.
  //
  // The app has FIVE groups and twenty-two sections, and twenty-seven rows do
  // not fit a 208px rail at native density. So the rail lists the five GROUPS,
  // each with its indexed row count, and only the ACTIVE group discloses its
  // sections beneath it — the pane's sticky headers already carry the same
  // names. (`docs/redesign/round4/03-layered-glass/README.md`, "11's rail
  // corrects 05's".)
  //
  // The nav is a NAVIGATION landmark (a `role="list"` of group rows and their
  // disclosed section rows), NOT a tablist: settings is a single scrolling
  // panel with scroll-spy, so the items behave like in-page links — the active
  // one carries `aria-current="page"`. A roving tabindex + arrow/Home/End
  // keyboard nav keeps the whole list reachable with one Tab stop; the active
  // sub-section is driven by the shell via `activeSection`, and clicks /
  // keyboard activation call `onNavigate(section)`.
  //
  // The rail is always expanded (the collapse-to-icons feature was dropped).
  // A fixed top zone holds the "← Back to app" link + a search field. The rail
  // carries NO save state: G7 puts the whole autosave story in the top-anchored
  // chip (`ui/SettingsSaveChip.svelte`) — a pinned footer is a bottom strip, and
  // there is no bottom save bar, ever.
  //
  // Direction 03 pins one thing to the rail's bottom instead: the LIVE COST of
  // what you enabled, in the same surface as the controls that caused it. It
  // reads only stores that already exist (`captureControls` for the session,
  // `systemFacts` for the machine) — no new data flow — and it is bound by G8:
  // a figure with no real source renders as nothing, never as a placeholder.

  import { tick } from "svelte";
  import { goto } from "$app/navigation";
  import { captureControls, sourceSelection } from "$lib/capture-controls.svelte";
  import { systemFacts } from "./state/system-facts.svelte";
  import { formatBytes } from "./state/format";
  import IconBack from "~icons/lucide/chevron-left";
  import IconSearch from "~icons/lucide/search";
  import IconClear from "~icons/lucide/x";
  import { GROUP_ICONS } from "./section-icons";
  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
    type SettingsSectionId,
  } from "./groups";
  import { filterGroups } from "./rail-filter";
  import { indexedRowCounts } from "./settings-index";
  import { settingsFind } from "./state/settings-find.svelte";
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

  // Slice 4: the search field narrows the nav as you type. The (pure) filter
  // helper lives in `rail-filter.ts`; here we only bind state + render.
  let searchQuery = $state("");
  let searchInput = $state<HTMLInputElement | null>(null);

  // The visible (filtered) groups — what the nav actually renders. An empty or
  // whitespace query is a pass-through (all groups). A no-match query yields [].
  const visibleGroups = $derived(filterGroups(SETTINGS_GROUPS, searchQuery));

  // The indexed row count printed on each group row. While ⌘F is filtering it
  // counts the MATCHING rows, so the rail and the filtered pane agree on how
  // many hits live where. Both numbers come from the one index whose
  // completeness a test enforces — nothing here is hand-counted.
  const rowCounts = $derived(
    indexedRowCounts(settingsFind.active ? settingsFind.query : ""),
  );

  // The keyboard roving model, in rendered order: every group row, plus the
  // ACTIVE group's disclosed sections (a collapsed group's sections are not in
  // the DOM, so they must not be steppable either). Group rows navigate to the
  // group's first section.
  interface RailEntry {
    /** DOM id of the button. */
    domId: string;
    /** The section a click/keyboard activation navigates to. */
    section: SettingsSectionId;
  }

  const flatItems = $derived<RailEntry[]>(
    visibleGroups.flatMap((group) => [
      { domId: `settings-group-${group.id}`, section: group.sections[0].id },
      ...(group.id === activeGroup
        ? group.sections.map((s) => ({
            domId: `settings-tab-${s.id}`,
            section: s.id,
          }))
        : []),
    ]),
  );

  // The single roving-tabindex target: the one visible item that gets
  // `tabindex=0` (so the rail is reachable by Tab). It's the active section
  // while that row is disclosed; otherwise the first visible item, so the list
  // never becomes entirely `tabindex=-1` (unreachable by keyboard).
  const rovingTarget = $derived<string | undefined>(
    flatItems.some((e) => e.domId === `settings-tab-${activeSection}`)
      ? `settings-tab-${activeSection}`
      : flatItems[0]?.domId,
  );

  // Clearing the query on blur must NOT eat a click on a nav item: a click that
  // moves focus out of the input fires `blur` BEFORE the item's `click`. If we
  // cleared synchronously here, the item would unmount before its click landed.
  // Defer the clear to a macrotask so the pending click + navigation fire first.
  function clearSearch() {
    searchQuery = "";
  }
  // The in-field clear button and the empty-state "Clear search" CTA both wipe
  // the query and return focus to the input so the user can keep typing without
  // a second click — distinct from `clearSearch` (the deferred blur path).
  function clearSearchAndFocus() {
    searchQuery = "";
    searchInput?.focus();
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

  // ─── The rail footer: the cost of what you enabled ────────────────────────
  // Line 1 — what is recording right now. `requestedSources` minus
  // `maskedSources` is what is ACTUALLY capturing (a masked source is a
  // user-scoped pause on one source), so the list never over-claims.
  void systemFacts.ensureLoaded();

  // `sourceSelection.isSelected` already IS this answer ("live sources while
  // recording, settings while idle") — reuse it rather than re-deriving the
  // requested-minus-masked arithmetic a second time.
  const activeSources = $derived(
    (
      [
        ["screen", "screen"],
        ["microphone", "microphone"],
        ["systemAudio", "system audio"],
      ] as const
    )
      .filter(([key]) => sourceSelection.isSelected(key))
      .map(([, label]) => label),
  );

  // "Recording · all three sources" only when all three genuinely are.
  const sessionPhrase = $derived.by(() => {
    if (!captureControls.isRunning) return "Not recording";
    const verb = captureControls.paused ? "Paused" : "Recording";
    if (activeSources.length === 0) return verb;
    if (activeSources.length === 3) return `${verb} · all three sources`;
    return `${verb} · ${activeSources.join(" + ")}`;
  });

  // Line 2 — the two figures this machine can actually produce. The mockup's
  // "270 MB today / 34.2 GB kept" has no honest source (`SystemFacts` measures
  // a per-day AVERAGE over complete capture days and free space; it carries no
  // partial-day total and no total-kept figure), so G8 substitutes the two real
  // ones and drops whichever is null.
  const dailyRate = $derived.by(() => {
    const bytes = systemFacts.value?.measuredBytesPerDay;
    return bytes == null ? null : `${formatBytes(bytes)} a day`;
  });
  const freeSpace = $derived.by(() => {
    const bytes = systemFacts.value?.diskFreeBytes;
    return bytes == null ? null : `${formatBytes(bytes)} free`;
  });

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
    const focusedIndex = focusedTab
      ? flatItems.findIndex((e) => e.domId === focusedTab.id)
      : -1;
    // Anchor nav on the focused row; else the active section if it's still
    // visible; else the first survivor (a query may have filtered the active
    // section out, but the visible survivors must still be steppable). Bail only
    // when there's genuinely nothing to navigate.
    const activeIndex = flatItems.findIndex(
      (e) => e.domId === `settings-tab-${activeSection}`,
    );
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
    // Moving the roving focus activates the item (focus + navigate). Stepping
    // onto a collapsed group discloses it, which changes what comes next — so
    // focus the row by the id it had BEFORE the disclosure re-render, which is
    // still this row's id either way.
    const domId = next.domId;
    onNavigate(next.section);
    void tick().then(() => document.getElementById(domId)?.focus());
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
      {#if searchQuery}
        <button
          class="rail-search__clear"
          type="button"
          aria-label="Clear search"
          use:tip={"Clear search"}
          onclick={clearSearchAndFocus}
        >
          <IconClear aria-hidden="true" />
        </button>
      {/if}
    </div>
  </div>

  <!-- Scrolling nav: a NAVIGATION landmark whose items are buttons that scroll
       the single settings panel to a section (scroll-spy drives the active one).
       Modeled as a list of groups rather than a tablist — every "tab" would have
       pointed at the one mounted panel id, and most targets are unmounted, which
       a real tablist mustn't do. The active section carries aria-current="page".
       The roving-tabindex keyboard stepping (arrow/Home/End) lives on the
       .nav-item buttons themselves — keeping the keydown on the interactive
       elements, not the non-interactive role="list" container.

       Five group rows; only the active group discloses its sections (page 11).
       The group row is the accent-filled one — it says which pane you are in;
       the disclosed section reads one level quieter. -->
  <nav class="settings-nav rail-nav" aria-label="Settings sections">
    <div class="rail-nav__list" role="list">
      {#each visibleGroups as group (group.id)}
        {@const GroupIcon = GROUP_ICONS[group.id]}
        <!-- Disclosed when this is the pane you are in — or whenever the rail's
             own search is narrowing the nav, since the whole point of a
             surviving section is to be clickable. While ⌘F is filtering there
             is no "pane you are in" (every group's rows are in one list), so
             nothing is filled or disclosed; the count beside each group is how
             many of its rows matched. -->
        {@const open =
          (group.id === activeGroup || searchQuery.trim() !== "") && !settingsFind.active}
        <button
          class="nav-item nav-item--group"
          class:nav-item--active={group.id === activeGroup && !settingsFind.active}
          type="button"
          id="settings-group-{group.id}"
          aria-expanded={open}
          tabindex={rovingTarget === `settings-group-${group.id}` ? 0 : -1}
          onclick={() => onNavigate(group.sections[0].id)}
          onkeydown={handleNavKeydown}
        >
          <GroupIcon aria-hidden="true" />
          <span>{group.label}</span>
          <span class="nav-item__n" aria-label="{rowCounts[group.id]} settings">
            {rowCounts[group.id]}
          </span>
        </button>
        {#if open}
          {#each group.sections as section (section.id)}
            <button
              class="nav-item nav-item--sub"
              class:nav-item--sub-active={activeSection === section.id}
              type="button"
              id="settings-tab-{section.id}"
              data-section={section.id}
              aria-current={activeSection === section.id ? "page" : undefined}
              tabindex={rovingTarget === `settings-tab-${section.id}` ? 0 : -1}
              onclick={() => onNavigate(section.id)}
              onkeydown={handleNavKeydown}
            >
              <span>{section.label}</span>
            </button>
          {/each}
        {/if}
      {/each}
      {#if visibleGroups.length === 0}
        <div class="rail-empty" role="status">
          <p class="rail-empty__msg">No settings match “{searchQuery.trim()}”.</p>
          <button class="btn btn--ghost btn--sm" type="button" onclick={clearSearchAndFocus}>
            Clear search
          </button>
        </div>
      {/if}
    </div>
  </nav>

  <!-- The cost of what you enabled. Not save state (G7 owns that in the top
       strip) — a standing readout of the session and the machine. -->
  <div class="rail-foot">
    <p class="rail-foot__line">
      <span
        class="rail-foot__dot"
        class:rail-foot__dot--on={captureControls.isRunning && !captureControls.paused}
        class:rail-foot__dot--paused={captureControls.isRunning && captureControls.paused}
        aria-hidden="true"
      ></span>
      {sessionPhrase}
    </p>
    {#if dailyRate || freeSpace}
      <p class="rail-foot__line">
        {#if dailyRate}<span class="rail-foot__num">{dailyRate}</span>{/if}
        {#if freeSpace}
          <span class="rail-foot__num" class:rail-foot__num--trail={dailyRate}>{freeSpace}</span>
        {/if}
      </p>
    {/if}
  </div>
</aside>

<style>
  /* Slice 4: quiet empty state when the search query matches no sections.
     Namespaced with the rail's `rail-`/`nav-` family; tokens-only (muted text
     to match `.nav-cat`). Component-scoped so it never touches the shared
     `.settings-shell` cascade in settings-layout.css. */
  .rail-empty {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    margin: 4px 8px;
    padding: 6px 10px;
    font-size: 12px;
    line-height: 1.4;
    color: var(--app-text-subtle);
  }

  .rail-empty__msg {
    margin: 0;
    word-break: break-word;
  }

  /* In-field clear (X) for the search input. Sits at the right edge of the
     recessed search field, mirroring the leading search glyph; tokens-only and
     namespaced under the rail's search family. */
  .rail-search__clear {
    position: absolute;
    right: 6px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 5px;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: color 0.15s, background 0.15s;
  }
  .rail-search__clear:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }
  .rail-search__clear:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .rail-search__clear :global(svg) {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

</style>
