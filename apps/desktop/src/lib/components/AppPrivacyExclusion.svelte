<script lang="ts">
  import { tip } from "./tooltip";
  import type { AppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";

  let {
    controller,
    comboboxListId = "privacy-app-combobox-list",
  }: {
    controller: AppPrivacyExclusionController;
    comboboxListId?: string;
  } = $props();

  // Roving keyboard highlight for the combobox listbox. The controller exposes
  // the filtered candidates and the add/close actions, but the highlighted
  // option is pure view state, so it lives here in the component.
  let highlightedIndex = $state(0);
  // Error state: the query matched nothing selectable when the user committed.
  let comboboxError = $state(false);

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

  function onComboboxInput() {
    comboboxError = false;
    highlightedIndex = 0;
    controller.handlePrivacyAppComboboxInput();
  }

  // Component-owned keyboard handling so Enter picks the HIGHLIGHTED option
  // (not blindly filteredCandidates[0]) and Arrow keys rove the highlight.
  function onComboboxKeydown(event: KeyboardEvent) {
    const candidates = controller.filteredCandidates;
    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        if (!controller.comboboxOpen) {
          controller.comboboxOpen = true;
          highlightedIndex = 0;
          return;
        }
        if (candidates.length > 0) {
          highlightedIndex = (highlightedIndex + 1) % candidates.length;
        }
        return;
      }
      case "ArrowUp": {
        event.preventDefault();
        if (!controller.comboboxOpen) {
          controller.comboboxOpen = true;
          highlightedIndex = Math.max(candidates.length - 1, 0);
          return;
        }
        if (candidates.length > 0) {
          highlightedIndex = (highlightedIndex - 1 + candidates.length) % candidates.length;
        }
        return;
      }
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
      case "Escape": {
        controller.comboboxOpen = false;
        return;
      }
    }
  }

  function onOptionHover(index: number) {
    highlightedIndex = index;
  }
</script>

