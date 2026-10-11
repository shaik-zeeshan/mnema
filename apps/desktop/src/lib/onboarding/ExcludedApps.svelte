<!--
  Onboarding "Never recorded" (issue #195, slice 7).

  Skin of record: `docs/mockups/redesign/onboarding.html` (Capture & Storage,
  the "Never recorded" row). One `.ob-row`: each app is an `mx-btn--sm` toggle
  with `aria-pressed` (pressed = never recorded; crossed out when recorded) and
  an icon-only Add opens a type-ahead field.

  Load-bearing rules kept from the sentence version:
   · A struck name KEEPS ITS SLOT. Striking is the existing
     `set_privacy_excluded_app_enabled(sourceId, false)`, so the entry stays in
     `PrivacySettings.excluded_apps`. Nothing can be removed from here at all —
     `remove_privacy_excluded_app` is not wired.
   · A rule for an app that is not installed renders PENDING (dashed empty icon
     slot + "when installed"), never as protecting — `isPendingExclusion` /
     `activeExclusionBundleIds` are the frontend mirror of `evaluate_privacy`.
-->
<script lang="ts">
  import { tick } from "svelte";
  import { isPendingExclusion, type PrivacyAppCandidate } from "$lib/app-privacy-exclusion";
  import {
    addAnnouncement,
    entryLabel,
    excludedNote,
    resolveTypedApp,
    strikeAnnouncement,
  } from "$lib/onboarding/excluded-apps";
  import type { ExcludedAppEntry } from "$lib/types";
  import IconPlus from "~icons/lucide/plus";

  /**
   * The privacy controller's surface, declared structurally so this component
   * doesn't reach into `routes/`. `createAppPrivacyExclusionController()`
   * satisfies it — the wiring is `<ExcludedApps privacy={c.appPrivacyExclusion} />`.
   */
  interface ExcludedAppsStore {
    /** Plain rows, in stored order. A struck row is `enabled: false`, still here. */
    excludedApps: ExcludedAppEntry[];
    /** Installed apps, for name resolution and the add field's suggestions. */
    candidates: PrivacyAppCandidate[];
    appIconSrcForBundleId: (bundleId: string) => string | null;
    appIconFallback: (displayName: string | null, bundleId: string | null) => string;
    setPrivacyExcludedAppEnabled: (sourceId: string, enabled: boolean) => void;
    /**
     * Adds, or re-enables a struck rule with the same identity. A candidate with
     * an empty `bundleId` is the "not installed yet" rule:
     * `add_privacy_excluded_app({ bundleId: "", displayName })`.
     */
    addPrivacyAppCandidate: (candidate: PrivacyAppCandidate | null) => void;
  }

  let { privacy }: { privacy: ExcludedAppsStore } = $props();

  const entries = $derived(privacy.excludedApps);
  const note = $derived(excludedNote(entries));

  const listId = $props.id();
  let adding = $state(false);
  let announcement = $state("");
  let addInput = $state<HTMLInputElement | null>(null);
  let addChip = $state<HTMLButtonElement | null>(null);

  function strike(entry: ExcludedAppEntry) {
    const next = !entry.enabled;
    announcement = strikeAnnouncement(entry, next);
    privacy.setPrivacyExcludedAppEnabled(entry.id, next);
  }

  async function openAdd() {
    adding = true;
    await tick();
    addInput?.focus();
  }

  async function closeAdd() {
    adding = false;
    await tick();
    addChip?.focus();
  }

  function commitAdd(typed: string) {
    const name = typed.trim();
    if (!name) return;
    const installed = resolveTypedApp(name, privacy.candidates);
    announcement = addAnnouncement(installed?.displayName ?? name, Boolean(installed));
    // A name that matches nothing installed is still a real rule: stored with an
    // empty bundle id, held pending, resolved on first sighting.
    privacy.addPrivacyAppCandidate(
      installed ?? { id: "", enabled: true, bundleId: "", displayName: name, running: false, iconPath: null },
    );
  }

  function onAddKeydown(event: KeyboardEvent) {
    if (event.key === "Escape") {
      void closeAdd();
      return;
    }
    if (event.key !== "Enter") return;
    // Without this the SAME Enter keystroke activates the name button that lands
    // under focus, and the app you just added is struck out instantly. The mockup
    // shipped that bug; it is only visible by pressing the key.
    event.preventDefault();
    commitAdd(event.currentTarget instanceof HTMLInputElement ? event.currentTarget.value : "");
    void closeAdd();
  }
</script>

<div class="ob-row">
  <span class="mx-label">Never recorded</span>
  <div class="ob-row__ctl ob-apps">
    {#each entries as entry (entry.id)}
      {@const pending = isPendingExclusion(entry)}
      {@const src = pending ? null : privacy.appIconSrcForBundleId(entry.bundleId)}
      <button
        class="mx-btn mx-btn--sm"
        type="button"
        aria-pressed={entry.enabled}
        aria-label="{entryLabel(entry)}{pending ? ', not installed yet' : ''} — {entry.enabled
          ? 'never recorded, activate to record it'
          : 'recorded, activate to hide it again'}"
        onclick={() => strike(entry)}
      >
        <span class="ob-app" class:wait={pending} aria-hidden="true">
          {#if src}<img {src} alt="" loading="lazy" />{:else if !pending}{privacy.appIconFallback(
              entry.displayName,
              entry.bundleId,
            )}{/if}
        </span>
        {entryLabel(entry)}
        {#if pending}<small>· when installed</small>{/if}
      </button>
    {/each}
    {#if adding}
      <input
        bind:this={addInput}
        class="mx-input add-input"
        list={listId}
        autocomplete="off"
        spellcheck="false"
        placeholder="Type any app name"
        aria-label="Add an app Mnema should never record"
        onkeydown={onAddKeydown}
      />
    {:else}
      <button
        bind:this={addChip}
        class="mx-btn mx-btn--ghost mx-btn--sm mx-btn--icon"
        type="button"
        aria-label="Add an app it should never see"
        title="Add an app it should never see"
        onclick={openAdd}
      >
        <IconPlus width="14" height="14" />
      </button>
    {/if}
  </div>
  {#if note.text}<p class="ob-fine note">{note.text}</p>{/if}
</div>

<datalist id={listId}>
  {#each privacy.candidates as candidate (candidate.id)}
    <option value={candidate.displayName}></option>
  {/each}
</datalist>

<p class="ob-sr-only" aria-live="polite">{announcement}</p>

<style>
  .ob-apps .mx-btn:not(.mx-btn--icon) {
    gap: 6px;
    padding-left: 4px;
  }
  /* Not pressed = recorded like everything else: crossed out, icon dimmed. */
  .ob-apps .mx-btn[aria-pressed="false"] {
    --_fg: var(--app-text-subtle);
    --_bg: transparent;
    text-decoration: line-through;
    text-decoration-color: var(--app-text-faint);
  }
  .ob-apps .mx-btn[aria-pressed="false"] .ob-app {
    opacity: 0.45;
    filter: grayscale(1);
  }
  .ob-apps small {
    font: 400 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .ob-app {
    width: 18px;
    height: 18px;
    border-radius: 5px;
    display: grid;
    place-items: center;
    flex: none;
    overflow: hidden;
    font: 600 var(--text-xs) / 1 var(--font-sans);
    color: var(--app-text-strong);
    background: var(--app-surface-raised);
    box-shadow: inset 0 0 0 1px var(--app-border-strong);
  }
  .ob-app img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .ob-app:has(img) {
    background: none;
    box-shadow: none;
  }
  /* Not installed yet: there is no icon to draw, so the slot IS the signal. */
  .ob-app.wait {
    background: none;
    box-shadow: none;
    border: 1px dashed var(--app-warn);
  }
  .add-input {
    width: 190px;
    height: var(--h-sm);
  }
  .note {
    grid-column: 2;
    margin-top: -4px;
  }
</style>
