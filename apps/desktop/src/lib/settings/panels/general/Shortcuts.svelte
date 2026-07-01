<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import IconRestore from "~icons/lucide/rotate-ccw";
  import IconRestoreAll from "~icons/lucide/list-restart";
  import IconClear from "~icons/lucide/x";
  import IconAlert from "~icons/lucide/triangle-alert";

  const c = getSettingsController();
  const keyboard = c.keyboard;

  // ─── Store-derived read aliases (keep markup bare) ───────────────────────────
  const keyboardBindingsSettings = $derived(keyboard.keyboardBindingsSettings);
  const keyboardShortcutIssues = $derived(keyboard.keyboardShortcutIssues);
  const keyboardShortcutSaveBlocked = $derived(keyboard.keyboardShortcutSaveBlocked);
  const shortcutCaptureActionId = $derived(keyboard.shortcutCaptureActionId);
  const savingKeyboardBindings = $derived(keyboard.savingKeyboardBindings);

  // ─── Helper functions (preserve `this`) ──────────────────────────────────────
  const loadKeyboardBindingsSettings = () => keyboard.loadKeyboardBindingsSettings();
  const restoreDefaultShortcuts = () => keyboard.restoreDefaultShortcuts();
  const shortcutCategoryLabel = (category: string) => keyboard.shortcutCategoryLabel(category);
  const shortcutCategoryActions = (category: string) => keyboard.shortcutCategoryActions(category);
  const shortcutDraftBinding = (actionId: Parameters<typeof keyboard.shortcutDraftBinding>[0]) =>
    keyboard.shortcutDraftBinding(actionId);
  const shortcutIssueFor = (actionId: Parameters<typeof keyboard.shortcutIssueFor>[0]) =>
    keyboard.shortcutIssueFor(actionId);
  const shortcutKeyTokens = (binding: string) => keyboard.shortcutKeyTokens(binding);
  const startShortcutCapture = (actionId: Parameters<typeof keyboard.startShortcutCapture>[0]) =>
    keyboard.startShortcutCapture(actionId);
  const clearShortcut = (actionId: Parameters<typeof keyboard.clearShortcut>[0]) =>
    keyboard.clearShortcut(actionId);
  const resetShortcut = (actionId: Parameters<typeof keyboard.resetShortcut>[0]) =>
    keyboard.resetShortcut(actionId);

  // Category order — single source for both the rendered list and the
  // first-conflict scan (so "Jump to conflict" lands on the row the user sees
  // first, not whatever order the issues map happens to enumerate in).
  const SHORTCUT_CATEGORIES = ["global", "app", "dashboard", "audioDrawer"] as const;

  // The first action (in visual order) that currently carries an issue. The
  // conflict banner is global, but the offending row can be scrolled off in a
  // sibling category — so anchor a jump on this id.
  const firstConflictActionId = $derived.by(() => {
    for (const category of SHORTCUT_CATEGORIES) {
      for (const action of shortcutCategoryActions(category)) {
        if (shortcutIssueFor(action.id)) return action.id;
      }
    }
    return null;
  });

  function jumpToConflict() {
    if (firstConflictActionId === null) return;
    const row = document.getElementById(`shortcut-row-${firstConflictActionId}`);
    if (!row) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    row.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "center" });
  }

  // ─── Window-capture keydown/pointerdown effect (verbatim from legacy) ─────────
  $effect(() => {
    const actionId = keyboard.shortcutCaptureActionId;
    if (actionId === null) return;
    const onKeydown = (e: KeyboardEvent) => keyboard.captureShortcut(actionId, e);
    const onPointerDown = (e: Event) => {
      const t = e.target;
      if (t instanceof Element && t.closest(`[data-shortcut-capture="${actionId}"]`)) return;
      keyboard.cancelShortcutCapture();
    };
    window.addEventListener("keydown", onKeydown, { capture: true });
    window.addEventListener("pointerdown", onPointerDown, { capture: true });
    return () => {
      window.removeEventListener("keydown", onKeydown, { capture: true });
      window.removeEventListener("pointerdown", onPointerDown, { capture: true });
    };
  });
</script>

<SettingGroup
  id="settings-section-shortcuts"
  title="Keyboard Shortcuts"
  hint="Click a shortcut to rebind it, then press the keys. Esc cancels, ⌫ clears. Changes save automatically."
