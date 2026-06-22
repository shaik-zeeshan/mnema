<script lang="ts">
  // Settings left rail — Slice-5 shell-ification.
  //
  // Renders the 5 navigation groups from `groups.ts` (the source of truth),
  // the collapse toggle (+ its localStorage pref and auto-collapse breakpoint),
  // the live save-status line, and the capture-summary footer. The rail is a
  // WAI-ARIA tablist with roving tabindex + arrow/Home/End keyboard nav. Group
  // selection is driven by the shell via the bound `activeGroup`.

  import {
    SETTINGS_GROUPS,
    type SettingsGroupId,
  } from "./groups";
  import { getSettingsController } from "./state/controller.svelte";

  interface Props {
    activeGroup: SettingsGroupId;
    /** Bound: the shell's measured shell element (for the resize observer). */
    shellEl?: HTMLElement | null;
  }

  let { activeGroup = $bindable(), shellEl = null }: Props = $props();

  const c = getSettingsController();
  const rec = c.rec;
  const keyboard = c.keyboard;
  const audio = c.audio;

  // ─── Sidebar collapse ────────────────────────────────────────────────────
  const SIDEBAR_COLLAPSE_STORAGE_KEY = "mnema.settings.sidebarCollapsed";
  const SIDEBAR_AUTO_COLLAPSE_WIDTH = 640;

  let userSidebarCollapsed = $state(false);
  let shellWidth = $state(Number.POSITIVE_INFINITY);

  const autoSidebarCollapsed = $derived(shellWidth < SIDEBAR_AUTO_COLLAPSE_WIDTH);
  const sidebarCollapsed = $derived(autoSidebarCollapsed || userSidebarCollapsed);

  $effect(() => {
    if (typeof localStorage === "undefined") return;
    const stored = localStorage.getItem(SIDEBAR_COLLAPSE_STORAGE_KEY);
    if (stored !== null) userSidebarCollapsed = stored === "1";
  });

  // Measure the shell element (passed from the shell) so the breakpoint
  // reflects the width available to the rail + content.
  $effect(() => {
    const el = shellEl;
    if (!el || typeof ResizeObserver === "undefined") return;
    shellWidth = el.clientWidth;
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) shellWidth = entry.contentRect.width;
    });
    observer.observe(el);
    return () => observer.disconnect();
  });

  function toggleSidebar() {
    if (autoSidebarCollapsed) return;
    userSidebarCollapsed = !userSidebarCollapsed;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(SIDEBAR_COLLAPSE_STORAGE_KEY, userSidebarCollapsed ? "1" : "0");
    }
  }

  // ⌘B / Ctrl-B toggles the rail; skipped while a field has focus.
  $effect(() => {
    if (typeof window === "undefined") return;
    const onKeydown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey) return;
      if (event.key !== "b" && event.key !== "B") return;
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target?.isContentEditable) return;
      event.preventDefault();
      toggleSidebar();
    };
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  // ─── Tablist keyboard nav (roving tabindex) ──────────────────────────────
  function handleTabKeydown(event: KeyboardEvent) {
    const focusedTab = event.target instanceof Element
      ? event.target.closest<HTMLElement>('[role="tab"]')
      : null;
    const focusedGroupId = focusedTab?.id?.replace(/^settings-tab-/, "") ?? null;
    const focusedIndex = SETTINGS_GROUPS.findIndex((g) => g.id === focusedGroupId);
    const currentIndex = focusedIndex >= 0
      ? focusedIndex
      : SETTINGS_GROUPS.findIndex((g) => g.id === activeGroup);
    if (currentIndex === -1) return;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight" || event.key === "ArrowDown") {
      nextIndex = (currentIndex + 1) % SETTINGS_GROUPS.length;
    } else if (event.key === "ArrowLeft" || event.key === "ArrowUp") {
      nextIndex = (currentIndex - 1 + SETTINGS_GROUPS.length) % SETTINGS_GROUPS.length;
    } else if (event.key === "Home") {
      nextIndex = 0;
    } else if (event.key === "End") {
      nextIndex = SETTINGS_GROUPS.length - 1;
    }
    if (nextIndex === null) return;
    event.preventDefault();
    event.stopPropagation();
    const nextGroup = SETTINGS_GROUPS[nextIndex];
    activeGroup = nextGroup.id;
    const el = document.getElementById(`settings-tab-${nextGroup.id}`);
    el?.focus();
  }
