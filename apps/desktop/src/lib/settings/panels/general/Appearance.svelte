<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

  // Near-the-control autosave cue (theme + follow-live both persist through the
  // "display" recording domain).
  const displaySaving = $derived(c.rec.savingRecDomains.display);
  const displaySaved = $derived(c.recSavedDomain === "display");
</script>

<SettingGroup
  id="settings-section-appearance"
  title="Appearance"
  hint="Theme switches immediately when saved and is also available from every titlebar."
>
  <SettingRow
    label="Theme"
    description="System follows your OS setting; pick Light or Dark to override it."
    full
    saving={displaySaving}
    saved={displaySaved}
  >
    {#snippet control()}
      <ThemeModeControl bind:value={rec.draftAppearance} />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Follow live recording"
    description="Keep the timeline pinned to the latest captured data while recording"
    saving={displaySaving}
    saved={displaySaved}
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftFollowTimelineLive} ariaLabel="Follow live recording" />
    {/snippet}
  </SettingRow>
</SettingGroup>
