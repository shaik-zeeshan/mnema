<script lang="ts">
  // ⌘F settings row filter (DECISIONS.md G7) — direction 04 puts it at the
  // right end of the tab strip, because in this direction ⌘F *is* the
  // navigation: typing turns the pane into one ranked list of matching rows,
  // each with its section breadcrumb and its real, live control.
  //
  // Two states in one element (the mockup's `.ffield`): a chip advertising the
  // key while closed, the actual input while open. Mounted for the whole
  // /settings route, so the ⌘F listener is route-scoped and never shadows
  // search on other surfaces.

  import IconSearch from "~icons/lucide/search";
  import IconClear from "~icons/lucide/x";
  import { settingsFind } from "$lib/settings/state/settings-find.svelte";
  import { getSettingsController } from "../state/controller.svelte";

  interface Props {
    /** Matching rows in the index — rendered as the field's trailing count. */
    hits?: number;
  }

  let { hits = 0 }: Props = $props();

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

  // Focus on open (the input only exists once opened).
  $effect(() => {
    if (settingsFind.open) input?.focus();
  });
</script>

{#if settingsFind.open}
  <div class="ffield ffield--on">
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
    {#if settingsFind.query.trim()}
      <span class="ffield__count" aria-live="polite">{hits}</span>
    {/if}
    <button
      class="ffield__clear"
      type="button"
      aria-label="Clear filter"
      onclick={() => settingsFind.close()}
    >
      <IconClear aria-hidden="true" />
    </button>
  </div>
{:else}
  <button class="ffield" type="button" onclick={() => (settingsFind.open = true)}>
    <IconSearch aria-hidden="true" />
    <span class="ffield__ph">Filter settings</span>
    <span class="kbd" aria-hidden="true">⌘F</span>
  </button>
{/if}