</script>

<!-- Sidebar nav glyphs — one per group, drawn on a 24 viewBox with a 1.8
     stroke so the rail reads as one icon family. -->
{#snippet navIcon(kind: SettingsGroupId)}
  <span class="settings-nav__icon" aria-hidden="true">
    {#if kind === "general"}
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z" />
      </svg>
    {:else if kind === "capture"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8" />
        <path d="M12 17v4" />
      </svg>
    {:else if kind === "intelligence"}
      <svg viewBox="0 0 24 24">
        <path d="M12 3a4 4 0 0 0-4 4 3 3 0 0 0-1 5.8V17a3 3 0 0 0 5 2" />
        <path d="M12 3a4 4 0 0 1 4 4 3 3 0 0 1 1 5.8V17a3 3 0 0 1-5 2" />
        <path d="M12 3v18" />
      </svg>
    {:else if kind === "data"}
      <svg viewBox="0 0 24 24">
        <ellipse cx="12" cy="6" rx="7" ry="3" />
        <path d="M5 6v12c0 1.7 3.1 3 7 3s7-1.3 7-3V6" />
        <path d="M5 12c0 1.7 3.1 3 7 3s7-1.3 7-3" />
      </svg>
    {:else if kind === "about"}
      <svg viewBox="0 0 24 24">
        <circle cx="12" cy="12" r="9" />
        <path d="M12 11v6" />
        <path d="M12 7h.01" />
      </svg>
    {/if}
  </span>
{/snippet}

<!-- Capture-summary glyphs — same 24 viewBox / 1.8 stroke family. -->
{#snippet captureStatIcon(kind: "screen" | "mic" | "sysaudio")}
  <span class="status-pill__icon" aria-hidden="true">
    {#if kind === "screen"}
      <svg viewBox="0 0 24 24">
        <rect x="3" y="5" width="18" height="12" rx="2" />
        <path d="M8 21h8" />
        <path d="M12 17v4" />
      </svg>
    {:else if kind === "mic"}
      <svg viewBox="0 0 24 24">
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0" />
        <path d="M12 18v3" />
        <path d="M9 21h6" />
      </svg>
    {:else}
      <svg viewBox="0 0 24 24">
        <path d="M11 5 6 9H3v6h3l5 4Z" />
        <path d="M16 9a5 5 0 0 1 0 6" />
        <path d="M19 7a8 8 0 0 1 0 10" />
      </svg>
    {/if}
  </span>
{/snippet}

<aside
  id="settings-sidebar"
  class="settings-sidebar"
  class:settings-sidebar--collapsed={sidebarCollapsed}
>
  <div class="settings-sidebar__head">
    <div class="settings-sidebar__titlebar">
      <h1 class="settings-sidebar__title">Settings</h1>
      <button
        class="settings-sidebar__toggle"
        type="button"
        onclick={toggleSidebar}
        disabled={autoSidebarCollapsed}
        aria-expanded={!sidebarCollapsed}
        aria-controls="settings-sidebar"
        aria-keyshortcuts="Meta+B Control+B"
        aria-label={sidebarCollapsed ? "Expand sidebar" : "Collapse sidebar"}
        title={autoSidebarCollapsed
          ? "Widen the window to expand"
          : sidebarCollapsed
            ? "Expand sidebar (⌘B)"
            : "Collapse sidebar (⌘B)"}
      >
        <svg class="settings-sidebar__toggle-icon" viewBox="0 0 24 24" aria-hidden="true">
          <path d="M15 6l-6 6 6 6" />
        </svg>
      </button>
    </div>
    <div class="settings-sidebar__status" aria-live="polite">
      {#if rec.recError || keyboard.keyboardBindingsError || audio.micError}
        <span class="status-text status-text--error"><span class="status-text__label">save failed</span></span>
      {:else if c.recSaveBlocked || audio.micApplyBlocked}
        <span class="status-text status-text--blocked"><span class="status-text__label">resolve issues</span></span>
      {:else if c.savingRecSettings || keyboard.savingKeyboardBindings || audio.savingMicSettings}
        <span class="status-text status-text--saving"><span class="status-text__label">saving</span></span>
      {:else if rec.recSaved || keyboard.keyboardBindingsSaved || audio.micSaved}
        <span class="status-text status-text--ok"><span class="status-text__label">saved</span></span>
      {:else}
        <span class="status-text"><span class="status-text__label">auto-save on</span></span>
      {/if}
    </div>
  </div>

  <nav class="settings-nav" aria-label="Settings categories">
    <div class="settings-nav__list" role="tablist" tabindex="-1" onkeydown={handleTabKeydown}>
      {#each SETTINGS_GROUPS as group (group.id)}
        <button
          class="settings-nav__item"
          class:settings-nav__item--active={activeGroup === group.id}
          role="tab"
          aria-selected={activeGroup === group.id}
          aria-controls="settings-panel-{group.id}"
          aria-label={sidebarCollapsed ? group.label : null}
          id="settings-tab-{group.id}"
          tabindex={activeGroup === group.id ? 0 : -1}
          title={sidebarCollapsed ? group.label : null}
          onkeydown={handleTabKeydown}
          onclick={() => { activeGroup = group.id; }}
          type="button"
        >
          {@render navIcon(group.id)}
          <span class="settings-nav__text">
            <span class="settings-nav__label">{group.label}</span>
            <span class="settings-nav__hint">{group.description}</span>
          </span>
        </button>
      {/each}
    </div>
  </nav>

  {#if rec.recordingSettings}
    <div class="settings-sidebar__foot">
      <span class="settings-sidebar__foot-label">Capture summary</span>
      <ul class="status-strip" aria-label="Current capture summary">
        <li
          class="status-pill"
          class:status-pill--on={rec.draftCaptureScreen}
          title={sidebarCollapsed ? `Screen ${rec.draftCaptureScreen ? "on" : "off"}` : null}
        >
          {@render captureStatIcon("screen")}
          <span class="status-pill__dot"></span>
          <span class="status-pill__label">Screen</span>
        </li>
        <li
          class="status-pill"
          class:status-pill--on={rec.draftCaptureMicrophone}
          title={sidebarCollapsed ? `Mic ${rec.draftCaptureMicrophone ? "on" : "off"}` : null}
        >
          {@render captureStatIcon("mic")}
          <span class="status-pill__dot"></span>
          <span class="status-pill__label">Mic</span>
        </li>
        <li
          class="status-pill"
          class:status-pill--on={rec.draftCaptureSystemAudio}
          title={sidebarCollapsed ? `System audio ${rec.draftCaptureSystemAudio ? "on" : "off"}` : null}
        >
          {@render captureStatIcon("sysaudio")}
          <span class="status-pill__dot"></span>
          <span class="status-pill__label">Sys Audio</span>
        </li>
        <li class="status-pill status-pill--info">
          <span class="status-pill__label">{rec.draftFrameRate}fps</span>
        </li>
        <li class="status-pill status-pill--info">
          <span class="status-pill__label">
            {#if rec.draftResolutionMode === "original"}original{:else if rec.draftResolutionMode === "preset"}{rec.draftResolutionPreset}{:else}custom{/if}
          </span>
        </li>
      </ul>
    </div>
  {/if}
</aside>
