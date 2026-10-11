<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import type { EditableShortcutAction } from "$lib/keyboard-bindings.svelte";

  const c = getSettingsController();
  const keyboard = c.keyboard;

  const keyboardBindingsSettings = $derived(keyboard.keyboardBindingsSettings);
  const keyboardShortcutIssues = $derived(keyboard.keyboardShortcutIssues);
  const keyboardShortcutSaveBlocked = $derived(keyboard.keyboardShortcutSaveBlocked);
  const shortcutCaptureActionId = $derived(keyboard.shortcutCaptureActionId);
  const savingKeyboardBindings = $derived(keyboard.savingKeyboardBindings);

  // Category order — single source for both the rendered list and the
  // first-conflict scan (so "Jump to conflict" lands on the row the user sees
  // first, not whatever order the issues map happens to enumerate in).
  const SHORTCUT_CATEGORIES = ["global", "app", "dashboard", "audioDrawer"] as const;

  // The first action (in visual order) that currently carries an issue. The
  // conflict notice is global, but the offending row can be scrolled off in a
  // sibling category — so anchor a jump on this id.
  const firstConflictActionId = $derived.by(() => {
    for (const category of SHORTCUT_CATEGORIES) {
      for (const action of keyboard.shortcutCategoryActions(category)) {
        if (keyboard.shortcutIssueFor(action.id)) return action.id;
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

  // While a row listens, every keydown goes to it (Esc cancels, Backspace/Delete
  // clears) and a pointerdown anywhere else cancels.
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

{#snippet chord(action: EditableShortcutAction, issue: string | null)}
  {@const listening = shortcutCaptureActionId === action.id}
  {@const tokens = keyboard.shortcutKeyTokens(keyboard.shortcutDraftBinding(action.id))}
  <button
    class="mx-btn kb-btn"
    class:kb-btn--issue={issue !== null}
    type="button"
    data-shortcut-capture={action.id}
    aria-pressed={listening}
    disabled={savingKeyboardBindings}
    aria-label={listening ? `Listening for ${action.label} shortcut` : `Set shortcut for ${action.label}`}
    onclick={(event) => { keyboard.startShortcutCapture(action.id); event.currentTarget.focus(); }}
  >
    {#if listening}
      <span class="mx-spin mx-spin--sm"></span>Press keys…
    {:else if tokens}
      <span class="kb-keys">{#each tokens as token, i (i)}<kbd>{token}</kbd>{/each}</span>
    {:else}
      Set shortcut
    {/if}
  </button>
{/snippet}

<SettingGroup
  id="settings-section-shortcuts"
  title="Keyboard Shortcuts"
  hint="Click a shortcut to rebind it, then press the keys. Esc cancels, ⌫ clears."
  loading={keyboardBindingsSettings === null}
>
  {#snippet actions()}
    <ReloadButton
      onclick={() => keyboard.loadKeyboardBindingsSettings()}
      disabled={savingKeyboardBindings}
      label="Reload shortcuts from saved settings"
    />
  {/snippet}

  {#if keyboardShortcutSaveBlocked && Object.keys(keyboardShortcutIssues).length > 0}
    <div class="mx-notice kb-notice" data-tone="danger" role="alert">
      <span class="mx-notice__icon"><IconAlert width="15" height="15" /></span>
      <b class="mx-notice__title">Shortcuts not saved</b>
      <p class="mx-notice__text">Resolve shortcut conflicts or invalid shortcuts before changes are saved.</p>
      {#if firstConflictActionId !== null}
        <div class="mx-notice__acts">
          <button type="button" class="mx-btn mx-btn--sm" onclick={jumpToConflict}>Jump to conflict</button>
        </div>
      {/if}
    </div>
  {/if}

  <SettingRow
    label="Global shortcuts"
    description="Work from any app while Mnema is in the background. Background shortcuts need Command/Control or Option."
  >
    {#snippet control()}
      <Switch bind:checked={keyboard.draftGlobalShortcutsEnabled} ariaLabel="Global shortcuts" />
    {/snippet}
  </SettingRow>

  {#each SHORTCUT_CATEGORIES as category (category)}
    <h3 class="kb-cat">{keyboard.shortcutCategoryLabel(category)}</h3>
    {#each keyboard.shortcutCategoryActions(category) as action (action.id)}
      {@const issue = keyboard.shortcutIssueFor(action.id)}
      {#if issue}
        <!-- The issue line sits under the copy, so the chord moves to `aside`. -->
        <SettingRow id={`shortcut-row-${action.id}`} label={action.label} description={action.description} full>
          {#snippet aside()}{@render chord(action, issue)}{/snippet}
          {#snippet control()}
            <span class="mx-inline" data-tone="danger"><IconAlert width="13" height="13" />{issue}</span>
          {/snippet}
        </SettingRow>
      {:else}
        <SettingRow id={`shortcut-row-${action.id}`} label={action.label} description={action.description}>
          {#snippet control()}{@render chord(action, null)}{/snippet}
        </SettingRow>
      {/if}
    {/each}
  {/each}

  <SettingRow label="Restore all default shortcuts" description="Asks first.">
    {#snippet control()}
      <button
        type="button"
        class="mx-btn"
        disabled={savingKeyboardBindings}
        onclick={() => keyboard.restoreDefaultShortcuts()}
      >
        Restore defaults
      </button>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .kb-notice {
    margin: var(--s-3) 0;
  }

  /* Quiet category sub-header between row runs. */
  .kb-cat {
    margin: 0;
    padding: var(--s-5) 0 var(--s-2);
    font: 500 var(--text-base)/1.3 var(--font-sans);
    color: var(--app-text-muted);
    border-bottom: 1px solid var(--mx-hairline);
  }

  .kb-btn--issue {
    --_bd: var(--app-danger-border);
  }

  /* Mockup `.st-keys`: the chord's caps inside the button. */
  .kb-keys {
    display: inline-flex;
    gap: 3px;
  }

  .kb-keys kbd {
    margin: 0;
    min-width: 20px;
    height: 20px;
    font-size: var(--text-sm);
  }
</style>