<div class="app-privacy-exclusion">
  <div class="privacy-disclosure">
    <p>Browsers are recorded unless the browser app is excluded.</p>
    <p>Private/incognito browser windows are recorded unless the browser app is excluded.</p>
    <p>Mnema does not detect browser password pages or password fields.</p>
    <p>Browser extensions and websites are not excluded separately.</p>
  </div>

  {#if controller.pendingRecommendedApps.length > 0}
    <div class="recommendation-section">
      <span class="group-label">Recommended App Exclusions</span>
      <div class="recommendation-section__list">
        {#each controller.pendingRecommendedApps as app (app.bundleId)}
          {@const iconSrc = controller.appIconSrcForBundleId(app.bundleId)}
          <div class="settings-list-item settings-list-item--app-rule">
            <span class="app-rule-icon" aria-hidden="true">
              {#if iconSrc}
                <img src={iconSrc} alt="" loading="lazy" />
              {:else}
                <span>{controller.appIconFallback(app.displayName, app.bundleId)}</span>
              {/if}
            </span>
            <div class="settings-list-item__main">
              <span class="settings-list-item__title">{app.displayName}</span>
              <span class="settings-list-item__description">{app.categoryLabel} · {app.bundleId}</span>
            </div>
            <button
              class="btn btn--ghost btn--sm"
              type="button"
              disabled={controller.commandInFlight}
              onclick={() => controller.applyRecommendation(app)}
            >
              {controller.recommendationActionLabel(app.exclusionState)}
            </button>
          </div>
        {/each}
      </div>
    </div>
  {/if}

  {#if controller.visibleBrowserDisclosureApps.length > 0}
    <div class="recommendation-section">
      <span class="group-label">Known Browsers</span>
      <div class="recommendation-section__list">
        {#each controller.visibleBrowserDisclosureApps as app (app.bundleId)}
          {@const iconSrc = controller.appIconSrcForBundleId(app.bundleId)}
          <div class="settings-list-item settings-list-item--app-rule">
            <span class="app-rule-icon" aria-hidden="true">
              {#if iconSrc}
                <img src={iconSrc} alt="" loading="lazy" />
              {:else}
                <span>{controller.appIconFallback(app.displayName, app.bundleId)}</span>
              {/if}
            </span>
            <div class="settings-list-item__main">
              <span class="settings-list-item__title">{app.displayName}</span>
              <span class="settings-list-item__description">Browser screen content is recorded unless this app is excluded.</span>
            </div>
            {#if app.exclusionState === "enabled"}
              <span class="badge" data-tone="ok">Excluded</span>
            {:else}
              <button
                class="btn btn--ghost btn--sm"
                type="button"
                disabled={controller.commandInFlight}
                onclick={() => controller.applyRecommendation(app)}
              >
                {controller.recommendationActionLabel(app.exclusionState)}
              </button>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {/if}

  <div class="app-combobox">
    <input
      class="text-input app-combobox__input"
      class:is-error={comboboxError}
      role="combobox"
      aria-expanded={controller.comboboxOpen}
      aria-controls={comboboxListId}
      aria-autocomplete="list"
      aria-activedescendant={activeDescendantId}
      aria-invalid={comboboxError}
      bind:value={controller.comboboxQuery}
      placeholder="Search installed apps"
      oninput={onComboboxInput}
      onfocus={() => { controller.comboboxOpen = true; }}
      onblur={controller.closePrivacyAppComboboxSoon}
      onkeydown={onComboboxKeydown}
      disabled={controller.commandInFlight}
    />
    {#if controller.comboboxOpen}
      <div class="app-combobox__panel" id={comboboxListId} role="listbox">
        {#if controller.filteredCandidates.length > 0}
          {#each controller.filteredCandidates as candidate, index (candidate.bundleId)}
            {@const iconSrc = controller.privacyAppIconSrc(candidate)}
            <button
              class="app-combobox__option"
              class:option--active={index === highlightedIndex}
              id={optionId(index)}
              type="button"
              role="option"
              aria-selected={index === highlightedIndex}
              onmousedown={(event) => event.preventDefault()}
              onmouseenter={() => onOptionHover(index)}
              onclick={() => controller.addPrivacyAppCandidate(candidate)}
            >
              <span class="app-combobox__option-content">
                <span class="app-combobox__icon" aria-hidden="true">
                  {#if iconSrc}
                    <img src={iconSrc} alt="" loading="lazy" />
                  {:else}
                    <span>{controller.appIconFallback(candidate.displayName, candidate.bundleId)}</span>
                  {/if}
                </span>
                <span class="app-combobox__option-main">
                  <span class="app-combobox__name">{candidate.displayName}</span>
                  <span class="app-combobox__bundle">{candidate.bundleId}</span>
                </span>
              </span>
              {#if candidate.running}
                <span class="badge" data-tone="ok">Running</span>
              {/if}
            </button>
          {/each}
        {:else}
          <span class="app-combobox__empty">No matching installed apps</span>
        {/if}
      </div>
    {/if}
  </div>
  <p class="hint">Press Enter or choose a result to exclude it. Running apps are marked when they match an installed app bundle.</p>

  <div class="settings-list">
    {#if controller.excludedApps.length > 0}
      <div class="exclusion-chips">
        {#each controller.excludedApps as app (app.id)}
          {@const iconSrc = controller.appIconSrcForBundleId(app.bundleId)}
          <div class="exclusion-chip" class:exclusion-chip--off={!app.enabled}>
            <button
              type="button"
              class="exclusion-chip__toggle"
              aria-pressed={app.enabled}
              aria-label={`${app.displayName} — ${app.enabled ? "exclusion active, activate to disable" : "exclusion disabled, activate to enable"}`}
              use:tip={`${app.displayName}\n${app.bundleId}\n${app.enabled ? "Excluded · click to disable" : "Disabled · click to enable"}`}
              disabled={controller.commandInFlight}
              onclick={() => controller.setPrivacyExcludedAppEnabled(app.id, !app.enabled)}
            >
              <span class="exclusion-chip__icon" aria-hidden="true">
                {#if iconSrc}
                  <img src={iconSrc} alt="" loading="lazy" />
                {:else}
                  <span>{controller.appIconFallback(app.displayName, app.bundleId)}</span>
                {/if}
              </span>
              <span class="exclusion-chip__dot" aria-hidden="true"></span>
            </button>
            <button
              type="button"
              class="exclusion-chip__remove"
              aria-label={`Remove ${app.displayName} from exclusions`}
              use:tip={"Remove exclusion"}
              disabled={controller.commandInFlight}
              onclick={() => controller.removePrivacyApp(app.id)}
            >
              ×
            </button>
          </div>
        {/each}
      </div>
    {:else}
      <p class="empty-state">No app exclusions.</p>
    {/if}
  </div>
  <p class="hint">Click an icon to pause or resume its exclusion; hover an icon to remove it. Mnema records visible content from non-excluded apps, including private/incognito browser windows.</p>
</div>

<style>
  .app-privacy-exclusion {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 12px;
  }

  .privacy-disclosure {
    display: grid;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface-subtle);
  }

  .privacy-disclosure p,
  .hint {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
  }

  .recommendation-section {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .recommendation-section__list,
  .settings-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .group-label {
    display: block;
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .settings-list-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
    padding: 8px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
  }

  .settings-list-item--app-rule {
    grid-template-columns: 28px minmax(0, 1fr) auto;
  }

  .settings-list-item__main {
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .settings-list-item__title {
    overflow: hidden;
    color: var(--app-text);
    font-size: 12px;
    font-weight: 700;
    line-height: 1.3;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .settings-list-item__description {
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  .app-rule-icon,
  .app-combobox__icon {
    display: grid;
    width: 28px;
    height: 28px;
    flex: 0 0 28px;
    place-items: center;
    overflow: hidden;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-size: 11px;
    font-weight: 800;
    line-height: 1;
  }

  .app-rule-icon img,
  .app-combobox__icon img {
    width: 22px;
    height: 22px;
    object-fit: contain;
  }

  /* Excluded apps render as compact icon chips. The toggle button flips the
     exclusion between active and disabled in place; a hover/focus × removes it. */
  .exclusion-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }

  .exclusion-chip {
    position: relative;
  }

  .exclusion-chip__toggle {
    position: relative;
    display: grid;
    width: 42px;
    height: 42px;
    place-items: center;
    padding: 0;
    border: 1px solid var(--app-accent-border);
    border-radius: 9px;
    background: var(--app-accent-bg);
    cursor: pointer;
    transition: border-color 0.12s, background 0.12s, opacity 0.12s, transform 0.12s;
  }

  .exclusion-chip__toggle:not(:disabled):hover {
    border-color: var(--app-accent);
    transform: translateY(-1px);
  }

  .exclusion-chip__toggle:focus-visible {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .exclusion-chip__toggle:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .exclusion-chip__icon {
    display: grid;
    width: 28px;
    height: 28px;
    place-items: center;
    overflow: hidden;
    color: var(--app-text);
    font-size: 11px;
    font-weight: 800;
    line-height: 1;
  }

  .exclusion-chip__icon img {
    width: 26px;
    height: 26px;
    object-fit: contain;
  }

  /* Status dot: accent when the exclusion is active, hollow + muted when off. */
  .exclusion-chip__dot {
    position: absolute;
    right: -3px;
    bottom: -3px;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    border: 2px solid var(--app-surface);
    background: var(--app-accent);
  }

  .exclusion-chip--off .exclusion-chip__toggle {
    border-style: dashed;
    border-color: var(--app-border-strong);
    background: var(--app-surface-subtle);
  }

  .exclusion-chip--off .exclusion-chip__icon {
    color: var(--app-text-faint);
  }

  .exclusion-chip--off .exclusion-chip__icon img {
    filter: grayscale(1);
    opacity: 0.45;
  }

  .exclusion-chip--off .exclusion-chip__dot {
    background: var(--app-surface);
    border-color: var(--app-border-strong);
  }

  .exclusion-chip__remove {
    position: absolute;
    top: -7px;
    right: -7px;
    display: grid;
    width: 17px;
    height: 17px;
    place-items: center;
    padding: 0;
    border: 1px solid var(--app-border-strong);
    border-radius: 50%;
    background: var(--app-surface-raised);
    color: var(--app-text-muted);
    font-size: 12px;
    line-height: 1;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.12s, color 0.12s, border-color 0.12s;
  }

  .exclusion-chip:hover .exclusion-chip__remove,
  .exclusion-chip:focus-within .exclusion-chip__remove {
    opacity: 1;
  }

  .exclusion-chip__remove:not(:disabled):hover {
    color: var(--app-danger);
    border-color: var(--app-danger-border);
  }

  .exclusion-chip__remove:focus-visible {
    opacity: 1;
    outline: none;
    box-shadow: 0 0 0 2px var(--app-accent-glow);
  }

  .exclusion-chip__remove:disabled {
    cursor: not-allowed;
  }

  .app-combobox {
    position: relative;
    min-width: 0;
  }

  .app-combobox__input {
    width: 100%;
  }

  .app-combobox__panel {
    position: absolute;
    z-index: 40;
    top: calc(100% + 4px);
    right: 0;
    left: 0;
    display: flex;
    max-height: 240px;
    flex-direction: column;
    gap: 2px;
    overflow-y: auto;
    padding: 4px;
    border: 1px solid var(--app-border-strong);
    border-radius: 6px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover);
  }

  .app-combobox__option {
    display: flex;
    width: 100%;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 7px 9px;
    border: 1px solid transparent;
    border-radius: 4px;
    background: transparent;
    color: var(--app-text);
    font-family: inherit;
    text-align: left;
    cursor: pointer;
  }

  .app-combobox__option:hover {
    border-color: var(--app-border-hover);
    background: var(--app-surface-hover);
  }

  /* Roving keyboard highlight: the active option reads as the Enter target. */
  .app-combobox__option.option--active {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-text-strong);
  }

  .app-combobox__option-content {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 9px;
  }

  .app-combobox__option-main {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .app-combobox__name {
    overflow: hidden;
    color: var(--app-text-strong);
    font-size: 12px;
    font-weight: 700;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .app-combobox__bundle {
    overflow: hidden;
    color: var(--app-text-faint);
    font-size: 10px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .app-combobox__empty,
  .empty-state {
    margin: 0;
    padding: 10px;
    color: var(--app-text-faint);
    font-size: 11px;
    font-style: italic;
  }

  .text-input {
    flex: 1;
    padding: 7px 10px;
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 4px;
    font-family: inherit;
    font-size: 12px;
    color: var(--app-text);
    outline: none;
    transition: border-color 0.12s;
  }

  .text-input:focus {
    border-color: var(--app-accent);
    box-shadow: var(--app-ring);
  }

  /* Invalid state: Enter committed a query that matches no selectable app. */
  .text-input.is-error,
  .text-input[aria-invalid="true"] {
    border-color: var(--app-danger-border);
  }

  .text-input.is-error:focus,
  .text-input[aria-invalid="true"]:focus {
    border-color: var(--app-danger);
    box-shadow: var(--app-ring-danger);
  }

  .text-input::placeholder {
    color: var(--app-text-faint);
  }

  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 8px 16px;
    border-radius: 4px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    cursor: pointer;
    border: 1px solid transparent;
    transition: background 0.12s, border-color 0.12s, opacity 0.12s;
    outline: none;
  }

  .btn:disabled {
    opacity: var(--app-disabled-opacity);
    cursor: not-allowed;
  }

  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  .btn--ghost {
    background: transparent;
    color: var(--app-text-muted);
    border-color: var(--app-border-strong);
    font-size: 10px;
  }

  .btn--ghost:not(:disabled):hover {
    background: var(--app-surface-hover);
    color: var(--app-text);
    border-color: var(--app-border-hover);
  }

  .btn--sm {
    padding: 3px 8px;
    font-size: 9px;
  }

  .badge {
    align-self: flex-start;
    margin-top: 2px;
    padding: 1px 6px;
    border-radius: 3px;
    border: 1px solid var(--app-border);
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .badge[data-tone="ok"] {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
</style>
