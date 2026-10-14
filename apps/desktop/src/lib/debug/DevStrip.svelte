<!--
  The developer window's strip (developer.html `.dv-strip`): lights + drag
  region from WindowStrip, title "Developer", and the right-aligned actions —
  pause · keep on top · mini · ⋯ (Copy diagnostics, Reveal logs, Open app log).

  `compact` is the mini frame (`.dv-mini`): a 28px strip with smaller lights, a
  pin glyph and an expand button, around the mini content (MiniView, F2). A
  click anywhere below the strip expands too.
-->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { writeText } from "@tauri-apps/plugin-clipboard-manager";
  import WindowStrip from "$lib/components/WindowStrip.svelte";
  import { tip } from "$lib/components/tooltip";
  import { humanizeError } from "$lib/format-error";
  import type { AppLogTailDto } from "$lib/types";
  import IconPause from "~icons/lucide/pause";
  import IconPlay from "~icons/lucide/play";
  import IconPin from "~icons/lucide/pin";
  import IconShrink from "~icons/lucide/minimize-2";
  import IconExpand from "~icons/lucide/maximize-2";
  import IconMore from "~icons/lucide/ellipsis";
  import IconCopy from "~icons/lucide/copy";
  import IconFolder from "~icons/lucide/folder";
  import IconLog from "~icons/lucide/file-text";
  import IconOk from "~icons/lucide/check";
  import IconFail from "~icons/lucide/circle-alert";
  import { getDebugController } from "./state/controller.svelte";
  import { collectDiagnostics } from "./diagnostics";
  import { enterMini, exitMini, isPinned, setPinned } from "./window-mode";

  let { compact = false, children }: { compact?: boolean; children?: Snippet } = $props();

  const controller = getDebugController();
  const pauseLabel = $derived(controller.live ? "Pause live updates" : "Resume live updates");

  let pinned = $state(isPinned());
  let switching = false;

  async function togglePin() {
    try {
      await setPinned(!pinned);
    } catch (err) {
      console.error("[developer] keep on top failed", err);
    }
    pinned = isPinned();
  }

  // Content never outgrows the window mid-switch: go mini before shrinking,
  // grow before going full.
  async function toMini() {
    if (switching) return;
    switching = true;
    menuOpen = false;
    controller.mode = "mini";
    try {
      await enterMini();
    } catch (err) {
      console.error("[developer] mini mode failed", err);
    } finally {
      switching = false;
    }
  }

  async function expand() {
    if (switching) return;
    switching = true;
    try {
      await exitMini();
    } catch (err) {
      console.error("[developer] expand failed", err);
    } finally {
      controller.mode = "full";
      switching = false;
    }
  }

  function onMiniClick(event: MouseEvent) {
    // The strip is the drag region (and holds the close light); only the body expands.
    if ((event.target as Element).closest(".mx-strip")) return;
    void expand();
  }

  // ── ⋯ menu ──
  let menuOpen = $state(false);
  let moreEl = $state<HTMLButtonElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    if (!menuOpen) return;
    void tick().then(() => popEl?.querySelector<HTMLElement>("[role=menuitem]")?.focus({ preventScroll: true }));
    return () => {
      if (popEl?.contains(document.activeElement)) moreEl?.focus({ preventScroll: true });
    };
  });

  function onWindowPointerDown(event: PointerEvent) {
    const target = event.target as Node | null;
    if (!menuOpen || !target || popEl?.contains(target) || moreEl?.contains(target)) return;
    menuOpen = false;
  }

  // Capture phase: Esc closes the menu before the layout's close-the-window
  // handler sees it, and ⇧⌘C works from anywhere in the window.
  $effect(() => {
    const onKeydown = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      if (event.key === "Escape" && menuOpen) {
        event.preventDefault();
        menuOpen = false;
      } else if (event.code === "KeyC" && event.metaKey && event.shiftKey && !event.altKey && !event.ctrlKey) {
        event.preventDefault();
        void copyDiagnostics();
      }
    };
    window.addEventListener("keydown", onKeydown, true);
    return () => window.removeEventListener("keydown", onKeydown, true);
  });

  function run(action: () => Promise<void>) {
    menuOpen = false;
    void action();
  }

  function onItemKeydown(event: KeyboardEvent, action: () => Promise<void>) {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    run(action);
  }

  async function copyDiagnostics() {
    try {
      await writeText(await collectDiagnostics(controller.health.entries));
      showToast({ tone: "ok", text: "Diagnostics copied · health, versions, queues, last 200 log lines" });
    } catch (err) {
      showToast({ tone: "danger", text: `Couldn’t copy diagnostics · ${humanizeError(err)}` });
    }
  }

  async function revealLogs() {
    try {
      const { path } = await invoke<AppLogTailDto>("tail_app_log", { lines: 1 });
      await revealItemInDir(path);
    } catch (err) {
      showToast({ tone: "danger", text: `Couldn’t reveal logs · ${humanizeError(err)}` });
    }
  }

  async function openAppLog() {
    try {
      await invoke("open_general_app_log");
    } catch (err) {
      showToast({ tone: "danger", text: `Couldn’t open the app log · ${humanizeError(err)}` });
    }
  }

  // ── Toast (kit `.mx-toast`, hand-rolled like SubjectDetail) ──
  let toast = $state<{ tone: "ok" | "danger"; text: string } | null>(null);
  let toastTimer: ReturnType<typeof setTimeout> | undefined;
  function showToast(next: { tone: "ok" | "danger"; text: string }) {
    clearTimeout(toastTimer);
    toast = next;
    toastTimer = setTimeout(() => (toast = null), 4000);
  }
  $effect(() => () => clearTimeout(toastTimer));
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

