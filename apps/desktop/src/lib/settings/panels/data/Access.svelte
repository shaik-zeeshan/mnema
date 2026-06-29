<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import {
    grantStatus,
    formatGrantScope,
    grantStatusLabel,
  } from "$lib/settings/state/cli-access.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";

  const c = getSettingsController();
  const cliAccess = c.cliAccess;

  const brokerGrants = $derived(cliAccess.brokerGrants);
  const brokerGrantLoading = $derived(cliAccess.brokerGrantLoading);
  const brokerGrantSaving = $derived(cliAccess.brokerGrantSaving);
  const isGrantRevoking = (id: string) => cliAccess.isGrantRevoking(id);
  const brokerGrantError = $derived(cliAccess.brokerGrantError);
  const mnemaCliStatus = $derived(cliAccess.mnemaCliStatus);
  const mnemaCliLoading = $derived(cliAccess.mnemaCliLoading);
  const mnemaCliInstalling = $derived(cliAccess.mnemaCliInstalling);
  const mnemaCliError = $derived(cliAccess.mnemaCliError);

  const loadBrokerGrants = () => cliAccess.loadBrokerGrants();
  const loadMnemaCliStatus = () => cliAccess.loadMnemaCliStatus();
  const installMnemaCli = () => cliAccess.installMnemaCli();
  const revokeAgentBrokerGrant = (id: string) => cliAccess.revokeAgentBrokerGrant(id);
</script>

<SettingGroup
  id="settings-section-access"
  title="Access"
  hint="Let local tools request time-bounded, redacted access to your Mnema history."
>
  <SettingRow label="CLI Access" full divider={false}>
    {#snippet control()}
      <!-- The agent-access section is the `?focus=cliAccess` deeplink target: it
           must be focusable (the shell calls `.focus({ preventScroll: true })`)
           and carries the attention tint when the broker-authorization prompt is
           live. The bordered `.settings-stack` sub-block is the intended card-like
           surface for the CLI status + grant list. -->
      <div
        class:settings-group--attention={c.brokerAuthorizationPromptVisible}
        bind:this={c.agentAccessSection}
        class="agent-access"
        tabindex="-1"
      >
        <div class="settings-stack">
          {#if c.brokerAuthorizationPromptVisible}
            <div class="agent-access-callout" role="status">
              <strong>CLI access request</strong>
              <p>Review the request window or native prompt, then rerun the CLI command if needed.</p>
            </div>
          {/if}
          <div class="privacy-disclosure">
            <p>CLI Access lets local tools request time-bounded access to searchable Mnema text, including screen text, audio transcripts, and timeline results.</p>
            <p>CLI output does not include media paths, raw database rows, app/window titles, browser URLs, or deep-link URLs.</p>
          </div>
          {#if mnemaCliStatus}
            {#if !mnemaCliStatus.installed}
              <p class="group-hint group-hint--warn">mnema is not installed at {mnemaCliStatus.installPath} — install the CLI before local tools can request access.</p>
            {:else if !mnemaCliStatus.installDirInPath}
              <p class="group-hint group-hint--warn">mnema is installed at {mnemaCliStatus.installPath}, but {mnemaCliStatus.installDir} is not in PATH for this app session.</p>
            {:else}
              <p class="group-hint">mnema installed at {mnemaCliStatus.installPath}.</p>
            {/if}
          {/if}
          <div class="row-actions">
            <button class="btn btn--ghost btn--sm" type="button" disabled={mnemaCliInstalling || mnemaCliLoading} aria-busy={mnemaCliInstalling} onclick={installMnemaCli}>
              {#if mnemaCliInstalling}<ButtonSpinner />Installing…{:else}{mnemaCliStatus?.installed ? "Reinstall CLI" : "Install CLI"}{/if}
            </button>
            <ReloadButton
              onclick={() => { void loadBrokerGrants(); void loadMnemaCliStatus(); }}
              busy={brokerGrantLoading || mnemaCliLoading}
              disabled={brokerGrantSaving}
              title="Refresh"
              label="Refresh CLI access status"
            />
          </div>
          {#if mnemaCliError}
            <p class="error-text">{mnemaCliError}</p>
          {/if}
          {#if brokerGrantError}
            <p class="error-text">{brokerGrantError}</p>
          {/if}
          {#if brokerGrantLoading && brokerGrants.length === 0}
            <p class="group-hint">Loading grants…</p>
          {:else if brokerGrants.length > 0}
            <ul class="grant-list">
              {#each brokerGrants as grant (grant.id)}
                {@const status = grantStatus(grant)}
                <li class="grant-row" class:grant-row--inactive={status !== "active"}>
                  <span class="grant-row__status grant-row__status--{status}" aria-hidden="true"></span>
                  <div class="grant-row__meta">
                    <span class="grant-row__name" title={grant.label}>{grant.label}</span>
                    <span class="grant-row__detail">
                      <span class="grant-row__scope">{formatGrantScope(grant.scope)}</span>
                      <span class="grant-row__sep" aria-hidden="true">·</span>
                      <span title={new Date(grant.expiresAtUnixMs).toLocaleString()}>{grantStatusLabel(grant)}</span>
                    </span>
                  </div>
                  <button
                    class="btn btn--danger btn--sm"
                    type="button"
                    disabled={isGrantRevoking(grant.id) || status !== "active"}
                    aria-busy={isGrantRevoking(grant.id)}
                    onclick={() => revokeAgentBrokerGrant(grant.id)}
                  >
                    {#if isGrantRevoking(grant.id)}<ButtonSpinner />Revoking…{:else}Revoke{/if}
                  </button>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="group-hint">No CLI Access grants yet. Tools you approve will appear here.</p>
          {/if}
        </div>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Focus target wrapper for the `?focus=cliAccess` deeplink — `bind:this`
     needs a real element, and `tabindex=-1` makes `.focus()` land here without
     a visible outline. The attention tint lives on the global
     `.settings-group--attention .settings-stack` rule, so the wrapper just
     carries that toggle class. */
  .agent-access {
    width: 100%;
  }

  .agent-access:focus {
    outline: none;
  }
</style>
