<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import type { RetentionPolicy } from "$lib/types";

  const c = getSettingsController();
  const rec = c.rec;

  const retentionCleanupSummary = $derived(c.retentionCleanupSummary);
  const retentionCleanupRunning = $derived(c.retentionCleanupRunning);
  const retentionCleanupError = $derived(c.retentionCleanupError);

  const runRetentionCleanupNow = () => c.runRetentionCleanupNow();
</script>

<SettingGroup
  id="settings-section-storage"
  title="Storage & Startup"
  hint="Where capture files live on disk and how long they are kept."
>
  <SettingRow
    label="Save Directory"
    description="Where capture files are saved on disk."
    full
  >
    {#snippet control()}
      <div class="input-row">
        <input
          type="text"
          class="text-input"
          class:text-input--empty={!rec.draftSaveDirectory}
          bind:value={rec.draftSaveDirectory}
          placeholder="/path/to/recordings"
        />
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
        <SelectMenu
          value={rec.draftRetentionPolicy}
          onValueChange={(v) => {
            rec.draftRetentionPolicy = v as RetentionPolicy;
          }}
          label="Delete captured data"
          options={[
            { value: "never", label: "Never" },
            { value: "days_7", label: "After 7 days" },
            { value: "days_14", label: "After 14 days" },
            { value: "days_30", label: "After 30 days" },
          ]}
        />
        <div class="row-actions">
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            onclick={runRetentionCleanupNow}
            disabled={retentionCleanupRunning}
          >
            {retentionCleanupRunning ? "Running…" : "Run cleanup now"}
          </button>
        </div>
        {#if retentionCleanupSummary}
          <p class="group-hint">
            Latest cleanup: {retentionCleanupSummary.deletedCaptureSegments} segment(s), {retentionCleanupSummary.deletedFrames}
            frame(s), {retentionCleanupSummary.deletedAudioSegments} audio segment(s).
          </p>
        {/if}
        {#if retentionCleanupError}
          <p class="group-hint group-hint--error">{retentionCleanupError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* The retention control stacks its SelectMenu, the run-now action, and any
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
</style>
