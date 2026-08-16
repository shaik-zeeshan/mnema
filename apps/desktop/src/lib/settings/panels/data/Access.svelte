<script lang="ts">
  import { join, appConfigDir } from "@tauri-apps/api/path";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { tip } from "$lib/components/tooltip";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import {
    grantStatus,
    formatGrantScope,
    grantStatusLabel,
    formatGrantTime,
    formatCommand,
    formatActivityDetail,
    formatOutcome,
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
  const isGrantSaving = (id: string) => cliAccess.isGrantSaving(id);
  const brokerGrantError = $derived(cliAccess.brokerGrantError);
  const brokerHistory = $derived(cliAccess.brokerHistory);
  const mnemaCliStatus = $derived(cliAccess.mnemaCliStatus);
  const mnemaCliLoading = $derived(cliAccess.mnemaCliLoading);
  const mnemaCliInstalling = $derived(cliAccess.mnemaCliInstalling);
  const mnemaCliError = $derived(cliAccess.mnemaCliError);

  const refresh = () => {
    void cliAccess.loadBrokerGrants();
    void cliAccess.loadBrokerHistory();
  };
  const loadMnemaCliStatus = () => cliAccess.loadMnemaCliStatus();
  const installMnemaCli = () => cliAccess.installMnemaCli();

  // Permissions have NO backend change event: one can be approved from the CLI
  // Access Request window, or blocked elsewhere, while this panel sits open.
  // This panel only mounts while the Data group is the active surface, so a lazy
  // poll + window-focus refetch keeps the list honest, and a ticking `now` keeps
  // the relative times from freezing at mount. All torn down on unmount.
  let now = $state(Date.now());
  $effect(() => {
    // First (and only) loader for the activity list — nothing loads it at
    // settings boot, since the audit file is only read by this panel.
    void cliAccess.loadBrokerHistory();
    const tick = setInterval(() => (now = Date.now()), 15000);
    const poll = setInterval(refresh, 30000);
    const onFocus = () => refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(tick);
      clearInterval(poll);
      window.removeEventListener("focus", onFocus);
    };
  });

  // The audit file lives beside the permission file in the app config dir; the
  // JS `appConfigDir()` resolves the same path the Rust side writes to.
  async function revealAuditFile() {
    try {
      await revealItemInDir(await join(await appConfigDir(), "broker-audit.json"));
    } catch (err) {
      console.error("[Access] reveal audit file failed", err);
    }
  }
</script>

<SettingGroup
  id="settings-section-access"
  title="Access"
  hint="Let local command-line tools read your Mnema history, one standing permission per tool."
