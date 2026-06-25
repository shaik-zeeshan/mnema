<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import {
    ABOUT_REPO_URL,
    ABOUT_RELEASES_URL,
    appUpdateStateLabel,
    appUpdateStatusMessage,
    updateChannelLabel,
    platformLabel,
    formatUpdateDate,
    formatCheckedAt,
    appUpdateProgressText,
    appUpdateProgressPercent,
  } from "$lib/settings/state/about.svelte";
  import RadioGroup from "$lib/components/RadioGroup.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import type { AppUpdateChannel, AppUpdateStatus } from "$lib/types";

  const c = getSettingsController();
  const about = c.about;

  // ─── c.about read aliases ───────────────────────────────────────────────
  const appUpdateStatus = $derived(about.appUpdateStatus);
  const checkingAppUpdate = $derived(about.checkingAppUpdate);
  const switchingAppUpdateChannel = $derived(about.switchingAppUpdateChannel);
  const installingAppUpdate = $derived(about.installingAppUpdate);
  const restartingAfterUpdate = $derived(about.restartingAfterUpdate);
  const aboutDetailsCopied = $derived(about.aboutDetailsCopied);
  const aboutActionError = $derived(about.aboutActionError);
  const thirdPartyNotices = $derived(about.thirdPartyNotices);
  const loadingThirdPartyNotices = $derived(about.loadingThirdPartyNotices);
  const thirdPartyNoticesError = $derived(about.thirdPartyNoticesError);
  const thirdPartyNoticesCopied = $derived(about.thirdPartyNoticesCopied);
  const thirdPartyNoticeGroups = $derived(about.thirdPartyNoticeGroups);

  // NOTE: `appUpdateActionError` and `previewConfirmationVisible` are written
  // by the markup, so they reference the store directly (`about.X`, which has
  // setters) rather than read-only `$derived` aliases.

  // ─── canInstall/canRestart are getter PROPERTIES on the store, but the
  //     markup calls them as arg-ignoring FUNCTIONS — recreate verbatim. ──
  const canInstallAppUpdate = (_s: AppUpdateStatus | null) => about.canInstallAppUpdate;
  const canRestartAfterUpdate = (_s: AppUpdateStatus | null) => about.canRestartAfterUpdate;

  // ─── method wrappers ────────────────────────────────────────────────────
  const checkForAppUpdate = () => about.checkForAppUpdate();
  const installAppUpdate = () => about.installAppUpdate();
  const chooseAppUpdateChannel = (ch: AppUpdateChannel) => about.chooseAppUpdateChannel(ch);
  const useAppUpdateChannel = (ch: AppUpdateChannel) => about.useAppUpdateChannel(ch);
  const restartAfterAppUpdate = () => about.restartAfterAppUpdate();
  const copyAboutDetails = () => about.copyAboutDetails();
  const openExternalUrl = (u: string) => about.openExternalUrl(u);
  const copyThirdPartyNotices = () => about.copyThirdPartyNotices();
  // loadAppUpdateStatus / loadThirdPartyNotices are driven by the shell mount
  // effects, not this panel's markup, so no panel-local wrappers are needed.

  const checkDisabled = $derived(
    checkingAppUpdate ||
      switchingAppUpdateChannel ||
      installingAppUpdate ||
      appUpdateStatus?.state === "downloading" ||
      appUpdateStatus?.state === "installing" ||
      appUpdateStatus?.state === "restartRequired",
  );
</script>

