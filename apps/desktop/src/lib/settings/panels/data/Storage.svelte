<script lang="ts">
  import { setSettingsSection } from "$lib/settings/state/settings-find.svelte";

  // Every SettingRow below belongs to this section (⌘F row index scope, G7).
  setSettingsSection("storage");

  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { humanizeError } from "$lib/format-error";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import RetentionPicker from "$lib/components/RetentionPicker.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import { systemFacts } from "$lib/settings/state/system-facts.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  const retentionCleanupSummary = $derived(c.retentionCleanupSummary);
  const retentionCleanupRunning = $derived(c.retentionCleanupRunning);
  const retentionCleanupError = $derived(c.retentionCleanupError);

  // G8: what this window actually keeps, at the rate this machine measured.
  // Null until a complete capture day exists — then the row simply says nothing.
  // The phrase itself now lives inside the retention INSTRUMENT's readout
  // (direction 05), so this row no longer prints it a second time; the facts
  // are still loaded here because the cleanup button below re-reads them.
  void systemFacts.ensureLoaded();

  // A cleanup changes what is on disk, so the measured rate is re-read after it.
  const runRetentionCleanupNow = async () => {
    await c.runRetentionCleanupNow();
    await systemFacts.refresh();
  };

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

  // ── Delete Recent Capture ───────────────────────────────────────────────────
  // Page 11 draws this beside retention; until now it existed only in the tray
  // submenu. Same backend command, same confirm wording. The backend accepts
  // exactly 60/300/900 (`validate_delete_recent_window`), so the control is
  // three buttons rather than a free window — an honest control, not a picker
  // that can be set to a value the command rejects.
  // ponytail: no window state, no picker component; three invokes.
  const DELETE_RECENT_WINDOWS = [
    { seconds: 60, label: "Last minute", phrase: "last 1 minute" },
    { seconds: 300, label: "Last 5 minutes", phrase: "last 5 minutes" },
    { seconds: 900, label: "Last 15 minutes", phrase: "last 15 minutes" },
  ] as const;

  interface DeleteRecentSummary {
    deletedCaptureSegments: number;
    deletedFrames: number;
    deletedAudioSegments: number;
    fileDeleteErrors: number;
    pendingFileTombstones: number;
  }

  let deletingRecentSeconds = $state<number | null>(null);
  let deleteRecentSummary = $state<DeleteRecentSummary | null>(null);
  let deleteRecentError = $state<string | null>(null);

  async function deleteRecentCapture(seconds: number, phrase: string) {
    if (deletingRecentSeconds !== null) return;
    const ok = await confirm(
      `Delete the ${phrase} from Mnema's library? This removes whole overlapping capture segments and cannot be undone.`,
      { title: "Delete Recent Capture", kind: "warning" },
    );
    if (!ok) return;
    deletingRecentSeconds = seconds;
    deleteRecentError = null;
    deleteRecentSummary = null;
    try {
      deleteRecentSummary = await invoke<DeleteRecentSummary>("delete_recent_capture", {
        request: { windowSeconds: seconds },
      });
      // Deleting changes what is on disk, so the measured rate the retention
      // instrument prices itself against is re-read.
      await systemFacts.refresh();
    } catch (err) {
      deleteRecentError = humanizeError(err);
    } finally {
      deletingRecentSeconds = null;
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
                class="btn btn--primary btn--sm"
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
  >
    {#snippet control()}
      <div class="retention-control">
        <!-- The instrument owns the consequence now: its readout states what
             this window keeps, with its denominator. The old `retentionHint`
             paragraph said the same thing a second time. -->
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
          <div class="cleanup-result" aria-live="polite">
            <strong>Latest cleanup</strong>
            <p>
              {retentionCleanupSummary.deletedCaptureSegments} segment(s), {retentionCleanupSummary.deletedFrames}
              frame(s), {retentionCleanupSummary.deletedAudioSegments} audio segment(s).
            </p>
          </div>
        {/if}
        {#if retentionCleanupError}
          <p class="error-text">{retentionCleanupError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Delete recent capture"
    description="Remove everything recorded in a recent window — screen, mic and system audio. Whole overlapping segments go; this cannot be undone."
    full
  >
    {#snippet control()}
      <div class="delete-recent">
        <div class="row-actions">
          {#each DELETE_RECENT_WINDOWS as window (window.seconds)}
            <button
              type="button"
              class="btn btn--danger btn--sm"
              onclick={() => deleteRecentCapture(window.seconds, window.phrase)}
              disabled={deletingRecentSeconds !== null}
              aria-busy={deletingRecentSeconds === window.seconds}
            >
              {#if deletingRecentSeconds === window.seconds}<ButtonSpinner />Deleting…{:else}{window.label}{/if}
            </button>
          {/each}
        </div>
        {#if deleteRecentSummary}
          <div class="cleanup-result" aria-live="polite">
            <strong>Deleted</strong>
            <p>
              {deleteRecentSummary.deletedCaptureSegments} segment(s), {deleteRecentSummary.deletedFrames}
              frame(s), {deleteRecentSummary.deletedAudioSegments} audio segment(s).
              {#if deleteRecentSummary.fileDeleteErrors > 0}
                {deleteRecentSummary.fileDeleteErrors} file(s) could not be removed from disk and were
                queued for retry ({deleteRecentSummary.pendingFileTombstones} pending).
              {/if}
            </p>
          </div>
        {/if}
        {#if deleteRecentError}
          <p class="error-text">{deleteRecentError}</p>
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
    /* Read-only display: de-chrome the editable recess into a flat path/code
       chip so the field doesn't invite typing. */
    cursor: default;
    box-shadow: none;
    background: var(--app-surface-subtle);
    font-family: var(--app-font-mono);
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

  /* Same stacked shape as the retention control: the three window buttons, then
     whatever the last run reported. */
  .delete-recent {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  .delete-recent .row-actions {
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
