<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import type { AppearanceSetting, TimeFormatSetting } from "$lib/types";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;

</script>

<SettingGroup
  id="settings-section-appearance"
  title="Appearance"
  hint="Theme switches immediately when saved."
>
  <SettingRow
    label="Theme"
    description="System follows your OS setting; pick Light or Dark to override it."
  >
    {#snippet control()}
      <Segmented
        md
        value={rec.draftAppearance}
        onValueChange={(v) => (rec.draftAppearance = v as AppearanceSetting)}
        ariaLabel="Theme"
        options={[
          { value: "system", label: "System" },
          { value: "light", label: "Light" },
          { value: "dark", label: "Dark" },
        ]}
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Time format" description="System follows your Mac's 24-hour time setting.">
    {#snippet control()}
      <Segmented
        md
        value={rec.draftTimeFormat}
        onValueChange={(v) => (rec.draftTimeFormat = v as TimeFormatSetting)}
        ariaLabel="Time format"
        options={[
          { value: "system", label: "System" },
          { value: "h12", label: "12-hour" },
          { value: "h24", label: "24-hour" },
        ]}
      />
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
