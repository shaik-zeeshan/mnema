<script lang="ts">
  import Switch from "$lib/components/Switch.svelte";
  import type { AppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";

  let {
    controller,
    comboboxListId = "privacy-app-combobox-list",
  }: {
    controller: AppPrivacyExclusionController;
    comboboxListId?: string;
  } = $props();
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
      role="combobox"
      aria-expanded={controller.comboboxOpen}
      aria-controls={comboboxListId}
      aria-autocomplete="list"
      bind:value={controller.comboboxQuery}
      placeholder="Search installed apps"
      oninput={controller.handlePrivacyAppComboboxInput}
      onfocus={() => { controller.comboboxOpen = true; }}
      onblur={controller.closePrivacyAppComboboxSoon}
      onkeydown={controller.handlePrivacyAppComboboxKeydown}
      disabled={controller.commandInFlight}
    />
    {#if controller.comboboxOpen}
      <div class="app-combobox__panel" id={comboboxListId} role="listbox">
        {#if controller.filteredCandidates.length > 0}
          {#each controller.filteredCandidates as candidate (candidate.bundleId)}
            {@const iconSrc = controller.privacyAppIconSrc(candidate)}
            <button
              class="app-combobox__option"
              type="button"
              role="option"
              aria-selected="false"
              onmousedown={(event) => event.preventDefault()}
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
      {#each controller.excludedApps as app (app.id)}
        {@const iconSrc = controller.appIconSrcForBundleId(app.bundleId)}
        <div class="settings-list-item settings-list-item--app-rule">
          <span class="app-rule-icon" aria-hidden="true">
            {#if iconSrc}
              <img src={iconSrc} alt="" loading="lazy" />
            {:else}
              <span>{controller.appIconFallback(app.displayName, app.bundleId)}</span>
            {/if}
          </span>
          <Switch
            checked={app.enabled}
            onCheckedChange={(enabled) => controller.setPrivacyExcludedAppEnabled(app.id, enabled)}
            label={app.displayName}
            description={app.bundleId}
            disabled={controller.commandInFlight}
          />
          <button
            class="btn btn--ghost btn--sm"
            type="button"
            disabled={controller.commandInFlight}
            onclick={() => controller.removePrivacyApp(app.id)}
          >
            Remove
          </button>
        </div>
      {/each}
    {:else}
      <p class="empty-state">No app exclusions.</p>
    {/if}
  </div>
  <p class="hint">Mnema records visible content from non-excluded apps, including private/incognito browser windows. Exclude the whole browser app to keep browser content out of recordings.</p>
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

  .app-privacy-exclusion :global(.settings-list-item--app-rule .switch-wrapper),
  .app-privacy-exclusion :global(.settings-list-item--app-rule .switch-text) {
    min-width: 0;
  }

  .app-privacy-exclusion :global(.settings-list-item--app-rule .switch-label),
  .app-privacy-exclusion :global(.settings-list-item--app-rule .switch-description) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    box-shadow: 0 12px 30px color-mix(in srgb, var(--app-bg) 34%, transparent);
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
    opacity: 0.35;
    cursor: not-allowed;
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
