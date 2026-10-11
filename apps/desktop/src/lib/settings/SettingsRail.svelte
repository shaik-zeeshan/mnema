<script lang="ts">
  // Settings rail — the full-height 252px sidebar of the full-window Settings
  // mode (settings.html `.st-side`): a 52px drag strip that reserves the native
  // traffic lights, "Back to app" (Esc), a search field (`/`), the five groups
  // as tinted-tile buttons with the active group's sections nested beneath, and
  // a REC + lanes + version footer. Save status lives in the content header
  // (SettingsBar.svelte), not here.
  //
  // The nav is a NAVIGATION landmark, not a tablist: settings is one scrolling
  // panel with scroll-spy, so items behave like in-page links and the active one
  // carries `aria-current`. Sub-section items keep a roving tabindex with
  // arrow/Home/End stepping across every visible section (stepping into a closed
  // group opens it). Search narrows the rail (`rail-filter.ts`); while a query is
  // active every surviving group is open and shows its match count.

  import { goto } from "$app/navigation";
  import { tick } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import IconBack from "~icons/lucide/arrow-left";
  import IconSearch from "~icons/lucide/search";
  import IconChev from "~icons/lucide/chevron-right";
  import { GROUP_ICONS } from "./section-icons";
  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
    type SettingsSection,
    type SettingsSectionId,
  } from "./groups";
  import { filterGroups, flattenSections } from "./rail-filter";
  import { getSettingsController } from "./state/controller.svelte";
  import { getLastMainSurface } from "$lib/surface-windows";
  import { captureControls, sourceSelection, subscribeRuntimeSources } from "$lib/capture-controls.svelte";
  import { captureSession } from "$lib/session.svelte";
  import { renderIdle } from "$lib/render-idle.svelte";
  import { LANES, formatClock, liveLanes, recView, type StatusInput } from "$lib/shell/status-bar";

  interface Props {
    /** The active group (the one group panel currently mounted). */
    activeGroup: SettingsGroupId;
    /** The active sub-section — drives the rail's active item. */
    activeSection: SettingsSectionId;
    /** Called on click / keyboard activation of a group or sub-section. */
    onNavigate: (section: SettingsSectionId) => void;
  }

  let { activeGroup, activeSection, onNavigate }: Props = $props();

  const c = getSettingsController();

  // Tile tint per group (settings.html `.st-tile --t`).
  const TINT: Record<SettingsGroupId, string> = {
    general: "var(--chart-grey-4)",
    capture: "var(--app-source-screen)",
    intelligence: "var(--cat-learning)",
    data: "var(--cat-organizing)",
    about: "var(--cat-research)",
  };

  let searchQuery = $state("");
  let searchInput = $state<HTMLInputElement | null>(null);
  const filtering = $derived(searchQuery.trim() !== "");
  const visibleGroups = $derived(filterGroups(SETTINGS_GROUPS, searchQuery));
  const flatItems = $derived<SettingsSection[]>(flattenSections(visibleGroups));
  // The one item with tabindex=0: the active section while visible, else the
  // first visible one so the list never becomes unreachable by Tab.
  const rovingTarget = $derived<SettingsSectionId | undefined>(
    flatItems.some((s) => s.id === activeSection) ? activeSection : flatItems[0]?.id,
  );

  // A click that moves focus out of the field fires `blur` before the item's
  // `click`; clearing synchronously would unmount the item first. Defer it, and
  // don't clear at all when focus stays inside the rail (Tab into results).
  function onSearchBlur(event: FocusEvent) {
    const next = event.relatedTarget;
    if (next instanceof HTMLElement && next.closest("#settings-sidebar")) return;
    setTimeout(() => (searchQuery = ""), 0);
  }
  function onSearchKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      searchQuery = "";
      searchInput?.blur();
    }
  }

  // Settings is the `/settings` route inside the Main window: leaving it is an
  // in-window navigation back to the last main surface (Timeline / Insights).
  function backToApp() {
    void goto(getLastMainSurface());
  }

  // `/` focuses search; Esc goes back to the app — unless the key belongs to a
  // field, an open listbox/menu/modal, or a handler that already claimed it
  // (shortcut rebinding captures keys and prevents default).
  function onWindowKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.isComposing) return;
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('input, textarea, select, [contenteditable="true"], [role="textbox"], [role="combobox"]')) return;
    if (document.querySelector('[role="listbox"], [role="menu"], [aria-modal="true"]')) return;
    if (event.key === "/") {
      event.preventDefault();
      searchInput?.focus();
    } else if (event.key === "Escape" && !event.shiftKey) {
      event.preventDefault();
      backToApp();
    }
  }

  function handleNavKeydown(event: KeyboardEvent) {
    const focused = event.target instanceof Element ? event.target.closest<HTMLElement>(".st-subnav__item") : null;
    const focusedIndex = flatItems.findIndex((s) => s.id === focused?.dataset.section);
    const activeIndex = flatItems.findIndex((s) => s.id === activeSection);
    const current = focusedIndex >= 0 ? focusedIndex : activeIndex >= 0 ? activeIndex : flatItems.length ? 0 : -1;
    if (current === -1) return;

    let next: number | null = null;
    if (event.key === "ArrowDown" || event.key === "ArrowRight") next = (current + 1) % flatItems.length;
    else if (event.key === "ArrowUp" || event.key === "ArrowLeft") next = (current - 1 + flatItems.length) % flatItems.length;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = flatItems.length - 1;
    if (next === null) return;

    event.preventDefault();
    event.stopPropagation();
    const id = flatItems[next].id;
    onNavigate(id);
    // Stepping into a closed group opens it on the next render.
    void tick().then(() => document.getElementById(`settings-tab-${id}`)?.focus());
  }

  // ── Footer: recording state + lanes (display only) + version ─────────────
  const recInput = $derived<StatusInput>({
    running: captureControls.running,
    loadingStart: captureControls.loadingStart,
    userPaused: captureControls.isUserPaused,
    inactivityPaused: captureControls.isInactivityPaused,
    lowDisk: captureControls.isLowDiskSuspended,
    runtime: captureControls.runtimeSources,
    selected: {
      screen: sourceSelection.screen,
      microphone: sourceSelection.microphone,
      systemAudio: sourceSelection.systemAudio,
    },
    permissions: null,
    startError: null,
    readOnly: false,
    // ponytail: the status bar's "STOPPED · disk full" needs the notification
    // history; Settings shows plain NOT RECORDING. Pass it if the footer needs it.
    stoppedReason: null,
  });
  const rec = $derived(recView(recInput));
  const lanes = $derived(liveLanes(recInput, rec));
  const sessionStartMs = $derived.by<number | null>(() => {
    const s = captureSession.value?.sourceSessions;
    const starts = [s?.screen, s?.microphone, s?.systemAudio].flatMap((m) => (m ? [m.startedAtUnixMs] : []));
    return starts.length ? Math.min(...starts) : null;
  });
  let now = $state(Date.now());
  $effect(() => {
    if (rec.clock !== "elapsed") return;
    now = Date.now();
    const handle = setInterval(() => {
      if (!renderIdle()) now = Date.now();
    }, 1000);
    return () => clearInterval(handle);
  });
  $effect(() => {
    if (!captureControls.running) return;
    return subscribeRuntimeSources();
  });
  const LANE_TIP: Record<string, string> = { screen: "screen", mic: "mic", sysaudio: "system audio" };
  const version = $derived(c.about.appUpdateStatus?.app.version ?? null);
