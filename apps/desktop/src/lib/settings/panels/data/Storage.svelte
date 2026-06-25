<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  const retentionCleanupSummary = $derived(c.retentionCleanupSummary);
  const retentionCleanupRunning = $derived(c.retentionCleanupRunning);
  const retentionCleanupError = $derived(c.retentionCleanupError);

  const runRetentionCleanupNow = () => c.runRetentionCleanupNow();

  // The resolved on-disk storage root, fetched from the backend so it reflects
  // the env-honoring resolution (MNEMA_SAVE_DIRECTORY, else ~/.mnema) rather
  // than whatever raw string happens to be persisted. Display-only; the folder
  // is changed through the Browse picker, which writes `save_directory`.
  let storageLocation = $state("");
  let storageLocationError = $state<string | null>(null);
  let browsing = $state(false);

  const displayPath = $derived(storageLocation || rec.draftSaveDirectory);

  async function loadStorageLocation() {
    try {
      storageLocation = await invoke<string>("get_storage_location");
      storageLocationError = null;
    } catch (err) {
      storageLocationError = typeof err === "string" ? err : String(err);
    }
  }

  onMount(loadStorageLocation);

  async function browseSaveDirectory() {
    if (browsing) return;
    browsing = true;
    try {
      const picked = await open({
        directory: true,
        multiple: false,
        title: "Choose where Mnema stores captures",
        defaultPath: displayPath || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        // Drive the autosaved draft; mirror it into the display immediately so
        // the field doesn't lag behind the picker (the backend only re-resolves
        // the storage root on the next launch).
        rec.draftSaveDirectory = picked;
        storageLocation = picked;
      }
    } finally {
      browsing = false;
    }
  }
</script>

{#snippet spinner()}
  <svg class="btn-spinner" viewBox="0 0 24 24" aria-hidden="true">
    <path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8" />
    <path d="M21 3v5h-5" />
  </svg>
{/snippet}

<SettingGroup
  id="settings-section-storage"
  title="Storage"
  hint="Where capture files live on disk and how long they are kept."
>
  <SettingRow
    label="Save Directory"
    description="Where captures, the database, and model caches live on disk."
    full
  >
    {#snippet control()}
      <div class="storage-control">
        <div class="path-field">
          <input
            type="text"
            class="text-input"
            class:text-input--empty={!displayPath}
            value={displayPath}
            readonly
            placeholder={storageLocationError ? "Couldn't resolve storage location" : "Resolving storage location…"}
            aria-label="Storage location"
          />
          <button
            type="button"
            class="btn btn--ghost"
            onclick={browseSaveDirectory}
            disabled={browsing}
            aria-busy={browsing}
          >
            {#if browsing}{@render spinner()}Browsing…{:else}Browse{/if}
          </button>
        </div>
        {#if storageLocationError}
          <p class="error-text">{storageLocationError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Retention"
    description="Automatically delete captured data after the chosen window."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="retention-control">
        <RetentionPicker bind:value={rec.draftRetentionPolicy} />
        <div class="row-actions">
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            onclick={runRetentionCleanupNow}
            disabled={retentionCleanupRunning}
            aria-busy={retentionCleanupRunning}
          >
            {#if retentionCleanupRunning}{@render spinner()}Running…{:else}Run cleanup now{/if}
          </button>
        </div>
        {#if retentionCleanupSummary}
          <p class="group-hint">
            Latest cleanup: {retentionCleanupSummary.deletedCaptureSegments} segment(s), {retentionCleanupSummary.deletedFrames}
            frame(s), {retentionCleanupSummary.deletedAudioSegments} audio segment(s).
          </p>
        {/if}
        {#if retentionCleanupError}
          <p class="error-text">{retentionCleanupError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Save-directory control: a read-only path that mirrors the resolved storage
     root, paired with a Browse button that opens a folder picker. */
  .storage-control {
    display: flex;
    flex-direction: column;
    gap: 8px;
    width: 100%;
  }

  .path-field {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
  }

  .path-field .text-input {
    flex: 1 1 auto;
    min-width: 0;
    /* Read-only display: keep the recessed field look but signal non-editing. */
    cursor: default;
  }

  .path-field .btn {
    flex-shrink: 0;
  }

  /* The retention control stacks its picker, the run-now action, and any
     summary/error hint. The shared row primitive already gives a full-width
     column; this just spaces the parts. */
  .retention-control {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  .retention-control .row-actions {
    justify-content: flex-start;
  }

  /* Inline busy spinner shown beside a button label while an action is in
     flight; reuses the shared settings-icon-spin keyframe. */
  .btn-spinner {
    width: 13px;
    height: 13px;
    margin-right: 6px;
    vertical-align: -2px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
    animation: settings-icon-spin 0.7s linear infinite;
  }

  @media (prefers-reduced-motion: reduce) {
    .btn-spinner {
      animation: none;
    }
  }
</style>
