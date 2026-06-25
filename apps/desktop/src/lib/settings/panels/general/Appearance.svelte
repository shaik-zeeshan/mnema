<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import ThemeModeControl from "$lib/components/ThemeModeControl.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;
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
  >
    {#snippet control()}
      <ThemeModeControl bind:value={rec.draftAppearance} />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Follow live recording"
    description="Keep the timeline pinned to the latest captured data while recording"
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftFollowTimelineLive} ariaLabel="Follow live recording" />
    {/snippet}
  </SettingRow>
</SettingGroup>
