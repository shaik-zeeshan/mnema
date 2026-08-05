<script lang="ts">
  // ⌘F settings row filter (DECISIONS.md G7).
  //
  // Direction 05 renders this as a PERSISTENT scoped field in the settings
  // toolbar rather than a strip that appears on ⌘F — the mockup's toolbar
  // carries "Search settings" at rest, and a field you can see is a better
  // affordance than a shortcut you have to know. ⌘F still focuses and selects
  // it; Escape still clears and closes.
  //
  // Filtering itself is unchanged: a STATE over the content pane, not a nav.
  // Typing filters the whole settings surface down to matching ROWS, each
  // rendered in place with its breadcrumb and its real, live control (autosave
  // + row echo unchanged).

  import IconSearch from "~icons/lucide/search";
  import IconClear from "~icons/lucide/x";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
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
      input?.focus();
      input?.select();
      return;
    }
    if (event.key === "Escape" && settingsFind.open) {
      event.preventDefault();
      settingsFind.close();
      input?.blur();
    }
  }

  $effect(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });
</script>

<div class="ti-setsearch" class:is-focus={settingsFind.open}>
  <IconSearch aria-hidden="true" />
  <input
    bind:this={input}
    bind:value={settingsFind.query}
    type="text"
    placeholder="Search settings"
    aria-label="Search settings"
    spellcheck="false"
    autocomplete="off"
    onfocus={() => (settingsFind.open = true)}
    oninput={() => (settingsFind.open = true)}
  />
  {#if settingsFind.query}
    <button
      class="ti-setsearch__clear"
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

<style>
  /* A stock native search field — quiet by design. It is not an instrument:
     nothing about it has a physical consequence. */
  .ti-setsearch {
    position: relative;
    display: flex;
    align-items: center;
    width: 176px;
    height: var(--h-md);
    margin-left: auto;
    padding: 0 var(--pad-control);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
  }

  .ti-setsearch.is-focus {
    box-shadow:
      inset 0 0 0 var(--hairline) var(--app-accent-border),
      var(--ring);
  }

  .ti-setsearch :global(svg) {
    width: 13px;
    height: 13px;
    flex: 0 0 auto;
    fill: none;
    stroke: var(--app-text-subtle);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
    pointer-events: none;
  }

  .ti-setsearch input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    padding: 0 var(--s-6);
    border: 0;
    background: none;
    color: var(--app-text-strong);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    outline: none;
  }

  .ti-setsearch input::placeholder {
    color: var(--app-text-subtle);
  }

  .ti-setsearch__clear {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: none;
    color: var(--app-text-muted);
    cursor: default;
    flex: 0 0 auto;
  }

  .ti-setsearch__clear:hover {
    color: var(--app-text-strong);
  }

  .ti-setsearch__clear :global(svg) {
    width: 11px;
    height: 11px;
    stroke: currentColor;
  }
</style>
