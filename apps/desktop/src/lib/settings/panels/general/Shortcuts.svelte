<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

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
    <button class="btn btn--ghost btn--sm" onclick={loadKeyboardBindingsSettings} disabled={savingKeyboardBindings}>
      Reload
    </button>
    <button class="btn btn--ghost btn--sm" onclick={restoreDefaultShortcuts} disabled={savingKeyboardBindings}>
      Restore defaults
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
        <Switch bind:checked={keyboard.draftGlobalShortcutsEnabled} />
      {/snippet}
    </SettingRow>

    {#if keyboardShortcutSaveBlocked && Object.keys(keyboardShortcutIssues).length > 0}
      <div class="shortcuts-error-row">
        <div class="inline-error" role="alert">
          <span class="inline-error__icon" aria-hidden="true">⚠</span>
          <span class="inline-error__msg">Resolve shortcut conflicts or invalid shortcuts before changes are saved.</span>
        </div>
      </div>
    {/if}
  {/if}
</SettingGroup>

{#if keyboardBindingsSettings !== null}
  {#each ["global", "app", "dashboard", "audioDrawer"] as category (category)}
    <SettingGroup title={shortcutCategoryLabel(category)}>
      <div class="shortcut-editor-list">
        {#each shortcutCategoryActions(category) as action (action.id)}
          {@const binding = shortcutDraftBinding(action.id)}
          {@const issue = shortcutIssueFor(action.id)}
          {@const tokens = shortcutKeyTokens(binding)}
          {@const listening = shortcutCaptureActionId === action.id}
          <div class="shortcut-editor-row" class:shortcut-editor-row--error={issue !== null} class:shortcut-editor-row--listening={listening}>
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
                class="shortcut-icon-btn"
                type="button"
                title="Reset to default"
                aria-label={`Reset ${action.label} to default`}
                onclick={() => resetShortcut(action.id)}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="M4 4v5h5" />
                  <path d="M4 9a8 8 0 1 1-1.5 5" />
                </svg>
              </button>
              <button
                class="shortcut-icon-btn"
                type="button"
                title="Clear shortcut"
                aria-label={`Clear ${action.label}`}
                disabled={!binding}
                onclick={() => clearShortcut(action.id)}
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path d="m6 6 12 12" />
                  <path d="m18 6-12 12" />
                </svg>
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
</style>