>
  {#snippet actions()}
    <ReloadButton
      onclick={loadKeyboardBindingsSettings}
      disabled={savingKeyboardBindings}
      label="Reload shortcuts from saved settings"
    />
    <button
      class="settings-icon-btn"
      type="button"
      use:tip={"Restore all defaults"}
      aria-label="Restore all default shortcuts"
      onclick={restoreDefaultShortcuts}
      disabled={savingKeyboardBindings}
    >
      <IconRestoreAll aria-hidden="true" />
    </button>
  {/snippet}

  {#if keyboardBindingsSettings === null}
    <p class="loading-text">Loading shortcuts…</p>
  {:else}
    <SettingRow
      label="Global shortcuts"
      description="Use system-wide shortcuts for recording and showing Mnema while it is in the background. Background shortcuts require Command/Control or Alt; foreground shortcuts are ignored while typing in text fields."
    >
      {#snippet control()}
        <Switch bind:checked={keyboard.draftGlobalShortcutsEnabled} ariaLabel="Global shortcuts" />
      {/snippet}
    </SettingRow>

    {#if keyboardShortcutSaveBlocked && Object.keys(keyboardShortcutIssues).length > 0}
      <div class="shortcuts-error-row">
        <div class="inline-error" role="alert">
          <span class="inline-error__icon" aria-hidden="true"><IconAlert /></span>
          <span class="inline-error__msg">Resolve shortcut conflicts or invalid shortcuts before changes are saved.</span>
          {#if firstConflictActionId !== null}
            <button type="button" class="btn btn--ghost btn--sm shortcuts-jump" onclick={jumpToConflict}>
              Jump to conflict
            </button>
          {/if}
        </div>
      </div>
    {/if}
  {/if}
</SettingGroup>

{#if keyboardBindingsSettings !== null}
  {#each SHORTCUT_CATEGORIES as category (category)}
    <SettingGroup title={shortcutCategoryLabel(category)} bare nested>
      <div class="shortcut-editor-list">
        {#each shortcutCategoryActions(category) as action (action.id)}
          {@const binding = shortcutDraftBinding(action.id)}
          {@const issue = shortcutIssueFor(action.id)}
          {@const tokens = shortcutKeyTokens(binding)}
          {@const listening = shortcutCaptureActionId === action.id}
          <div id={`shortcut-row-${action.id}`} class="shortcut-editor-row" class:shortcut-editor-row--error={issue !== null} class:shortcut-editor-row--listening={listening}>
            <div class="shortcut-editor-row__main">
              <span class="shortcut-editor-row__title">{action.label}</span>
              <span class="shortcut-editor-row__description">{action.description}</span>
              {#if issue}
                <span class="shortcut-editor-row__error">{issue}</span>
              {/if}
            </div>
            <div class="shortcut-editor-row__controls">
              <button
                class="shortcut-capture"
                class:shortcut-capture--recording={listening}
                class:shortcut-capture--empty={!tokens && !listening}
                type="button"
                data-shortcut-capture={action.id}
                disabled={savingKeyboardBindings}
                aria-label={listening ? `Listening for ${action.label} shortcut` : `Set shortcut for ${action.label}`}
                onclick={(event) => { startShortcutCapture(action.id); event.currentTarget.focus(); }}
              >
                {#if listening}
                  <span class="shortcut-capture__pulse" aria-hidden="true"></span>
                  <span class="shortcut-capture__hint">Press keys…</span>
                {:else if tokens}
                  <span class="shortcut-capture__keys">
                    {#each tokens as token, i (i)}
                      <kbd class="shortcut-cap">{token}</kbd>
                    {/each}
                  </span>
                {:else}
                  <span class="shortcut-capture__hint">Set shortcut</span>
                {/if}
              </button>
              <button
                class="settings-icon-btn"
                type="button"
                use:tip={"Reset to default"}
                aria-label={`Reset ${action.label} to default`}
                disabled={savingKeyboardBindings}
                onclick={() => resetShortcut(action.id)}
              >
                <IconRestore aria-hidden="true" />
              </button>
              <button
                class="settings-icon-btn"
                type="button"
                use:tip={"Clear shortcut"}
                aria-label={`Clear ${action.label}`}
                disabled={savingKeyboardBindings || !binding}
                onclick={() => clearShortcut(action.id)}
              >
                <IconClear aria-hidden="true" />
              </button>
            </div>
          </div>
        {/each}
      </div>
    </SettingGroup>
  {/each}
{/if}

<style>
  /* The inline shortcut-conflict alert is a full-width banner, not a labeled
     row — break it out of the row grid so it spans the group. */
  .shortcuts-error-row {
    padding: 12px 0;
  }

  /* "Jump to conflict" sits at the trailing edge of the banner so the message
     keeps the lead and the action reads as the escape hatch. */
  .shortcuts-jump {
    margin-left: auto;
    flex-shrink: 0;
  }
</style>
