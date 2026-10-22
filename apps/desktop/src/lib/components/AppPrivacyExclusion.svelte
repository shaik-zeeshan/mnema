<script lang="ts">
  import { tip } from "./tooltip";
  import IconPlus from "~icons/lucide/plus";
  import IconX from "~icons/lucide/x";
  import IconShield from "~icons/lucide/shield";
  import type { AppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";

  let {
    controller,
    comboboxListId = "privacy-app-combobox-list",
  }: {
    controller: AppPrivacyExclusionController;
    comboboxListId?: string;
  } = $props();

  // Roving keyboard highlight for the add-app listbox. The controller exposes
  // the filtered candidates and the add/close actions, but the highlighted
  // option is pure view state, so it lives here in the component.
  let highlightedIndex = $state(0);
  // Error state: the query matched nothing selectable when the user committed.
  let comboboxError = $state(false);
  let addButton = $state<HTMLButtonElement | null>(null);

  const enabledCount = $derived(controller.excludedApps.filter((app) => app.enabled).length);
  const optionId = (index: number) => `${comboboxListId}-option-${index}`;
  const activeDescendantId = $derived(
    controller.comboboxOpen &&
      controller.filteredCandidates.length > 0 &&
      highlightedIndex >= 0 &&
      highlightedIndex < controller.filteredCandidates.length
      ? optionId(highlightedIndex)
      : undefined,
  );

  // Keep the highlight in bounds as the filtered list changes (typing / open).
  $effect(() => {
    const count = controller.filteredCandidates.length;
    if (count === 0) {
      if (highlightedIndex !== 0) highlightedIndex = 0;
      return;
    }
    if (highlightedIndex > count - 1) highlightedIndex = count - 1;
    if (highlightedIndex < 0) highlightedIndex = 0;
  });

  const focus = (node: HTMLElement) => node.focus();

  function onComboboxInput() {
    comboboxError = false;
    highlightedIndex = 0;
    controller.handlePrivacyAppComboboxInput();
  }

  // Enter picks the HIGHLIGHTED option (not blindly filteredCandidates[0]) and
  // Arrow keys rove the highlight.
  function onComboboxKeydown(event: KeyboardEvent) {
    const candidates = controller.filteredCandidates;
    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        if (candidates.length > 0) highlightedIndex = (highlightedIndex + 1) % candidates.length;
        return;
      case "ArrowUp":
        event.preventDefault();
        if (candidates.length > 0) {
          highlightedIndex = (highlightedIndex - 1 + candidates.length) % candidates.length;
        }
        return;
      case "Enter": {
        event.preventDefault();
        const target = candidates[highlightedIndex];
        if (target) {
          comboboxError = false;
          controller.addPrivacyAppCandidate(target);
        } else {
          // Non-empty query that resolves to no selectable app → invalid input.
          comboboxError = controller.comboboxQuery.trim().length > 0;
        }
        return;
      }
      case "Escape":
        event.preventDefault();
        controller.comboboxOpen = false;
        addButton?.focus();
        return;
    }
  }
</script>

