<script lang="ts">
  // ⌘F settings row filter (DECISIONS.md G7).
  //
  // Nav-agnostic by design: this is a strip over the CONTENT pane, not a nav —
  // the settings navigation shape is a per-direction phase-2 decision, and the
  // rail's own search filters the nav, not the rows. Typing here filters the
  // whole settings surface down to matching ROWS, each rendered in place with
  // its breadcrumb and its real, live control (autosave + row echo unchanged).
  //
  // Mounted for the whole /settings route (it renders nothing while closed), so
  // the ⌘F listener is route-scoped and never shadows search on other surfaces.

  import IconSearch from "~icons/lucide/search";
  import IconClear from "~icons/lucide/x";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
  import {
    SETTINGS_ROW_COUNT,
    matchingRowCount,
  } from "$lib/settings/settings-index";
  import { getSettingsController } from "../state/controller.svelte";

  const c = getSettingsController();

  let input = $state<HTMLInputElement | null>(null);

  // The keybinding rows capture keys on a window CAPTURE-phase listener and
  // `stopImmediatePropagation()` everything while a rebind is listening, so a
  // bubble-phase listener here already cannot fire mid-capture. The explicit
  // check keeps that true even if the rebind UI ever stops swallowing keys.
  function onKeydown(event: KeyboardEvent) {
    if (c.keyboard.shortcutCaptureActionId !== null) return;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      settingsFind.open = true;
      // Re-pressing ⌘F while open re-focuses + selects instead of closing.
      queueMicrotask(() => input?.select());
      return;
    }
    if (event.key === "Escape" && settingsFind.open) {
      event.preventDefault();
      settingsFind.close();
    }
  }

  $effect(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });


  // Focus on open (the field only exists once opened).
  $effect(() => {
    if (settingsFind.open) input?.focus();
  });
</script>

{#if settingsFind.open}
  <div class="find-strip">
    <div class="find-field">
      <IconSearch aria-hidden="true" />
      <input
        bind:this={input}
        bind:value={settingsFind.query}
        type="text"
        placeholder="Filter settings…"
        aria-label="Filter settings"
        spellcheck="false"
        autocomplete="off"
      />
      {#if settingsFind.query}
        <button
          class="find-field__clear"
          type="button"
          aria-label="Clear filter"
          onclick={() => {
            settingsFind.query = "";
            input?.focus();
          }}
        >
          <IconClear aria-hidden="true" />
        </button>
      {/if}
    </div>
    <!-- The denominator is counted off the row INDEX, not the DOM: it is a
         statement about the settings Mnema has, and stays true whether or not a
         conditional row happens to be mounted right now. "Filtering every
         section" said nothing; this says how much is left. -->
    <span class="find-count">
      {#if settingsFind.query}
        {matchingRowCount(settingsFind.query)} of {SETTINGS_ROW_COUNT} settings
      {:else}
        {SETTINGS_ROW_COUNT} settings
      {/if}
    </span>
    <button class="find-close" type="button" onclick={() => settingsFind.close()}>
      <kbd class="kbd">esc</kbd>
    </button>
  </div>
{/if}

<style>
  .find-strip {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .find-field {
    position: relative;
    display: flex;
    align-items: center;
    flex: 1 1 auto;
    min-width: 0;
    max-width: 420px;
  }

  .find-field :global(svg) {
    position: absolute;
    left: 9px;
    width: 13px;
    height: 13px;
    fill: none;
    stroke: var(--app-text-muted);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    pointer-events: none;
  }

  .find-field input {
    width: 100%;
    height: 28px;
    padding: 0 28px 0 28px;
    border: 1px solid var(--app-border);
    border-radius: var(--r-pill);
    background: var(--app-surface);
    color: var(--app-text);
    font-family: inherit;
    font-size: var(--t-ui);
    outline: none;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .find-field input:focus {
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  .find-field__clear {
    position: absolute;
    right: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--app-text-muted);
    cursor: pointer;
  }

  .find-field__clear:hover {
    color: var(--app-text-strong);
  }

  .find-field__clear :global(svg) {
    position: static;
    width: 11px;
    height: 11px;
    stroke: currentColor;
  }

  .find-count {
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    white-space: nowrap;
  }

  /* Direction 04 skin: the close control IS its key. The button is only the hit
     target — the shared `.kbd` primitive draws the cap, so this reads the same
     as every other advertised key in the app instead of as a pill that happens
     to say "Esc". */
  .find-close {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    padding: 0;
    border: 0;
    background: transparent;
    cursor: pointer;
  }

  .find-close :global(.kbd) {
    pointer-events: none;
  }

  .find-close:hover :global(.kbd) {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }

  .find-close:focus-visible {
    outline: none;
    border-radius: var(--r-sm);
    box-shadow: var(--app-ring);
  }
</style>
