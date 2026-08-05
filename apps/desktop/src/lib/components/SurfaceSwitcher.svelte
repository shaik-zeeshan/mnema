<script lang="ts">
  // Title-bar surface switcher (design frame 07): Timeline ⌘1 | Overview ⌘2.
  // Extracted from the +layout `.surface-toggle`; the ⌘1/⌘2 keydown handling
  // itself lives in the layout's global-shortcut handler — this renders the
  // control, navigates on click, and owns the right-click "Open Mnema on"
  // context menu that persists the `ui.main_surface` default.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { tick } from "svelte";
  import { isMainAppRoute, normalizeAppPathname } from "$lib/route-path";
  import { getLastMainSurface, openSettings } from "$lib/surface-windows";
  import {
    getMainSurfaceSetting,
    setMainSurfaceSetting,
    surfaceRoute,
    type MainSurface,
  } from "$lib/main-surface";
  import { GLOBAL_SHORTCUTS } from "$lib/global-shortcuts";
  import { detectKeyboardPlatform, formatShortcut, getFocusableElements } from "$lib/keyboard";

  const platform = detectKeyboardPlatform();
  const kbdTimeline = formatShortcut(GLOBAL_SHORTCUTS.surfaceTimeline.bindings[0], platform).join("");
  const kbdOverview = formatShortcut(GLOBAL_SHORTCUTS.surfaceOverview.bindings[0], platform).join("");
  const kbdSettings = formatShortcut(
    GLOBAL_SHORTCUTS.openSettings.bindings[0],
    platform,
  ).join("");

  const normalizedPathname = $derived(normalizeAppPathname($page.url.pathname));
  const isTimeline = $derived(isMainAppRoute($page.url.pathname));
  const isOverview = $derived(normalizedPathname.startsWith("/insights"));
  const isSettingsRoute = $derived(normalizedPathname === "/settings");

  // On the Settings route neither surface is the current page; the toggle mutes
  // and quietly marks the surface "Back to app" returns to (visual only).
  const settingsReturnsToOverview = $derived(
    isSettingsRoute && normalizeAppPathname(getLastMainSurface()).startsWith("/insights"),
  );
  const settingsReturnsToTimeline = $derived(isSettingsRoute && !settingsReturnsToOverview);

  function goToSurface(surface: MainSurface): void {
    const target = surfaceRoute(surface);
    if (normalizeAppPathname($page.url.pathname) === target) return;
    void goto(target);
  }

  // ── "Open Mnema on" context menu (frame 07) ───────────────────────────
  // A DOM menu (not NSMenu): the switcher is webview chrome, and the app's
  // menu-shaped popovers (notifications, record pill) are DOM already.
  let menuOpen = $state(false);
  let menuLeft = $state(0);
  let menuEl = $state<HTMLDivElement | null>(null);
  let switcherEl = $state<HTMLElement | null>(null);
  // The persisted default surface, loaded when the menu opens; null = loading.
  let defaultSurface = $state<MainSurface | null>(null);
  let savingDefault = $state(false);

  function openMenu(event: MouseEvent): void {
    event.preventDefault();
    menuLeft = switcherEl?.getBoundingClientRect().left ?? event.clientX;
    menuOpen = true;
    defaultSurface = null;
    void getMainSurfaceSetting()
      .then((surface) => {
        defaultSurface = surface;
      })
      .catch(() => {
        // Leave the checkmark blank; picking an item still writes the setting.
      });
    void tick().then(() => {
      getFocusableElements(menuEl)[0]?.focus({ preventScroll: true });
    });
  }

  function closeMenu(): void {
    menuOpen = false;
  }

  function chooseDefault(surface: MainSurface): void {
    savingDefault = true;
    void setMainSurfaceSetting(surface)
      .then(() => {
        defaultSurface = surface;
      })
      .catch(() => {
        // Best-effort: the menu closes either way; Settings shows the truth.
      })
      .finally(() => {
        savingDefault = false;
        closeMenu();
      });
  }

  function onWindowPointerDown(event: PointerEvent): void {
    if (!menuOpen) return;
    const target = event.target as Node | null;
    if (target && (menuEl?.contains(target) || switcherEl?.contains(target))) return;
    closeMenu();
  }

  function onMenuKeydown(event: KeyboardEvent): void {
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      closeMenu();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const items = getFocusableElements(menuEl);
      if (items.length === 0) return;
      const index = items.indexOf(document.activeElement as HTMLElement);
      const delta = event.key === "ArrowDown" ? 1 : -1;
      items[(index + delta + items.length) % items.length]?.focus({ preventScroll: true });
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div
  bind:this={switcherEl}
  class="surface-switcher"
  class:surface-switcher--muted={isSettingsRoute}
  role="navigation"
  aria-label="Main surface"
  oncontextmenu={openMenu}
>
  <button
    type="button"
    class:active={isTimeline}
    class:return-target={settingsReturnsToTimeline}
    aria-current={isTimeline ? "page" : undefined}
    onclick={() => goToSurface("timeline")}
  >
    Timeline
    <kbd class="surface-switcher__kbd" aria-hidden="true">{kbdTimeline}</kbd>
  </button>
  <button
    type="button"
    class:active={isOverview}
    class:return-target={settingsReturnsToOverview}
    aria-current={isOverview ? "page" : undefined}
    onclick={() => goToSurface("overview")}
  >
    Overview
    <kbd class="surface-switcher__kbd" aria-hidden="true">{kbdOverview}</kbd>
  </button>
</div>

{#if menuOpen}
  <div
    bind:this={menuEl}
    class="surface-menu"
    style:left="{menuLeft}px"
    role="menu"
    tabindex="-1"
    aria-label="Open Mnema on"
    onkeydown={onMenuKeydown}
    oncontextmenu={(e) => e.preventDefault()}
  >
    <div class="surface-menu__hd" aria-hidden="true">Open Mnema on</div>
    <button
      type="button"
      class="surface-menu__item"
      role="menuitemradio"
      aria-checked={defaultSurface === "timeline"}
      disabled={savingDefault}
      onclick={() => chooseDefault("timeline")}
    >
      <span class="surface-menu__check" aria-hidden="true">
        {#if defaultSurface === "timeline"}
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 6.5 4.8 9.5 10 2.8" /></svg>
        {/if}
      </span>
      Timeline
      <kbd class="surface-menu__kbd" aria-hidden="true">{kbdTimeline}</kbd>
    </button>
    <button
      type="button"
      class="surface-menu__item"
      role="menuitemradio"
      aria-checked={defaultSurface === "overview"}
      disabled={savingDefault}
      onclick={() => chooseDefault("overview")}
    >
      <span class="surface-menu__check" aria-hidden="true">
        {#if defaultSurface === "overview"}
          <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M2 6.5 4.8 9.5 10 2.8" /></svg>
        {/if}
      </span>
      Overview
      <kbd class="surface-menu__kbd" aria-hidden="true">{kbdOverview}</kbd>
    </button>
    <div class="surface-menu__sep" role="separator"></div>
    <button
      type="button"
      class="surface-menu__item"
      role="menuitem"
      onclick={() => {
        closeMenu();
        void openSettings();
      }}
    >
      <span class="surface-menu__check" aria-hidden="true"></span>
      Settings…
      <kbd class="surface-menu__kbd" aria-hidden="true">{kbdSettings}</kbd>
    </button>
  </div>
{/if}

<style>
  /* ── Switcher (frame 07's NSSegmentedControl-in-the-title-bar) ──────
     Carried over from the +layout `.surface-toggle` contract: active segment is
     signalled by an accent fill alone so the segments stay even-width. */
  .surface-switcher {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
  }
  .surface-switcher button {
    font: inherit;
    font-size: var(--t-ui);
    line-height: 1;
    letter-spacing: 0.02em;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 0 10px;
    height: 22px;
    border: 1px solid transparent;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s ease, border-color 0.12s ease, color 0.12s ease;
  }
  .surface-switcher button:hover {
    color: var(--app-text-strong);
  }
  .surface-switcher button:not(.active):hover {
    background: var(--app-surface-hover);
  }
  .surface-switcher button:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }
  .surface-switcher button:not(:disabled):active {
    transform: translateY(0.5px);
    filter: brightness(0.92);
  }
  .surface-switcher button.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    /* Active = "you are here": --app-accent (AA-legible on accent-bg) + 600. */
    color: var(--app-accent);
    font-weight: 600;
  }
  .surface-switcher__kbd {
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    line-height: 1;
    color: var(--app-text-subtle);
  }
  .surface-switcher button.active .surface-switcher__kbd {
    color: inherit;
    opacity: 0.7;
  }
  /* On the Settings route neither surface is the current page; de-emphasize the
     toggle and quietly mark the surface "Back to app" returns to (no accent
     fill — that's reserved for the active page). */
  .surface-switcher--muted {
    opacity: 0.72;
  }
  .surface-switcher--muted button.return-target {
    color: var(--app-text);
    border-color: var(--app-border);
    background: var(--app-surface-raised);
  }

  /* Degradation ladder (frame 07/11 conventions): the switcher itself never
     drops — only its kbd hints do, then padding tightens. Container widths
     mirror the record pill's titlebar offsets (window ≈ container + 86px). */
  @container titlebar (max-width: 734px) {
    .surface-switcher__kbd {
      display: none;
    }
    .surface-switcher button {
      gap: 0;
    }
  }
  @container titlebar (max-width: 514px) {
    .surface-switcher button {
      padding: 0 8px;
    }
  }

  /* ── "Open Mnema on" context menu ───────────────────────────────────
     Fixed, not absolute: `.titlebar { overflow: hidden }` clips absolute
     descendants (same rationale as the notification popover). */
  .surface-menu {
    position: fixed;
    top: calc(var(--app-titlebar-height) + 6px);
    z-index: 200;
    min-width: 224px;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover, 0 8px 24px rgba(0, 0, 0, 0.32));
  }
  .surface-menu__hd {
    padding: 5px 10px 4px;
    font-size: var(--t-label);
    letter-spacing: 0.04em;
    color: var(--app-text-subtle);
  }
  .surface-menu__item {
    font: inherit;
    font-size: var(--t-ui);
    line-height: 1;
    display: flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    padding: 0 10px 0 4px;
    height: 26px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    text-align: left;
  }
  .surface-menu__item:hover,
  .surface-menu__item:focus-visible {
    outline: none;
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .surface-menu__item:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .surface-menu__check {
    flex: 0 0 auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
  }
  .surface-menu__kbd {
    margin-left: auto;
    font-family: var(--app-font-mono);
    font-size: var(--t-label);
    color: var(--app-text-subtle);
  }
  .surface-menu__item:hover .surface-menu__kbd,
  .surface-menu__item:focus-visible .surface-menu__kbd {
    color: inherit;
    opacity: 0.7;
  }
  .surface-menu__sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--app-border);
  }
</style>