{#if compact}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions — the expand button is the keyboard path -->
  <div class="dv-mini" role="dialog" aria-label="Developer, mini" tabindex="-1" use:tip={"Click to expand"} onclick={onMiniClick}>
    <WindowStrip title="Developer">
      {#snippet actions()}
        <IconPin class="dv-mini__pin" width="12" height="12" aria-label="Always on top" role="img" />
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" style="--_h:20px" aria-label="Expand" onclick={expand}>
          <IconExpand width="12" height="12" aria-hidden="true" />
        </button>
      {/snippet}
    </WindowStrip>
    {@render children?.()}
  </div>
{:else}
  <WindowStrip title="Developer">
    {#snippet actions()}
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm dv-pause"
        aria-pressed={!controller.live}
        aria-label={pauseLabel}
        use:tip={pauseLabel}
        onclick={controller.toggleLive}
      >
        {#if controller.live}
          <IconPause width="12" height="12" aria-hidden="true" /><span class="dv-live"></span>
        {:else}
          <IconPlay width="12" height="12" aria-hidden="true" />
        {/if}
      </button>
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
        aria-pressed={pinned}
        aria-label="Keep on top"
        use:tip={"Keep on top"}
        onclick={togglePin}
      ><IconPin width="14" height="14" aria-hidden="true" /></button>
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
        aria-label="Mini mode"
        use:tip={"Mini mode"}
        onclick={toMini}
      ><IconShrink width="14" height="14" aria-hidden="true" /></button>
      <div class="mx-pop-anchor">
        <button
          bind:this={moreEl}
          type="button"
          class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
          aria-label="More"
          aria-haspopup="menu"
          aria-expanded={menuOpen}
          use:tip={"More"}
          onclick={() => (menuOpen = !menuOpen)}
        ><IconMore width="15" height="15" aria-hidden="true" /></button>
        <div class="mx-pop" data-open={menuOpen ? "" : undefined} role="menu" aria-hidden={!menuOpen} inert={!menuOpen} bind:this={popEl}>
          <ul class="mx-menu">
            <li role="menuitem" tabindex="0" onclick={() => run(copyDiagnostics)} onkeydown={(e) => onItemKeydown(e, copyDiagnostics)}>
              <IconCopy width="14" height="14" aria-hidden="true" />Copy diagnostics<span class="mx-menu__meta">⇧⌘C</span>
            </li>
            <li role="menuitem" tabindex="0" onclick={() => run(revealLogs)} onkeydown={(e) => onItemKeydown(e, revealLogs)}>
              <IconFolder width="14" height="14" aria-hidden="true" />Reveal logs in Finder
            </li>
            <li role="menuitem" tabindex="0" onclick={() => run(openAppLog)} onkeydown={(e) => onItemKeydown(e, openAppLog)}>
              <IconLog width="14" height="14" aria-hidden="true" />Open app log
            </li>
          </ul>
        </div>
      </div>
    {/snippet}
  </WindowStrip>
{/if}

{#if toast}
  <div class="mx-toasts dv-toasts">
    <div class="mx-toast" data-tone={toast.tone} role={toast.tone === "danger" ? "alert" : "status"}>
      {#if toast.tone === "ok"}<IconOk width="15" height="15" aria-hidden="true" />{:else}<IconFail width="15" height="15" aria-hidden="true" />{/if}
      <span>{toast.text}</span>
    </div>
  </div>
{/if}

<style>
  /* No status bar in this window, and the strip is position: relative — pin to the window. */
  .dv-toasts { position: fixed; bottom: var(--s-4); }

  /* Mini frame (developer.html `.mx-window.dv-mini`). */
  .dv-mini { flex: 1 1 auto; min-height: 0; display: grid; grid-template-rows: 28px minmax(0, 1fr); background: var(--app-surface); cursor: pointer; outline: none; }
  .dv-mini :global(.mx-strip) { height: 28px; padding-right: 4px; cursor: default; }
  .dv-mini :global(.mx-strip__lights) { width: 54px; gap: 6px; padding-left: 9px; }
  .dv-mini :global(.mx-strip__lights > *) { width: 9px; height: 9px; }
  .dv-mini :global(.mx-strip__title) { font-size: var(--text-sm); }
  .dv-mini :global(.dv-mini__pin) { color: var(--app-text-subtle); margin-right: 2px; }
</style>
