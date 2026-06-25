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
      <Switch bind:checked={rec.draftMetadataEnabled} ariaLabel="Capture frame context" />
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

  {#if c.geckoUrlAccess.installed && rec.draftMetadataEnabled && rec.draftBrowserUrlMode !== "off"}
    {@const gecko = c.geckoUrlAccess}
    <SettingRow
      label="Browser URL access (Firefox / Zen)"
      description="Firefox and Zen have no scriptable URL like Chrome/Safari; reading their page address needs the macOS Accessibility permission."
      full
    >
      {#snippet control()}
        <div class="gecko-access">
          <div class="permission-callout" class:permission-callout--ok={gecko.trusted}>
            <div class="permission-callout__copy">
              <span class="permission-callout__eyebrow">Accessibility</span>
              <strong>
                {gecko.installedNames.length > 0 ? gecko.installedNames.join(" / ") : "Firefox / Zen"}
                · {gecko.trusted ? "Granted" : "Not granted"}
              </strong>
              <p>Lets Mnema capture the page address for Firefox and Zen. Enable Mnema under Privacy &amp; Security → Accessibility.</p>
            </div>
            {#if !gecko.trusted}
              <button class="btn btn--ghost" onclick={() => gecko.request()} disabled={gecko.requesting}>
                {gecko.requesting ? "Requesting" : "Grant access"}
              </button>
            {/if}
          </div>
          {#if !gecko.trusted}
            <div class="row-actions">
              <button class="btn btn--ghost btn--sm" type="button" onclick={() => gecko.openSettings()}>
                Open System Settings
              </button>
              <button class="btn btn--ghost btn--sm" type="button" onclick={() => gecko.recheck()} disabled={gecko.rechecking}>
                {gecko.rechecking ? "Checking" : "Recheck"}
              </button>
            </div>
          {/if}
          {#if gecko.error}
            <p class="group-hint group-hint--warn">Browser URL access request failed: {gecko.error}</p>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {/if}

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

  /* Optional Gecko browser-URL access — stack the callout, action buttons, and
     any error vertically within the full-width control cell. */
  .gecko-access {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
    min-width: 0;
  }
</style>
