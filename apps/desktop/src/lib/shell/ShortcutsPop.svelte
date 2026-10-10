<script lang="ts">
  // The keyboard-shortcuts sheet (decision 24): a 22px ghost ? at the far right
  // of the status bar opening an upward `mx-pop mx-keys`. `/` and `?` toggle the
  // same `keyboardHelp.open` from the layout's key handler. Rows are live: the
  // Global group below plus whatever the current surface registers.
  import { tick } from "svelte";
  import { tip } from "$lib/components/tooltip";
  import { openSettings } from "$lib/surface-windows";
  import { GLOBAL_SHORTCUTS, getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { keyboardHelp, type KeyboardHelpGroup } from "$lib/keyboard-help.svelte";
  import { formatShortcut, getFocusableElements, type KeyboardPlatform, type ShortcutDefinition } from "$lib/keyboard";

  interface Props {
    platform: KeyboardPlatform;
    devEnabled: boolean;
    isCapturing: boolean;
    isUserPaused: boolean;
    canToggleRecording: boolean;
    canToggleSources: boolean;
  }
  let { platform, devEnabled, isCapturing, isUserPaused, canToggleRecording, canToggleSources }: Props = $props();

  let buttonEl = $state<HTMLButtonElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);
  let returnFocusEl: HTMLElement | null = null;

  const relabel = (d: ShortcutDefinition, label: string): ShortcutDefinition => ({ ...d, label });

  const globalGroup = $derived.by<KeyboardHelpGroup>(() => {
    const rows: KeyboardHelpGroup["rows"] = [];
    if (canToggleRecording) {
      rows.push(relabel(getEffectiveGlobalShortcut("toggleRecording"), isCapturing ? "Stop recording" : "Start recording"));
    }
    if (isCapturing) {
      rows.push(
        relabel(getEffectiveGlobalShortcut("pauseResumeRecording"), isUserPaused ? "Resume recording" : "Pause recording"),
      );
    }
    rows.push(getEffectiveGlobalShortcut("toggleMainWindow"));
    rows.push(getEffectiveGlobalShortcut("toggleQuickRecall"));
    rows.push(getEffectiveGlobalShortcut("openSettings"));
    if (devEnabled) rows.push(getEffectiveGlobalShortcut("openDebug"));
    if (canToggleSources) {
      rows.push(
        getEffectiveGlobalShortcut("toggleSourceScreen"),
        getEffectiveGlobalShortcut("toggleSourceMicrophone"),
        getEffectiveGlobalShortcut("toggleSourceSystemAudio"),
      );
    }
    rows.push(getEffectiveGlobalShortcut("toggleShortcutsHelp"), GLOBAL_SHORTCUTS.closeShortcutsHelp);
    return { id: "global", title: "Global", rows };
  });

  const groups = $derived(
    [globalGroup, ...keyboardHelp.contextualGroups]
      .map((g) => ({ ...g, rows: g.rows.filter((r) => r.enabled !== false && r.bindings.length > 0) }))
      .filter((g) => g.rows.length > 0),
  );

  const open = $derived(keyboardHelp.open);

  function onWindowPointerDown(event: PointerEvent): void {
    if (!open) return;
    const target = event.target as Node | null;
    if (!target || popEl?.contains(target) || buttonEl?.contains(target)) return;
    keyboardHelp.open = false;
  }

  // Focus moves in on open and back to where it was on close.
  $effect(() => {
    if (!open) return;
    returnFocusEl = document.activeElement as HTMLElement | null;
    let cancelled = false;
    void tick().then(() => {
      if (!cancelled) getFocusableElements(popEl)[0]?.focus({ preventScroll: true });
    });
    return () => {
      cancelled = true;
      const active = document.activeElement as HTMLElement | null;
      if (!active || active === document.body || popEl?.contains(active)) {
        returnFocusEl?.focus({ preventScroll: true });
      }
    };
  });
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div class="mx-pop-anchor">
  <button
    bind:this={buttonEl}
    type="button"
    class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
    style="--_h:22px"
    aria-label="Keyboard shortcuts"
    aria-haspopup="dialog"
    aria-expanded={open}
    use:tip={`Keyboard shortcuts · ${formatShortcut(getEffectiveGlobalShortcut("toggleShortcutsHelp").bindings[0], platform).join("")}`}
    onclick={() => (keyboardHelp.open = !open)}
  >
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 3-3 3" /><path d="M12 17h.01" /></svg>
  </button>
  <div
    class="mx-pop mx-pop--up mx-keys"
    data-open={open ? "" : undefined}
    role="dialog"
    aria-label="Keyboard shortcuts"
    aria-hidden={!open}
    inert={!open}
    bind:this={popEl}
  >
    <div class="mx-pop__head">
      <span class="mx-kicker">Keyboard shortcuts</span>
      <span class="mx-spacer"></span>
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--sm"
        onclick={() => {
          keyboardHelp.open = false;
          void openSettings("shortcuts");
        }}
      >Rebind the global ones</button>
    </div>
    <div class="mx-keys__cols">
      {#each groups as group (group.id)}
        <div class="mx-keys__grp">
          <span class="mx-label">{group.title}</span>
          <ul>
            {#each group.rows as row (row.id)}
              <li>
                <span>{row.label}</span>
                <span class="mx-keys__k">
                  <kbd>{formatShortcut(row.bindings[0], platform).join("")}</kbd>
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
  </div>
</div>
