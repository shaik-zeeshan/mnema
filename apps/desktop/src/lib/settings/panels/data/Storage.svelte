<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { homeDir } from "@tauri-apps/api/path";
  import { open, confirm } from "@tauri-apps/plugin-dialog";
  import { humanizeError } from "$lib/format-error";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import { retentionPresets } from "$lib/components/retention";
  import type { RetentionPolicy } from "$lib/types";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import IconFolder from "~icons/lucide/folder";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCheck from "~icons/lucide/check";
  import StorageMeter from "./StorageMeter.svelte";
  import DeleteRecentCapture from "./DeleteRecentCapture.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  const cleanup = $derived(c.retentionCleanupSummary);
  const cleanupRunning = $derived(c.retentionCleanupRunning);
  const cleanupError = $derived(c.retentionCleanupError);
  const runCleanup = () => c.runRetentionCleanupNow();

  const count = (n: number, one: string) => `${n.toLocaleString()} ${one}${n === 1 ? "" : "s"}`;

  // The resolved on-disk storage root, fetched from the backend so it reflects
  // the env-honoring resolution (MNEMA_SAVE_DIRECTORY, else ~/.mnema) rather
  // than whatever raw string happens to be persisted. Display-only; the folder
  // is changed through the Browse picker, which writes `save_directory`.
  // "" = still resolving (skeleton chip).
  let storageLocation = $state("");
  let storageLocationError = $state<string | null>(null);
  let browsing = $state(false);
  let home = $state("");

  // The storage root currently IN EFFECT (resolved at launch). The backend only
  // re-resolves it on the next launch, so a freshly-picked directory updates the
  // display but not this — the gap drives the "restart to apply" notice below.
  let appliedLocation = $state("");

  // A picked directory differs from the one the running app resolved at launch,
  // so the change is saved but won't take effect until Mnema restarts.
  const pendingRestart = $derived(
    appliedLocation.length > 0 && storageLocation.length > 0 && storageLocation !== appliedLocation,
  );

  // Home shown as `~` (display only; Browse and the saved value keep the full path).
  const shownPath = $derived(
    home && (storageLocation === home || storageLocation.startsWith(`${home}/`))
      ? `~${storageLocation.slice(home.length)}`
      : storageLocation,
  );

  async function loadStorageLocation() {
    storageLocationError = null;
    try {
      storageLocation = await invoke<string>("get_storage_location");
      appliedLocation = storageLocation;
    } catch (err) {
      storageLocationError = humanizeError(err);
    }
  }

  onMount(() => {
    void loadStorageLocation();
    homeDir()
      .then((h) => (home = (h ?? "").replace(/\/+$/, "")))
      .catch(() => {});
  });

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
        defaultPath: storageLocation || rec.draftSaveDirectory || undefined,
      });
      if (typeof picked === "string" && picked.trim().length > 0) {
        rec.draftSaveDirectory = picked;
        storageLocation = picked;
        storageLocationError = null;
      }
    } catch (err) {
      // The folder picker can reject (dialog plugin error / cancelled-by-error).
      storageLocationError = humanizeError(err);
    } finally {
      browsing = false;
    }
  }
</script>

<SettingGroup id="settings-section-storage" title="Storage">
  {#if pendingRestart}
    <div class="mx-notice storage-notice" data-tone="warn" role="status">
      <span class="mx-notice__icon"><IconFolder width="15" height="15" /></span>
      <b class="mx-notice__title">Saved — takes effect after a restart</b>
      <p class="mx-notice__text">Captures already on disk stay where they are. Any recording in progress is saved first.</p>
      <div class="mx-notice__acts">
        <button type="button" class="mx-btn mx-btn--sm" onclick={restartNow} disabled={restarting} aria-busy={restarting}>
          {#if restarting}<span class="mx-spin mx-spin--sm"></span>Restarting…{:else}Restart Mnema{/if}
        </button>
      </div>
    </div>
  {/if}
  <StorageMeter />
  <SettingRow label="Save directory" description="Where captures and the database live on disk. Downloaded models stay in Mnema's app data.">
    {#snippet control()}
      {#if storageLocation}
        <span class="storage-path" title={storageLocation}>{shownPath}</span>
      {:else if storageLocationError}
        <span class="storage-path storage-path--muted">—</span>
      {:else}
        <span class="storage-path storage-path--muted" aria-busy="true">
          <span class="mx-sr">Resolving storage location…</span><span class="mx-skel mx-skel--line"></span>
        </span>
      {/if}
      <button type="button" class="mx-btn" onclick={browseSaveDirectory} disabled={browsing} aria-busy={browsing}>
        {#if browsing}<span class="mx-spin mx-spin--sm"></span>{:else}<IconFolder width="14" height="14" />{/if}Browse…
      </button>
    {/snippet}
    {#snippet below()}
      {#if storageLocationError}
        <span class="mx-inline" data-tone="danger" role="alert">
          <IconAlert width="13" height="13" />{storageLocation ? storageLocationError : "Couldn’t resolve storage location."}
          {#if !storageLocation}
            <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={loadStorageLocation}>Retry</button>
          {/if}
        </span>
      {/if}
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Retention"
    description="Automatically delete captured data after the chosen window. Shortening it shows what will go first. Your User Context is kept."
  >
    {#snippet control()}
      <Segmented
        md
        value={rec.draftRetentionPolicy}
        onValueChange={(v) => (rec.draftRetentionPolicy = v as RetentionPolicy)}
        options={retentionPresets()}
        ariaLabel="Retention duration"
      />
      <button type="button" class="mx-btn mx-btn--ghost" onclick={runCleanup} disabled={cleanupRunning} aria-busy={cleanupRunning}>
        {#if cleanupRunning}<span class="mx-spin mx-spin--sm"></span>Running…{:else}Run cleanup now{/if}
      </button>
    {/snippet}
    {#snippet below()}
      {#if cleanupError}
        <span class="mx-inline" data-tone="danger" role="alert">
          <IconAlert width="13" height="13" />Cleanup failed: {cleanupError}
          <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={runCleanup} disabled={cleanupRunning}>Retry</button>
        </span>
      {:else if cleanup}
        <span class="mx-inline" data-tone="ok" aria-live="polite">
          <IconCheck width="13" height="13" /><b>Latest cleanup</b>
          {count(cleanup.deletedCaptureSegments, "segment")}, {count(cleanup.deletedFrames, "frame")}, {count(cleanup.deletedAudioSegments, "audio segment")}.
        </span>
      {/if}
    {/snippet}
  </SettingRow>
</SettingGroup>
<DeleteRecentCapture />

<style>
  /* Mockup `.st-section > .mx-notice`. */
  .storage-notice {
    margin-bottom: var(--s-3);
  }

  /* Mockup `.st-path`: a read-only mono path chip. */
  .storage-path {
    display: inline-block;
    height: var(--h-md);
    box-sizing: border-box;
    max-width: 320px;
    padding: 0 var(--s-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font: 400 var(--text-base) / calc(var(--h-md) - 2px) var(--font-mono);
    color: var(--app-text);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
  }

  .storage-path--muted {
    color: var(--app-text-subtle);
  }

  .storage-path .mx-skel {
    display: inline-block;
    vertical-align: middle;
    width: 90px;
  }
</style>
