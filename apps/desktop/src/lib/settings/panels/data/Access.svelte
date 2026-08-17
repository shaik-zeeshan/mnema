<script lang="ts">
  import { join, appConfigDir } from "@tauri-apps/api/path";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { tip } from "$lib/components/tooltip";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import {
    type BrokerGrant,
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
  const brokerHistoryLoading = $derived(cliAccess.brokerHistoryLoading);
  const brokerHistoryError = $derived(cliAccess.brokerHistoryError);
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

  // Blocking is reversible and costs the user nothing, so it stays unconfirmed.
  // ENABLING is the irreversible-ish direction: it restores a standing, no-expiry
  // permission at whatever scope the row still remembers (All retained history
  // survives a block), with no re-consent and no approval window — so that is the
  // side that asks.
  async function enableGrant(grant: BrokerGrant) {
    const ok = await confirm(
      `${grant.label} will be able to read ${formatGrantScope(grant.scope).toLowerCase()} again, with no expiry, until you block it.`,
      { title: "Restore CLI access?", kind: "warning", okLabel: "Restore access", cancelLabel: "Keep blocked" },
    );
    if (ok) await cliAccess.setGrantBlocked(grant, false);
  }

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

          <p class="group-label access-subhead">Tools with access</p>
          {#if brokerGrantLoading && brokerGrants.length === 0}
            <p class="group-hint">Loading tools…</p>
          {:else if brokerGrants.length > 0}
            <ul class="grant-list" aria-label="Tools with access">
              {#each brokerGrants as grant (grant.id)}
                {@const status = grantStatus(grant)}
                {@const saving = isGrantSaving(grant.id)}
                {@const statusTime = grantStatusLabel(grant, now)}
                <li class="grant-row" class:grant-row--inactive={status === "blocked"}>
                  <span class="grant-row__status grant-row__status--{status}" aria-hidden="true"></span>
                  <div class="grant-row__meta">
                    <span class="grant-row__head">
                      <span class="grant-row__name" use:tip={grant.label}>{grant.label}</span>
                      <!-- Blocked is a state the USER put the row in, not a fault:
                           a red dot on a dimmed row is this app's vocabulary for
                           broken, so the state reads as a badge on the name line. -->
                      {#if status === "blocked"}<span class="provider-row__tag grant-row__tag">Blocked</span>{/if}
                    </span>
                    <span class="grant-row__detail">
                      <span class="grant-row__scope">{formatGrantScope(grant.scope)}</span>
                      {#if statusTime}
                        <span class="grant-row__sep" aria-hidden="true">·</span>
                        <span use:tip={new Date(grant.blocked && grant.blockedAtUnixMs ? grant.blockedAtUnixMs : grant.lastUsedAtUnixMs).toLocaleString()}>{statusTime}</span>
                      {/if}
                    </span>
                  </div>
                  {#if status === "blocked"}
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      disabled={saving}
                      aria-busy={saving}
                      onclick={() => enableGrant(grant)}
                    >
                      {#if saving}<ButtonSpinner />Enabling…{:else}Enable{/if}
                    </button>
                  {:else}
                    <!-- Subtly-destructive ghost, not filled danger: blocking is the
                         safe, reversible direction and must not be the loudest
                         control in the list. -->
                    <button
                      class="btn btn--ghost btn--sm user-context-wipe__btn"
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

          <div class="group-label access-subhead access-subhead--split">
            <span>Recent activity</span>
            {#if brokerHistory.length > 0 || brokerHistoryError}
              <button class="btn btn--ghost btn--sm" type="button" onclick={revealAuditFile}>Reveal log in Finder</button>
            {/if}
          </div>
          {#if brokerHistory.length > 0}
            <ul class="activity-list" aria-label="Recent CLI activity">
              {#each brokerHistory as event, index (`${event.timestampUnixMs}-${index}`)}
                <li class="activity-row" class:activity-row--refused={!!formatOutcome(event.outcome)}>
                  <span class="activity-row__tool" use:tip={event.toolIdentity}>{event.toolIdentity}</span>
                  <code class="activity-row__command">{formatCommand(event.commandType)}</code>
                  <span class="activity-row__detail">{formatActivityDetail(event)}</span>
                  <span class="activity-row__time" use:tip={new Date(event.timestampUnixMs).toLocaleString()}>{formatGrantTime(event.timestampUnixMs, now)}</span>
                </li>
              {/each}
            </ul>
          {:else if brokerHistoryLoading}
            <p class="group-hint">Loading activity…</p>
          {:else if brokerHistoryError}
            <!-- An unreadable audit file must never render as "Nothing yet": on a
                 permission log, an empty list is a claim that nothing ran. -->
            <p class="error-text">{brokerHistoryError}</p>
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
     they did. These two labels are what make that order legible. Type comes from
     the shared `.group-label` (every other section label in Settings); this class
     only adds the layout. */
  .access-subhead {
    margin: 4px 0 0;
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
    padding: 8px 2px;
    border-bottom: 1px solid var(--app-border);
    color: var(--app-text-muted);
    font-size: 12px;
    line-height: 1.35;
    /* The result-count column is genuinely mixed ("88 results" / "Denied"), so it
       stays left-aligned — tabular figures are what stop the digits wobbling. */
    font-variant-numeric: tabular-nums;
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

  /* WHEN a tool read your history is audit evidence, not decoration:
     --app-text-faint is documented as sub-AA placeholder-only (1.57:1 on dark). */
  .activity-row__time {
    color: var(--app-text-subtle);
    font-size: 10px;
    white-space: nowrap;
  }
</style>
