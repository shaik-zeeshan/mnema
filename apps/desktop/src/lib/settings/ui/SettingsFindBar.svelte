<script lang="ts">
  // ⌘F settings row filter (DECISIONS.md G7), rehomed into the tool strip.
  //
  // Direction 02 deletes the settings rail: THE FILTER IS THE NAVIGATION, so it
  // is a permanent 22px field in the 30px tool strip rather than a panel that
  // ⌘F reveals. Only the surface moved — the index, the matcher and the
  // hide-don't-unmount behaviour are phase-1 machinery and untouched. Typing
  // here still filters the whole settings surface down to matching ROWS, each
  // rendered in place with its breadcrumb and its real, live control.
  //
  // ⌘F now focuses + selects the field (there is nothing to open), and Escape
  // clears the query rather than closing a strip that has no closed state.

  import IconSearch from "~icons/lucide/search";
  import IconClear from "~icons/lucide/x";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
  import { getSettingsController } from "../state/controller.svelte";

  const c = getSettingsController();

  let input = $state<HTMLInputElement | null>(null);

  // The field is always present, so `open` — which `active` gates on — is a
  // property of the route being mounted, not of a disclosure. Clear it on the
  // way out so a stale query can never survive into the next visit.
  $effect(() => {
    settingsFind.open = true;
    return () => settingsFind.close();
  });

  // The keybinding rows capture keys on a window CAPTURE-phase listener and
  // `stopImmediatePropagation()` everything while a rebind is listening, so a
  // bubble-phase listener here already cannot fire mid-capture. The explicit
  // check keeps that true even if the rebind UI ever stops swallowing keys.
  function onKeydown(event: KeyboardEvent) {
    if (c.keyboard.shortcutCaptureActionId !== null) return;
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
      event.preventDefault();
      input?.focus();
      input?.select();
    }
  }

  $effect(() => {
    window.addEventListener("keydown", onKeydown);
    return () => window.removeEventListener("keydown", onKeydown);
  });

  function onFieldKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    if (settingsFind.query) {
      settingsFind.query = "";
      return;
    }
    input?.blur();
  }
</script>

<div class="find-field input" class:is-focused={settingsFind.query !== ""}>
  <IconSearch aria-hidden="true" />
  <input
    bind:this={input}
    bind:value={settingsFind.query}
    type="text"
    placeholder="Filter settings…"
    aria-label="Filter settings"
    spellcheck="false"
    autocomplete="off"
    onkeydown={onFieldKeydown}
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
  {:else}
    <kbd class="kbd find-field__kbd">⌘F</kbd>
  {/if}
</div>

<style>
  /* A 22px tool-strip control: the phase-1 `.input` primitive supplies the
     frame; this only sets the strip's height/width and lays out the glyph,
     field and trailing affordance inside it. */
  .find-field {
    flex: 0 0 auto;
    /* `nowrap` is load-bearing: the shared `.input` primitive wraps, and a
       long query pushed the clear button onto a second line below the strip. */
    flex-wrap: nowrap;
    width: 260px;
    height: 22px;
    gap: 6px;
    padding: 0 4px 0 7px;
    overflow: hidden;
  }

  .find-field :global(svg) {
    width: 12px;
    height: 12px;
    flex: 0 0 auto;
    fill: none;
    stroke: var(--app-text-subtle);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    pointer-events: none;
  }

  .find-field input {
    flex: 1 1 auto;
    min-width: 0;
    height: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: var(--t-meta);
    outline: none;
  }

  .find-field input::placeholder {
    color: var(--app-text-subtle);
  }

  .find-field__kbd {
    flex: 0 0 auto;
    height: 16px;
    min-width: 24px;
    pointer-events: none;
  }

  .find-field__clear {
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
    cursor: pointer;
    flex: 0 0 auto;
  }

  .find-field__clear:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }

  .find-field__clear :global(svg) {
    width: 10px;
    height: 10px;
    stroke: currentColor;
    pointer-events: none;
  }
</style>
