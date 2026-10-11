<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Choice from "$lib/components/Choice.svelte";
  import IconOff from "~icons/lucide/ban";
  import IconShield from "~icons/lucide/shield";
  import IconLink from "~icons/lucide/link";
  import IconLock from "~icons/lucide/lock";
  import IconInfo from "~icons/lucide/info";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";

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
    description="How much of a captured browser URL is stored with the frame. Full asks first."
    disabled={!rec.draftMetadataEnabled}
    full
  >
    {#snippet control()}
      <Choice
        value={rec.draftBrowserUrlMode}
        onValueChange={setBrowserUrlMode}
        disabled={!rec.draftMetadataEnabled}
        ariaLabel="Browser URL mode"
        options={[
          { value: "off", label: "Off", icon: IconOff, description: "Don't store browser URLs with captured frames." },
          { value: "sanitized", label: "Sanitized", tag: "default", icon: IconShield, description: "Keep scheme, host, port and path; drop query strings and fragments." },
          { value: "full", label: "Full", icon: IconLink, description: "The complete URL, including query strings and fragments." },
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
          <div class="mx-notice" data-tone={gecko.trusted ? undefined : "warn"}>
            <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
            <b class="mx-notice__title">
              {gecko.installedNames.length > 0 ? gecko.installedNames.join(" / ") : "Firefox / Zen"}
              · Accessibility {gecko.trusted ? "granted" : "not granted"}
            </b>
            <p class="mx-notice__text">Allow Mnema under Privacy &amp; Security → Accessibility to capture the page address for Firefox and Zen.</p>
            {#if !gecko.trusted}
              <div class="mx-notice__acts">
                <button type="button" class="mx-btn mx-btn--sm" onclick={() => gecko.request()} disabled={gecko.requesting} aria-busy={gecko.requesting}>
                  {#if gecko.requesting}<ButtonSpinner />Requesting…{:else}Grant access{/if}
                </button>
                <button class="mx-btn mx-btn--sm" type="button" onclick={() => gecko.openSettings()}>
                  Open System Settings
                </button>
                <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => gecko.recheck()} disabled={gecko.rechecking} aria-busy={gecko.rechecking}>
                  {#if gecko.rechecking}<ButtonSpinner />Checking…{:else}Recheck{/if}
                </button>
              </div>
            {/if}
          </div>
          {#if gecko.error}
            <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />Browser URL access request failed: {gecko.error}</span>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow
    label="Excluded Apps"
    description="Apps whose visible content is never recorded."
    full
  >
    {#snippet control()}
      <div class="exclusion-cell" class:exclusion-cell--open={appPrivacyExclusion.comboboxOpen}>
        <AppPrivacyExclusion controller={appPrivacyExclusion} />
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Filter system audio"
    description="Also keep audio from excluded apps out of recordings."
    divider={false}
  >
    {#snippet control()}
      <Switch
        checked={rec.draftFilterSystemAudio}
        onCheckedChange={(enabled) => appPrivacyExclusion.setFilterSystemAudio(enabled)}
        disabled={appPrivacyExclusion.commandInFlight}
        ariaLabel="Filter system audio"
      />
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
