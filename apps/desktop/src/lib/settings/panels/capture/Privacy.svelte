<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Choice from "$lib/components/Choice.svelte";
  import IconOff from "~icons/lucide/ban";
  import IconShield from "~icons/lucide/shield";
  import IconLink from "~icons/lucide/link";
  import IconLock from "~icons/lucide/lock";
  import IconInfo from "~icons/lucide/info";
  import IconCheck from "~icons/lucide/check";
  import IconExternal from "~icons/lucide/external-link";
  import AppPrivacyExclusion from "$lib/components/AppPrivacyExclusion.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const excl = c.appPrivacyExclusion;
  const gecko = c.geckoUrlAccess;

  const setBrowserUrlMode = (m: string) => c.setBrowserUrlMode(m);
  const pendingCount = $derived(excl.pendingRecommendedApps.length);
  const geckoNames = $derived(gecko.installedNames.length > 0 ? gecko.installedNames.join(" / ") : "Firefox / Zen");
</script>

<SettingGroup id="settings-section-privacy" title="Privacy">
  {#if excl.showSensitiveRecommendationPrompt}
    <div class="mx-notice priv-top" data-tone="info">
      <span class="mx-notice__icon"><IconShield width="15" height="15" /></span>
      <b class="mx-notice__title">{pendingCount} recommended app exclusion{pendingCount === 1 ? "" : "s"} pending</b>
      <p class="mx-notice__text">Password managers and Apple Passwords can be excluded from future screen capture.</p>
      <div class="mx-notice__acts">
        <button
          class="mx-btn mx-btn--sm"
          type="button"
          disabled={excl.commandInFlight || excl.promptActionInFlight}
          onclick={() => void excl.applyAllRecommendedPrivacyApps()}
        >
          Add exclusions
        </button>
        <button
          class="mx-btn mx-btn--ghost mx-btn--sm"
          type="button"
          disabled={excl.promptActionInFlight}
          onclick={() => void excl.dismissSensitiveRecommendationPrompt()}
        >
          Dismiss
        </button>
      </div>
    </div>
  {/if}

  <SettingRow
    label="Excluded Apps"
    description="Apps whose visible content is never recorded. Browsers, including private windows, are recorded unless the browser itself is excluded. Mnema does not detect password pages or password fields."
    full
  >
    {#snippet control()}
      <div class="priv-full">
        <AppPrivacyExclusion controller={excl} />
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Filter system audio" description="Also keep audio from excluded apps out of recordings.">
    {#snippet control()}
      <Switch
        checked={rec.draftFilterSystemAudio}
        onCheckedChange={(enabled) => excl.setFilterSystemAudio(enabled)}
        disabled={excl.commandInFlight}
        ariaLabel="Filter system audio"
      />
    {/snippet}
  </SettingRow>

  <SettingRow label="Capture frame context" description="Store app, window, and supported browser context with frames.">
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

  {#if gecko.installed && rec.draftMetadataEnabled && rec.draftBrowserUrlMode !== "off"}
    <SettingRow
      label="Browser URL access (Firefox / Zen)"
      description="Firefox and Zen expose no scriptable URL, so reading their page address needs the macOS Accessibility permission."
      full
    >
      {#snippet control()}
        <div class="priv-full">
          {#if gecko.trusted}
            <span class="mx-inline" data-tone="ok"><IconCheck width="13" height="13" />{geckoNames} · Accessibility granted</span>
          {:else}
            <div class="mx-notice" data-tone="warn">
              <span class="mx-notice__icon"><IconLock width="15" height="15" /></span>
              <b class="mx-notice__title">{geckoNames} · Accessibility not granted</b>
              <p class="mx-notice__text">Allow Mnema under Privacy &amp; Security → Accessibility to capture {geckoNames}’s page address.</p>
              <div class="mx-notice__acts">
                <button type="button" class="mx-btn mx-btn--sm" onclick={() => gecko.request()} disabled={gecko.requesting} aria-busy={gecko.requesting}>
                  {#if gecko.requesting}<ButtonSpinner />Requesting…{:else}Grant access{/if}
                </button>
                <button class="mx-btn mx-btn--sm" type="button" onclick={() => gecko.openSettings()}>
                  Open System Settings<IconExternal width="12" height="12" />
                </button>
                <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={() => gecko.recheck()} disabled={gecko.rechecking} aria-busy={gecko.rechecking}>
                  {#if gecko.rechecking}<ButtonSpinner />Checking…{:else}Recheck{/if}
                </button>
              </div>
            </div>
          {/if}
          {#if gecko.error}
            <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" />Browser URL access request failed: {gecko.error}</span>
          {/if}
        </div>
      {/snippet}
    </SettingRow>
  {/if}
</SettingGroup>

<style>
  .priv-top {
    margin: var(--s-3) 0;
  }

  .priv-full {
    display: flex;
    flex-direction: column;
    gap: var(--s-2);
    width: 100%;
    min-width: 0;
  }
</style>
