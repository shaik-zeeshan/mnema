<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SelectMenu from "$lib/components/Select.svelte";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const appPrivacyExclusion = c.appPrivacyExclusion;

  const setBrowserUrlMode = (m: string) => c.setBrowserUrlMode(m);
</script>

<SettingGroup id="settings-section-privacy" title="Privacy">
  <SettingRow
    label="Capture frame context"
    description="Store app, window, and supported browser context with frames"
  >
    {#snippet control()}
      <Switch bind:checked={rec.draftMetadataEnabled} />
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Browser URL mode"
    description="Sanitized URLs keep scheme, host, port, and path while dropping query strings and fragments."
    disabled={!rec.draftMetadataEnabled}
  >
    {#snippet control()}
      <div class="select-cell">
        <SelectMenu
          value={rec.draftBrowserUrlMode}
          onValueChange={setBrowserUrlMode}
          options={[
            { value: "off", label: "Off" },
            { value: "sanitized", label: "Sanitized" },
            { value: "full", label: "Full" },
          ]}
          disabled={!rec.draftMetadataEnabled}
        />
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Excluded Apps"
    description="Apps whose visible content is never recorded."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="exclusion-cell" class:exclusion-cell--open={appPrivacyExclusion.comboboxOpen}>
        <AppPrivacyExclusion controller={appPrivacyExclusion} />
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Keep the browser-URL select compact in the right gutter rather than letting
     it grow full width inside the row control. */
  .select-cell {
    width: 200px;
    max-width: 100%;
  }

  /* The exclusion editor owns a combobox dropdown that overlays following rows;
     elevate this cell while it is open so the panel isn't covered. */
  .exclusion-cell {
    width: 100%;
    min-width: 0;
  }

  .exclusion-cell--open {
    position: relative;
    z-index: 10;
  }
</style>