{#snippet appIcon(name: string, bundleId: string, src: string | null)}
  <span class="excl-app" aria-hidden="true">
    {#if src}<img src={src} alt="" loading="lazy" />{:else}{controller.appIconFallback(name, bundleId)}{/if}
  </span>
{/snippet}

<!-- "Add app…" + its popover: the installed-app search (a combobox over a
     listbox). Blur closes it (controller.closePrivacyAppComboboxSoon). -->
{#snippet addApp()}
  <div class="mx-pop-anchor">
    <button
      bind:this={addButton}
      type="button"
      class="mx-btn mx-btn--sm"
      aria-haspopup="listbox"
      aria-expanded={controller.comboboxOpen}
      disabled={controller.commandInFlight}
      onclick={() => (controller.comboboxOpen = true)}
    >
      <IconPlus width="14" height="14" />Add app…
    </button>
    {#if controller.comboboxOpen}
      <div class="mx-pop excl-pop" data-open role="dialog" aria-label="Exclude an app">
        <div class="mx-pop__head">
          <input
            class="mx-input excl-pop__input"
            use:focus
            role="combobox"
            aria-label="Search installed apps"
            aria-expanded="true"
            aria-controls={comboboxListId}
            aria-autocomplete="list"
            aria-activedescendant={activeDescendantId}
            aria-invalid={comboboxError}
            bind:value={controller.comboboxQuery}
            placeholder="Search installed apps"
            oninput={onComboboxInput}
            onblur={controller.closePrivacyAppComboboxSoon}
            onkeydown={onComboboxKeydown}
          />
        </div>
        <div class="excl-pop__list" id={comboboxListId} role="listbox">
          {#each controller.filteredCandidates as candidate, index (candidate.bundleId)}
            <button
              class="excl-opt"
              class:excl-opt--active={index === highlightedIndex}
              id={optionId(index)}
              type="button"
              role="option"
              aria-selected={index === highlightedIndex}
              onmousedown={(event) => event.preventDefault()}
              onmouseenter={() => (highlightedIndex = index)}
              onclick={() => controller.addPrivacyAppCandidate(candidate)}
            >
              {@render appIcon(candidate.displayName, candidate.bundleId, controller.privacyAppIconSrc(candidate))}
              <span class="excl-opt__main">
                <span class="excl-name">{candidate.displayName}</span>
                <span class="excl-meta">{candidate.bundleId}</span>
              </span>
              {#if candidate.running}<span class="mx-chip">running</span>{/if}
            </button>
          {:else}
            <p class="excl-pop__empty">No matching installed apps</p>
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/snippet}

<div class="excl-list">
  {#each controller.excludedApps as app (app.id)}
    <div class="excl-item" class:excl-item--off={!app.enabled}>
      {@render appIcon(app.displayName, app.bundleId, controller.appIconSrcForBundleId(app.bundleId))}
      <span class="excl-name">{app.displayName}</span>
      <span class="excl-meta">{app.bundleId}</span>
      <!-- Pause/resume one exclusion without forgetting it. -->
      <input
        type="checkbox"
        role="switch"
        class="mx-switch excl-item__pause"
        checked={app.enabled}
        disabled={controller.commandInFlight}
        aria-label={`Exclude ${app.displayName}`}
        use:tip={app.enabled ? "Excluded · switch off to pause" : "Paused · switch on to exclude"}
        onchange={(e) => controller.setPrivacyExcludedAppEnabled(app.id, e.currentTarget.checked)}
      />
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
        aria-label={`Remove ${app.displayName}`}
        use:tip={"Remove exclusion"}
        disabled={controller.commandInFlight}
        onclick={() => controller.removePrivacyApp(app.id)}
      >
        <IconX width="15" height="15" />
      </button>
    </div>
  {:else}
    <div class="mx-empty mx-empty--compact">
      <span class="mx-empty__glyph"><IconShield width="14" height="14" /></span>
      <b class="mx-empty__title">No app exclusions</b>
      <p class="mx-empty__text">Every app’s visible content is recorded. Add the ones that should never be.</p>
      <div class="mx-empty__acts">{@render addApp()}</div>
    </div>
  {/each}
  {#if controller.excludedApps.length > 0}
    <div class="excl-foot">
      {@render addApp()}
      <span class="mx-label"><span class="num">{enabledCount}</span> excluded</span>
    </div>
  {/if}
</div>

<style>
  /* Mockup `.st-list`. No overflow:hidden — the add-app popover overflows it. */
  .excl-list { border: 1px solid var(--app-border); border-radius: var(--r-md); background: var(--app-surface); }
  .excl-item { display: flex; align-items: center; gap: var(--s-3); padding: 9px var(--s-3); border-top: 1px solid var(--mx-hairline); }
  .excl-item:first-child { border-top: 0; }
  .excl-item--off :is(.excl-app, .excl-name, .excl-meta) { opacity: 0.5; }
  .excl-item__pause { margin-left: auto; }
  .excl-app { flex: none; display: grid; place-items: center; width: 22px; height: 22px; border-radius: 6px; overflow: hidden; font: 600 var(--text-sm)/1 var(--font-sans); color: var(--app-text-strong); background: var(--app-surface-raised); border: 1px solid var(--app-border); }
  .excl-app img { width: 100%; height: 100%; object-fit: contain; }
  .excl-app:has(img) { border: 0; background: none; }
  .excl-name { font: 500 var(--text-md)/1.2 var(--font-sans); color: var(--app-text-strong); }
  .excl-meta { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: 400 var(--text-sm)/1 var(--font-mono); color: var(--app-text-subtle); }
  .excl-foot { display: flex; align-items: center; gap: var(--s-2); padding: 8px var(--s-3); border-top: 1px solid var(--mx-hairline); border-radius: 0 0 var(--r-md) var(--r-md); background: var(--app-surface-subtle); }
  .excl-foot .mx-label { margin-left: auto; }
  /* The trigger sits on the left, so the popover opens rightward. */
  .excl-pop { left: 0; right: auto; width: 340px; transform-origin: top left; }
  .excl-pop__input { width: 100%; }
  .excl-pop__list { max-height: 260px; overflow-y: auto; padding: 0 4px 4px; }
  .excl-opt { display: flex; align-items: center; gap: var(--s-3); width: 100%; padding: 7px 10px; border: 0; border-radius: var(--r-sm); background: none; text-align: left; cursor: pointer; }
  .excl-opt--active { background: var(--mx-wash-strong); }
  .excl-opt__main { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex: 1 1 auto; }
  .excl-pop__empty { margin: 0; padding: var(--s-2) 10px var(--s-3); font-size: var(--text-base); color: var(--app-text-subtle); }
</style>
