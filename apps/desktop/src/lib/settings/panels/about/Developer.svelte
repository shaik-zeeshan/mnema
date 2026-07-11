<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconClear from "~icons/lucide/x";

  const c = getSettingsController();
  const rec = c.rec;
  const logs = c.logs;

  // ─── c.logs read aliases ────────────────────────────────────────────────
  const debugLogStatus = $derived(logs.debugLogStatus);
  const loadingDebugLogStatus = $derived(logs.loadingDebugLogStatus);
  const openingDebugLog = $derived(logs.openingDebugLog);
  const deletingDebugLog = $derived(logs.deletingDebugLog);
  const debugLogDeleted = $derived(logs.debugLogDeleted);
  const generalLogStatus = $derived(logs.generalLogStatus);
  const loadingGeneralLogStatus = $derived(logs.loadingGeneralLogStatus);
  const openingGeneralLog = $derived(logs.openingGeneralLog);
  const deletingGeneralLog = $derived(logs.deletingGeneralLog);
  const generalLogDeleted = $derived(logs.generalLogDeleted);

  // NOTE: `debugLogError` / `generalLogError` are written by the markup, so they
  // reference the store directly (`logs.X`, which has setters) rather than
  // read-only `$derived` aliases.

  // ─── method wrappers ────────────────────────────────────────────────────
  const openDebugLog = () => logs.openDebugLog();
  const deleteDebugLog = () => logs.deleteDebugLog();
  const openGeneralLog = () => logs.openGeneralLog();
  const deleteGeneralLog = () => logs.deleteGeneralLog();
</script>

<SettingGroup id="settings-section-developer" title="Developer &amp; Logs">
  <SettingRow
    label="Enable developer options"
    description="Reveal the Debug surface in the navigation bar (raw session, system probe, idle policy, app-infra and background-job tools) and record verbose Debug-level entries in the General Application Log. When disabled, the Debug page is hidden, visiting it redirects to the Timeline, and the General Application Log keeps only high-level events and errors. Development builds always log verbosely. Changes auto-save and apply immediately."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftDeveloperOptionsEnabled} ariaLabel="Enable developer options" />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Native capture debug logging"
    description="Write native capture diagnostic output to a log file on disk for troubleshooting. Changes auto-save and apply immediately."
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftNativeCaptureDebugLoggingEnabled} ariaLabel="Native capture debug logging" />
    {/snippet}
  </SettingRow>

  {#if debugLogStatus || loadingDebugLogStatus || logs.debugLogError}
    <SettingRow label="Native capture log" description="Where diagnostic output is written on disk." full>
      {#snippet control()}
        <div class="dev-log">
          {#if debugLogStatus}
            <div class="debug-log-status">
              <div class="debug-log-status__row">
                <span class="debug-log-status__label">Status</span>
                <span class="debug-log-status__value">
                  {#if debugLogStatus.enabled}
                    <span class="debug-log-status__dot debug-log-status__dot--on"></span> Active
                  {:else}
                    <span class="debug-log-status__dot"></span> Inactive
                  {/if}
                </span>
              </div>
              <div class="debug-log-status__row">
                <span class="debug-log-status__label">Path</span>
                <span class="debug-log-status__path" use:tip={debugLogStatus.path}>{debugLogStatus.path}</span>
              </div>
              <div class="debug-log-status__row">
                <span class="debug-log-status__label">File</span>
                <span class="debug-log-status__value">{debugLogStatus.exists ? "Exists on disk" : "Not found"}</span>
              </div>
            </div>

            <div class="debug-log-actions">
              <button
                type="button"
                class="btn btn--ghost btn--sm"
                onclick={openDebugLog}
                disabled={openingDebugLog}
                aria-busy={openingDebugLog}
              >
                {#if openingDebugLog}
                  <ButtonSpinner />Opening…
                {:else if debugLogStatus.exists}
                  Open Log File
                {:else}
                  Open Containing Folder
                {/if}
              </button>
              {#if debugLogStatus.exists}
                <button
                  type="button"
                  class="btn btn--danger btn--sm"
                  onclick={deleteDebugLog}
                  disabled={deletingDebugLog}
                  aria-busy={deletingDebugLog}
                >
                  {#if deletingDebugLog}<ButtonSpinner />Deleting…{:else}Delete Log File{/if}
                </button>
              {/if}
              {#if debugLogDeleted}
                <span class="saved-badge">✓ Deleted</span>
              {/if}
            </div>
          {:else if loadingDebugLogStatus}
            <p class="loading-text">Loading log status…</p>
          {/if}

          {#if logs.debugLogError}
            <div class="inline-error">
              <span class="inline-error__icon" aria-hidden="true"><IconAlert /></span>
              <span class="inline-error__msg">{logs.debugLogError}</span>
              <button type="button" class="settings-icon-btn" aria-label="Dismiss error" onclick={() => logs.debugLogError = null}><IconClear aria-hidden="true" /></button>
            </div>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow
    label="General application log"
    description="Captures high-level runtime events and errors."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="dev-log">
        {#if generalLogStatus}
          <div class="debug-log-status">
            <div class="debug-log-status__row">
              <span class="debug-log-status__label">Path</span>
              <span class="debug-log-status__path" use:tip={generalLogStatus.path}>{generalLogStatus.path}</span>
            </div>
            <div class="debug-log-status__row">
              <span class="debug-log-status__label">File</span>
              <span class="debug-log-status__value">{generalLogStatus.exists ? "Exists on disk" : "Not found"}</span>
            </div>
          </div>

          <div class="debug-log-actions">
            <button
              type="button"
              class="btn btn--ghost btn--sm"
              onclick={openGeneralLog}
              disabled={openingGeneralLog}
              aria-busy={openingGeneralLog}
            >
              {#if openingGeneralLog}
                <ButtonSpinner />Opening…
              {:else if generalLogStatus.exists}
                Open Log File
              {:else}
                Open Containing Folder
              {/if}
            </button>
            {#if generalLogStatus.exists}
              <button
                type="button"
                class="btn btn--danger btn--sm"
                onclick={deleteGeneralLog}
                disabled={deletingGeneralLog}
                aria-busy={deletingGeneralLog}
              >
                {#if deletingGeneralLog}<ButtonSpinner />Deleting…{:else}Delete Log File{/if}
              </button>
            {/if}
            {#if generalLogDeleted}
              <span class="saved-badge">✓ Deleted</span>
            {/if}
          </div>
        {:else if loadingGeneralLogStatus}
          <p class="loading-text">Loading log status…</p>
        {/if}

        {#if logs.generalLogError}
          <div class="inline-error">
            <span class="inline-error__icon" aria-hidden="true"><IconAlert /></span>
            <span class="inline-error__msg">{logs.generalLogError}</span>
            <button type="button" class="settings-icon-btn" aria-label="Dismiss error" onclick={() => logs.generalLogError = null}><IconClear aria-hidden="true" /></button>
          </div>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Log status + actions stacked inside a full-width row's control slot. */
  .dev-log {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

</style>
