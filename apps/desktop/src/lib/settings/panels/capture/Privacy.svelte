<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
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
    description="How much of a captured browser URL is stored with the frame."
    disabled={!rec.draftMetadataEnabled}
    full
  >
    {#snippet control()}
      <RadioGroup
        value={rec.draftBrowserUrlMode}
        onValueChange={setBrowserUrlMode}
        disabled={!rec.draftMetadataEnabled}
        options={[
          { value: "off", label: "Off", description: "Don't store browser URLs with captured frames." },
          { value: "sanitized", label: "Sanitized", description: "Keep scheme, host, port, and path; drop query strings and fragments." },
          { value: "full", label: "Full", description: "Store the complete URL, including query strings and fragments." },
        ]}
      />
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