</script>

<svelte:window onkeydown={onWindowKeydown} />

<aside id="settings-sidebar" class="st-side" aria-label="Settings sections">
  <div class="st-side__top" data-tauri-drag-region></div>

  <div class="st-side__head">
    <button class="mx-btn mx-btn--ghost mx-btn--sm st-back" type="button" onclick={backToApp}>
      <IconBack width="14" height="14" aria-hidden="true" />Back to app<kbd>Esc</kbd>
    </button>
    <label class="mx-search">
      <IconSearch width="13" height="13" aria-hidden="true" />
      <input
        bind:this={searchInput}
        type="text"
        placeholder="Search settings"
        aria-label="Search settings"
        bind:value={searchQuery}
        onkeydown={onSearchKeydown}
        onblur={onSearchBlur}
      />
      <kbd>/</kbd>
    </label>
  </div>

  <nav class="st-nav" aria-label="Settings sections">
    {#each visibleGroups as group (group.id)}
      {@const GroupIcon = GROUP_ICONS[group.id]}
      {@const open = filtering || group.id === activeGroup}
      {@const current = !filtering && group.id === activeGroup}
      <div class="st-group" class:is-open={open} role="group" aria-labelledby="settings-cat-{group.id}">
        <button
          class="st-group__btn"
          type="button"
          id="settings-cat-{group.id}"
          aria-current={current ? "true" : undefined}
          tabindex="-1"
          onclick={() => onNavigate(group.sections[0].id)}
        >
          <span class="st-tile" style="--t:{TINT[group.id]}"><GroupIcon width="13" height="13" aria-hidden="true" /></span>
          {group.label}
          {#if filtering}
            <span class="st-group__count">{group.sections.length}</span>
          {:else}
            <IconChev class="st-group__chev" width="12" height="12" aria-hidden="true" />
          {/if}
        </button>
        {#if open}
          <div class="st-subnav">
            {#each group.sections as section (section.id)}
              <button
                class="st-subnav__item"
                type="button"
                id="settings-tab-{section.id}"
                data-section={section.id}
                aria-current={activeSection === section.id ? "page" : undefined}
                tabindex={rovingTarget === section.id ? 0 : -1}
                onclick={() => onNavigate(section.id)}
                onkeydown={handleNavKeydown}
              >{section.label}</button>
            {/each}
          </div>
        {/if}
      </div>
    {/each}
    {#if visibleGroups.length === 0}
      <div class="mx-empty st-nav__empty" role="status">
        <p class="mx-empty__text">No settings match “{searchQuery.trim()}”.</p>
        <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => { searchQuery = ""; searchInput?.focus(); }}>
          Clear search
        </button>
      </div>
    {/if}
  </nav>

  <div class="st-side__foot">
    <div class="st-foot__rec">
      <span class="mx-rec{rec.mod ? ` mx-rec--${rec.mod}` : ''}" use:tip={captureControls.running ? "Recording continues while you're in Settings" : null}>
        {#if rec.mod === "starting"}<span class="mx-spin mx-spin--sm"></span>{:else}<span class="mx-rec__dot"></span>{/if}
        {rec.label}{#if rec.clock === "elapsed" && sessionStartMs !== null} <span class="num">{formatClock(now - sessionStartMs, true)}</span>{/if}
      </span>
      <span class="st-foot__lanes">
        {#each lanes as lane, i (lane.key)}
          {@const meta = LANES[i]}
          <!-- Stopped: show the selection for the next recording, as the status bar's toggles do. -->
          {@const mod = captureControls.running || captureControls.loadingStart ? lane.mod : sourceSelection.isSelected(lane.key) ? "on" : "off"}
          <span class="mx-lane mx-lane--{meta.mod} mx-lane--{mod}" use:tip={LANE_TIP[meta.mod]}>{meta.label}</span>
        {/each}
      </span>
    </div>
    <div class="st-foot__meta">Mnema{#if version} <span class="num">{version}</span>{/if}</div>
  </div>
</aside>

<style>
  .st-side {
    flex: none;
    width: 252px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: color-mix(in srgb, var(--app-surface) 62%, var(--app-bg));
    border-right: 1px solid var(--mx-hairline);
  }
  :global([data-theme="light"]) .st-side {
    background: color-mix(in srgb, var(--app-fg) 3.5%, var(--app-bg));
  }
  .st-side__top {
    flex: none;
    height: 52px;
  }

  .st-side__head {
    display: grid;
    gap: var(--s-3);
    padding: 0 var(--s-3) var(--s-3);
  }
  .st-back {
    justify-self: start;
    margin-left: -6px;
  }
  .st-back :global(svg) {
    color: var(--app-text-subtle);
    transition: transform var(--t-med) var(--ease-expo);
  }
  .st-back:hover :global(svg) {
    transform: translateX(-2px);
    color: var(--app-text);
  }
  .st-side .mx-search {
    padding-left: 10px;
    background: color-mix(in srgb, var(--app-fg) 4%, transparent);
    border-color: var(--mx-hairline);
    box-shadow: none;
  }
  .st-side .mx-search input {
    font-size: var(--text-base);
  }
  .st-side .mx-search kbd {
    height: 17px;
    border-bottom-width: 1px;
    background: transparent;
    border-color: var(--mx-hairline);
  }

  .st-nav {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: grid;
    align-content: start;
    gap: 2px;
    padding: var(--s-1) var(--s-3) var(--s-4);
  }
  .st-group {
    display: grid;
  }
  .st-group__btn {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    height: 32px;
    padding: 0 var(--s-2) 0 5px;
    border: 0;
    border-radius: var(--r-md);
    background: none;
    cursor: pointer;
    text-align: left;
    font: 500 var(--text-md)/1 var(--font-sans);
    color: var(--app-text);
    transition: background-color var(--t-fast) var(--ease-quart), color var(--t-fast) var(--ease-quart);
  }
  .st-group__btn:hover {
    background: color-mix(in srgb, var(--app-fg) 5%, transparent);
    color: var(--app-text-strong);
  }
  .st-group__btn[aria-current="true"] {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .st-tile {
    flex: none;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 6px;
    color: var(--t);
    background: color-mix(in srgb, var(--t) 13%, transparent);
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--t) 22%, transparent);
    transition: background-color var(--t-med) var(--ease-quart), color var(--t-med) var(--ease-quart);
  }
  .st-group__btn[aria-current="true"] .st-tile {
    color: var(--app-bg);
    background: var(--t);
    box-shadow: none;
  }
  :global([data-theme="light"]) .st-group__btn[aria-current="true"] .st-tile {
    color: var(--app-surface);
  }
  .st-group__btn :global(.st-group__chev) {
    margin-left: auto;
    color: var(--app-text-faint);
    transition: transform var(--t-med) var(--ease-expo);
  }
  .st-group.is-open :global(.st-group__chev) {
    transform: rotate(90deg);
    color: var(--app-text-subtle);
  }
  .st-group__count {
    margin-left: auto;
    min-width: 18px;
    height: 16px;
    padding: 0 5px;
    display: inline-grid;
    place-items: center;
    border-radius: var(--r-pill);
    font: 500 var(--text-xs)/1 var(--font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }

  /* Nested sections: indented under the label with a hairline guide; the one
     in view gets the --mx-selected fill. */
  .st-subnav {
    position: relative;
    display: grid;
    gap: 1px;
    margin: 2px 0 6px 16px;
    padding-left: 13px;
  }
  .st-subnav::before {
    content: "";
    position: absolute;
    left: 0;
    top: 4px;
    bottom: 4px;
    width: 1px;
    background: var(--mx-hairline);
  }
  .st-subnav__item {
    display: block;
    width: 100%;
    padding: 5px var(--s-2);
    border: 0;
    border-radius: var(--r-sm);
    background: none;
    cursor: pointer;
    text-align: left;
    font: 400 var(--text-base)/1.25 var(--font-sans);
    color: var(--app-text-subtle);
    transition: color var(--t-fast) var(--ease-quart), background-color var(--t-fast) var(--ease-quart);
  }
  .st-subnav__item:hover {
    color: var(--app-text-strong);
    background: color-mix(in srgb, var(--app-fg) 4%, transparent);
  }
  .st-subnav__item[aria-current="page"] {
    color: var(--app-text-strong);
    font-weight: 500;
    background: var(--mx-selected);
  }
  .st-nav__empty {
    padding: var(--s-2);
  }

  .st-side__foot {
    flex: none;
    display: grid;
    gap: 8px;
    padding: var(--s-3) var(--s-4) var(--s-4);
    border-top: 1px solid var(--mx-hairline);
  }
  .st-foot__rec {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }
  .st-foot__rec .mx-rec {
    height: 18px;
    padding: 0 7px 0 6px;
    gap: 6px;
  }
  .st-foot__lanes {
    display: inline-flex;
    gap: 4px;
    margin-left: auto;
  }
  .st-foot__lanes .mx-lane {
    font-size: 0;
    gap: 0;
    text-decoration: none;
  }
  .st-foot__lanes .mx-lane::before {
    width: 6px;
    height: 6px;
  }
  .st-foot__meta {
    display: flex;
    align-items: center;
    gap: 4px;
    font: 400 var(--text-sm)/1 var(--font-sans);
    color: var(--app-text-subtle);
  }
</style>