>
  <SettingRow label="CLI Access" full divider={false}>
    {#snippet control()}
      <!-- The agent-access section is the `?focus=cliAccess` deeplink target: it
           must be focusable (the shell calls `.focus({ preventScroll: true })`)
           and carries the attention tint when the broker-authorization prompt is
           live. The bordered `.settings-stack` sub-block is the intended card-like
           surface for the CLI status + access list + activity. -->
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
              <p>Approve or deny it in the request window, then rerun the command if needed.</p>
            </div>
          {/if}
          <div class="privacy-disclosure">
            <p>CLI Access lets local tools read your searchable Mnema text — screen text, audio transcripts, and timeline results — plus the app and window each result came from and a sanitized host and path for web pages.</p>
            <p>It never returns media file paths, raw database rows, or full URLs with their query strings. A tool keeps its access until you block it here, and it lapses on its own after 30 days unused.</p>
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
              onclick={() => { refresh(); void loadMnemaCliStatus(); }}
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

          <p class="access-subhead">Tools with access</p>
          {#if brokerGrantLoading && brokerGrants.length === 0}
            <p class="group-hint">Loading tools…</p>
          {:else if brokerGrants.length > 0}
            <ul class="grant-list">
              {#each brokerGrants as grant (grant.id)}
                {@const status = grantStatus(grant)}
                {@const saving = isGrantSaving(grant.id)}
                <li class="grant-row" class:grant-row--inactive={status === "blocked"}>
                  <span class="grant-row__status grant-row__status--{status}" aria-hidden="true"></span>
                  <div class="grant-row__meta">
                    <span class="grant-row__name" use:tip={grant.label}>{grant.label}</span>
                    <span class="grant-row__detail">
                      <span class="grant-row__scope">{formatGrantScope(grant.scope)}</span>
                      <span class="grant-row__sep" aria-hidden="true">·</span>
                      <span use:tip={new Date(grant.blocked && grant.blockedAtUnixMs ? grant.blockedAtUnixMs : grant.lastUsedAtUnixMs).toLocaleString()}>{grantStatusLabel(grant, now)}</span>
                    </span>
                  </div>
                  {#if status === "blocked"}
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      disabled={saving}
                      aria-busy={saving}
                      onclick={() => cliAccess.setGrantBlocked(grant, false)}
                    >
                      {#if saving}<ButtonSpinner />Enabling…{:else}Enable{/if}
                    </button>
                  {:else}
                    <button
                      class="btn btn--danger btn--sm"
                      type="button"
                      disabled={saving}
                      aria-busy={saving}
                      onclick={() => cliAccess.setGrantBlocked(grant, true)}
                    >
                      {#if saving}<ButtonSpinner />Blocking…{:else}Block{/if}
                    </button>
                  {/if}
                </li>
              {/each}
            </ul>
          {:else}
            <p class="group-hint">No tools have access yet. Tools you approve will appear here.</p>
          {/if}

          <div class="access-subhead access-subhead--split">
            <span>Recent activity</span>
            {#if brokerHistory.length > 0}
              <button class="btn btn--ghost btn--sm" type="button" onclick={revealAuditFile}>Reveal log in Finder</button>
            {/if}
          </div>
          {#if brokerHistory.length > 0}
            <ul class="activity-list">
              {#each brokerHistory as event, index (`${event.timestampUnixMs}-${index}`)}
                <li class="activity-row" class:activity-row--refused={!!formatOutcome(event.outcome)}>
                  <span class="activity-row__tool" use:tip={event.toolIdentity}>{event.toolIdentity}</span>
                  <code class="activity-row__command">{formatCommand(event.commandType)}</code>
                  <span class="activity-row__detail">{formatActivityDetail(event)}</span>
                  <span class="activity-row__time" use:tip={new Date(event.timestampUnixMs).toLocaleString()}>{formatGrantTime(event.timestampUnixMs, now)}</span>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="group-hint">Nothing yet. Commands run by tools with access are recorded here.</p>
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

  /* The section reads top to bottom: install status → tools with access → what
     they did. These two labels are what make that order legible. */
  .access-subhead {
    margin: 4px 0 0;
    color: var(--app-text-muted);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }

  .access-subhead--split {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  /* One grid for the whole list with `subgrid` rows, so the command, outcome and
     time columns line up down the list instead of each row sizing its own. */
  .activity-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto auto;
  }

  /* ponytail: last ~20 rows, no paging or filtering. The whole file is one
     Reveal in Finder click away if someone needs more. */
  .activity-row {
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    gap: 8px;
    align-items: baseline;
    padding: 5px 2px;
    border-bottom: 1px solid var(--app-border-subtle, var(--app-border));
    color: var(--app-text-muted);
    font-size: 10px;
    line-height: 1.35;
  }

  .activity-row:last-child {
    border-bottom: none;
  }

  .activity-row__tool {
    overflow: hidden;
    color: var(--app-text);
    font-weight: 600;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .activity-row__command {
    color: var(--app-text);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10px;
  }

  .activity-row--refused .activity-row__detail {
    color: var(--app-warn);
    font-weight: 600;
  }

  .activity-row__time {
    color: var(--app-text-faint);
    white-space: nowrap;
  }
</style>
