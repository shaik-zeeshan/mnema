<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;
</script>

<SettingGroup id="settings-section-startup" title="Startup">
  <SettingRow
    label="Auto-start recording on launch"
    description="Begin capturing immediately when the app opens"
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftAutoStart} />
    {/snippet}
  </SettingRow>
</SettingGroup>