<SettingGroup id="settings-section-about" title="About" hint="Version, build details, and the projects Mnema is built on.">
  <SettingRow label="Mnema" description="Your memory, on rewind. Mnema records your screen so you can scrub back to anything you've seen: searchable, local, and yours." full>
    {#snippet control()}
      <div class="about-identity">
        <div class="about-id__mark">
          {#if appUpdateStatus?.app.version}
            <span class="about-id__version">v{appUpdateStatus.app.version}</span>
          {:else}
            <span class="about-id__version about-id__version--pending">checking…</span>
          {/if}
          <span class="badge badge--neutral badge--sm about-id__channel">
            {updateChannelLabel(appUpdateStatus?.channel)} channel
          </span>
        </div>
        <dl class="about-meta">
          <div class="about-meta__row">
            <dt>Platform</dt>
            <dd>{platformLabel(appUpdateStatus)}</dd>
          </div>
          <div class="about-meta__row">
            <dt>Identifier</dt>
            <dd>{appUpdateStatus?.app.identifier ?? "Unknown"}</dd>
          </div>
          <div class="about-meta__row">
            <dt>License</dt>
            <dd>MIT</dd>
          </div>
        </dl>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Links" description="Browse the source or read what changed in each release.">
    {#snippet control()}
      <div class="about-links">
        <button type="button" class="about-link" onclick={() => openExternalUrl(ABOUT_REPO_URL)}>
          Source<span class="about-link__arrow" aria-hidden="true">↗</span>
        </button>
        <button type="button" class="about-link" onclick={() => openExternalUrl(ABOUT_RELEASES_URL)}>
          Release notes<span class="about-link__arrow" aria-hidden="true">↗</span>
        </button>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Copy details" description="Copy version and build details to the clipboard." divider={false}>
    {#snippet control()}
      <button
        type="button"
        class="btn btn--ghost btn--sm"
        onclick={copyAboutDetails}
        aria-label="Copy version and build details to the clipboard"
      >
        {aboutDetailsCopied ? "Copied" : "Copy details"}
      </button>
    {/snippet}
  </SettingRow>
</SettingGroup>

{#if aboutActionError}
  <p class="error-text about-error" role="alert">{aboutActionError}</p>
{/if}

<SettingGroup title="Updates" hint="Mnema checks the selected channel at startup after onboarding.">
  <SettingRow label="Update channel" description={switchingAppUpdateChannel ? "Saving channel and checking for updates." : `Current channel: ${updateChannelLabel(appUpdateStatus?.channel)}. Switching channels checks immediately.`} full>
    {#snippet control()}
      <RadioGroup
        value={appUpdateStatus?.channel === "preview" ? "preview" : "stable"}
        onValueChange={(v) => chooseAppUpdateChannel(v as AppUpdateChannel)}
        disabled={switchingAppUpdateChannel || installingAppUpdate}
        label="Update channel"
        options={[
          { value: "stable", label: "Stable", description: "Published releases" },
          { value: "preview", label: "Preview", description: "Opt-in prereleases" },
        ]}
      />
    {/snippet}
  </SettingRow>

  {#if about.previewConfirmationVisible}
    <SettingRow label="Confirm preview channel" description="Preview builds may be less stable and may show macOS security warnings until Developer ID signing and notarization are available." warn full>
      {#snippet control()}
        <div class="preview-warning" role="alert">
          <div class="row-actions">
            <button class="btn btn--primary btn--sm" type="button" onclick={() => void useAppUpdateChannel("preview")} disabled={switchingAppUpdateChannel}>
              Use Preview Updates
            </button>
            <button class="btn btn--ghost btn--sm" type="button" onclick={() => { about.previewConfirmationVisible = false; }}>
              Keep Stable
            </button>
          </div>
        </div>
      {/snippet}
    </SettingRow>
  {/if}

  <SettingRow label="Status" description="Check for a new build and install it from here." full divider={false}>
    {#snippet control()}
      <div class="about-update">
        <div class="about-update__head">
          <span class="badge badge--neutral badge--sm">{appUpdateStateLabel(appUpdateStatus)}</span>
          <button class="btn btn--primary btn--sm" onclick={checkForAppUpdate} disabled={checkDisabled}>
            {checkingAppUpdate || appUpdateStatus?.state === "checking" ? "Checking" : "Check for Updates"}
          </button>
        </div>

        <div class="update-status-panel" class:update-status-panel--error={appUpdateStatus?.state === "failed" || appUpdateStatus?.state === "incompatible"}>
          <div class="update-status-panel__main">
            <div class="update-status-panel__headline">
              {#if appUpdateStatus?.update}
                <strong>Version {appUpdateStatus.update.version}</strong>
              {:else}
                <strong>{appUpdateStatus?.app.version ?? "Mnema"}</strong>
              {/if}
            </div>
            <p>{appUpdateStatusMessage(appUpdateStatus)}</p>
            <span class="update-status-panel__meta">Last checked: {formatCheckedAt(appUpdateStatus?.lastCheckedAtUnixMs)}</span>
          </div>

          {#if appUpdateStatus?.update?.date}
            <span class="update-status-panel__date">{formatUpdateDate(appUpdateStatus.update.date)}</span>
          {/if}
        </div>

        {#if appUpdateStatus?.progress}
          <div class="download-progress" aria-live="polite">
            <div class="download-progress__bar">
              <span style={`width: ${appUpdateProgressPercent(appUpdateStatus)}%`}></span>
            </div>
            <p class="group-hint">{appUpdateProgressText(appUpdateStatus)}</p>
          </div>
        {/if}

        {#if appUpdateStatus?.update?.notes}
          <div class="release-notes">
            <span class="group-label">Release notes</span>
            <p>{appUpdateStatus.update.notes}</p>
          </div>
        {/if}

        <div class="row-actions">
          {#if appUpdateStatus?.state === "restartRequired"}
            <button class="btn btn--primary" type="button" onclick={restartAfterAppUpdate} disabled={!canRestartAfterUpdate(appUpdateStatus)}>
              {restartingAfterUpdate ? "Restarting" : "Restart to Update"}
            </button>
          {:else}
            <button class="btn btn--primary" type="button" onclick={installAppUpdate} disabled={!canInstallAppUpdate(appUpdateStatus)}>
              {installingAppUpdate || appUpdateStatus?.state === "downloading" || appUpdateStatus?.state === "installing" ? "Installing" : "Install Update"}
            </button>
          {/if}
          {#if appUpdateStatus?.recordingActive && appUpdateStatus?.update}
            <span class="action-hint action-hint--warn">Stop recording to install this update.</span>
          {/if}
        </div>

        {#if about.appUpdateActionError}
          <div class="inline-error">
            <span class="inline-error__icon">⚠</span>
            <span class="inline-error__msg">{about.appUpdateActionError}</span>
            <button class="btn btn--ghost btn--sm" onclick={() => about.appUpdateActionError = null}>×</button>
          </div>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup title="Acknowledgements" hint="Mnema's on-device models are built on these open projects. Each is credited under its license.">
  <SettingRow label="Third-party notices" description="The full attribution list for the bundled open-source components." full divider={false}>
    {#snippet control()}
      <div class="about-notices">
        <div class="about-notices__head">
          <button
            type="button"
            class="btn btn--ghost btn--sm"
            onclick={copyThirdPartyNotices}
            disabled={!thirdPartyNotices || loadingThirdPartyNotices}
            aria-label="Copy the full third-party notices to the clipboard"
          >
            {thirdPartyNoticesCopied ? "Copied" : "Copy notices"}
          </button>
        </div>

        {#if loadingThirdPartyNotices && !thirdPartyNotices}
          <p class="group-hint">Loading notices…</p>
        {:else if thirdPartyNoticeGroups.length > 0}
          <div class="notice-groups">
            {#each thirdPartyNoticeGroups as group (group.kind)}
              <div class="notice-group">
                <span class="group-label">{group.kind}</span>
                <ul class="notice-list">
                  {#each group.entries as entry (entry.component)}
                    <li class="notice-item">
                      <div class="notice-item__main">
                        <span class="notice-item__name">{entry.displayName}</span>
                        {#if entry.license}
                          <span class="notice-item__license">{entry.license}</span>
                        {/if}
                      </div>
                      {#if entry.sourceUrl}
                        <button
                          type="button"
                          class="notice-item__source"
                          onclick={() => openExternalUrl(entry.sourceUrl ?? "")}
                          title={entry.sourceUrl}
                        >
                          {entry.sourceUrl}<span class="about-link__arrow" aria-hidden="true">↗</span>
                        </button>
                      {/if}
                    </li>
                  {/each}
                </ul>
              </div>
            {/each}
          </div>
        {:else if !loadingThirdPartyNotices}
          <p class="group-hint">No third-party notices to show.</p>
        {/if}

        {#if thirdPartyNoticesError}
          <p class="error-text about-error" role="alert">{thirdPartyNoticesError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Identity block laid into a full-width row's control slot. */
  .about-identity {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 100%;
  }

  /* The check-for-updates action sits beside the state badge in the
     status sub-block head; the row label carries the "Status" heading. */
  .about-update,
  .about-notices {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }

  .about-update__head,
  .about-notices__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
</style>
