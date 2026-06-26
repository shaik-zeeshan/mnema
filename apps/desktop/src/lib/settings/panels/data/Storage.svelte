<script lang="ts">
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { humanizeError } from "$lib/format-error";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  const retentionCleanupSummary = $derived(c.retentionCleanupSummary);
  const retentionCleanupRunning = $derived(c.retentionCleanupRunning);
  const retentionCleanupError = $derived(c.retentionCleanupError);

  // Per-control autosave micro-affordance: the rail footer status is remote from
  // the edit, so mirror the "storage" domain's saving/just-saved state right next
  // to these controls (Save Directory + Retention both autosave through it).
  const storageSaving = $derived(c.rec.savingRecDomains.storage);
  const storageSaved = $derived(c.recSavedDomain === "storage");

  const runRetentionCleanupNow = () => c.runRetentionCleanupNow();

  // The resolved on-disk storage root, fetched from the backend so it reflects
  // the env-honoring resolution (MNEMA_SAVE_DIRECTORY, else ~/.mnema) rather
  // than whatever raw string happens to be persisted. Display-only; the folder
  // is changed through the Browse picker, which writes `save_directory`.
  let storageLocation = $state("");
  let storageLocationError = $state<string | null>(null);
  let browsing = $state(false);

  // The storage root currently IN EFFECT (resolved at launch). The backend only
  // re-resolves it on the next launch, so a freshly-picked directory updates the
  // display but not this — the gap drives the "restart to apply" notice below.
  let appliedLocation = $state("");

  const displayPath = $derived(storageLocation || rec.draftSaveDirectory);

  // A picked directory differs from the one the running app resolved at launch,
  // so the change is saved but won't take effect until Mnema restarts.
  const pendingRestart = $derived(
    appliedLocation.length > 0 &&
      displayPath.length > 0 &&
      displayPath !== appliedLocation,
  );

  async function loadStorageLocation() {
    try {
      storageLocation = await invoke<string>("get_storage_location");
      appliedLocation = storageLocation;
      storageLocationError = null;
    } catch (err) {
      storageLocationError = humanizeError(err);
    }
  }

  onMount(loadStorageLocation);

  // Apply a pending save-directory change by relaunching. The backend
  // (`request_app_relaunch`) finalizes any in-flight recording before
  // restarting, so this is safe mid-capture — confirm first because a restart is
  // disruptive, then leave the button busy until the process tears down.
  let restarting = $state(false);

  async function restartNow() {
    if (restarting) return;
    const ok = await confirm(
      "Restart now? Any in-progress recording will be saved first.",
      { title: "Restart Mnema", kind: "warning" },
    );
    if (!ok) return;
    restarting = true;
    try {
      await invoke<void>("request_app_relaunch");
    } catch (err) {
      // Relaunch failed before tearing down — surface it and re-enable the button.
      storageLocationError = humanizeError(err);
      restarting = false;
    }
  }

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
        storageLocationError = null;
      }
    } catch (err) {
      // The folder picker can reject (dialog plugin error / cancelled-by-error).
      // Surface it instead of swallowing — the error-text block below renders it.
      storageLocationError = humanizeError(err);
    } finally {
      browsing = false;
    }
  }
</script>

<SettingGroup
  id="settings-section-storage"
  title="Storage"
  hint="Where capture files live on disk and how long they are kept."
>
  <SettingRow
    label="Save Directory"
    description="Where captures, the database, and model caches live on disk."
    full
    saving={storageSaving}
    saved={storageSaved}
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
            {#if browsing}<ButtonSpinner />Browsing…{:else}Browse{/if}
          </button>
        </div>
        {#if storageLocationError}
          <p class="error-text">{storageLocationError}</p>
        {/if}
        {#if pendingRestart}
          <div class="restart-notice" role="status">
            <p class="group-hint group-hint--warn">
              Saved — but this takes effect after you restart Mnema. Captures already on disk stay where they are.
            </p>
            <div class="row-actions">
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={restartNow}
                disabled={restarting}
                aria-busy={restarting}
              >
                {#if restarting}<ButtonSpinner />Restarting…{:else}Restart Mnema{/if}
              </button>
            </div>
          </div>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Retention"
    description="Automatically delete captured data after the chosen window."
    full
    divider={false}
    saving={storageSaving}
    saved={storageSaved}
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
            {#if retentionCleanupRunning}<ButtonSpinner />Running…{:else}Run cleanup now{/if}
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

  /* The pending-restart notice stacks its warning copy above an inline
     "Restart Mnema" action, left-aligned under the path field. */
  .restart-notice {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .restart-notice .row-actions {
    display: flex;
    justify-content: flex-start;
  }
</style>
