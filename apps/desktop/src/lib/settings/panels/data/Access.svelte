<script lang="ts">
  import { join, appConfigDir } from "@tauri-apps/api/path";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { tip } from "$lib/components/tooltip";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import {
    type BrokerGrant,
    IDENTITY_SOURCE,
    formatScopeChip,
    formatGrantScopeProse,
    formatGrantTime,
    formatCommand,
    formatActivityDetail,
    formatOutcome,
    grantLapse,
  } from "$lib/settings/state/cli-access.svelte";
  import { describeError } from "$lib/settings/state/format";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import IconTerminal from "~icons/lucide/square-terminal";
  import IconBan from "~icons/lucide/ban";
  import IconFolder from "~icons/lucide/folder";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";

  const c = getSettingsController();
  const cliAccess = c.cliAccess;

  const grants = $derived(cliAccess.brokerGrants);
  const active = $derived(grants.filter((g) => !g.blocked));
  const blocked = $derived(grants.filter((g) => g.blocked));
  const grantError = $derived(cliAccess.brokerGrantError);
  const history = $derived(cliAccess.brokerHistory);
  const historyError = $derived(cliAccess.brokerHistoryError);
  const cli = $derived(cliAccess.mnemaCliStatus);
  const cliError = $derived(cliAccess.mnemaCliError);
  const installing = $derived(cliAccess.mnemaCliInstalling);

  const refresh = () => {
    void cliAccess.loadBrokerGrants();
    void cliAccess.loadBrokerHistory();
  };

  // Permissions have NO backend change event: one can be approved from the CLI
  // Access Request window, or blocked elsewhere, while this panel sits open.
  // This panel only mounts while the Data group is the active surface, so a lazy
  // poll + window-focus refetch keeps the list honest, and a ticking `now` keeps
  // the relative times and lapse meters from freezing at mount.
  let now = $state(Date.now());
  $effect(() => {
    // First (and only) loader for the activity list — nothing loads it at
    // settings boot, since the audit file is only read by this panel.
    void cliAccess.loadBrokerHistory();
    const tick = setInterval(() => (now = Date.now()), 15000);
    const poll = setInterval(refresh, 30000);
    window.addEventListener("focus", refresh);
    return () => {
      clearInterval(tick);
      clearInterval(poll);
      window.removeEventListener("focus", refresh);
    };
  });

  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  // A row blocked from its very first request was never granted anything:
  // `lastUsedAtUnixMs` is 0 and its scope is a placeholder. Unblocking it
  // deletes the row, so the tool's next request opens the approval window.
  const neverGranted = (g: BrokerGrant) => g.lastUsedAtUnixMs === 0;

  // Blocking is reversible and costs nothing, so it stays unconfirmed. Unblocking
  // a once-granted row restores a standing permission at the scope it remembers,
  // with no approval window — so that side asks. A never-granted row grants
  // nothing on unblock (it asks again), so it needs no confirm.
  async function unblock(grant: BrokerGrant) {
    if (!neverGranted(grant)) {
      const ok = await confirm(
        `${grant.label} will be able to read ${formatGrantScopeProse(grant.scope)} again, with no expiry, until you block it.`,
        { title: "Enable CLI access again?", kind: "warning", okLabel: "Enable access", cancelLabel: "Keep blocked" },
      );
      if (!ok) return;
    }
    await cliAccess.setGrantBlocked(grant, false);
  }

  // The audit file lives beside the permission file in the app config dir; the
  // JS `appConfigDir()` resolves the same path the Rust side writes to.
  let revealError = $state<string | null>(null);
  async function revealAuditFile() {
    revealError = null;
    try {
      await revealItemInDir(await join(await appConfigDir(), "broker-audit.json"));
    } catch (err) {
      // `revealItemInDir` rejects when broker-audit.json does not exist yet.
      revealError = describeError(err);
    }
  }
</script>

{#snippet sourceChip(g: BrokerGrant)}
  {@const s = IDENTITY_SOURCE[g.identitySource] ?? IDENTITY_SOURCE.defaulted}
  <!-- Reference, not a trust verdict: neutral here (the request window warn-tints it). -->
  <span class="mx-chip" use:tip={s.detail}>{s.label}</span>
{/snippet}

{#snippet toolRow(g: BrokerGrant)}
  {@const saving = cliAccess.isGrantSaving(g.id)}
  {#if g.blocked}
    <div class="ac-tool is-blocked">
      <span class="ac-glyph"><IconBan width="15" height="15" /></span>
      <div class="ac-tool__main">
        <div class="ac-tool__name"><span class="ac-tool__label" use:tip={g.label}>{g.label}</span>{@render sourceChip(g)}</div>
        <div class="ac-tool__meta">
          {#if neverGranted(g)}Never allowed{:else}Remembers <span class="mx-chip">{formatScopeChip(g.scope)}</span>{/if}
          {#if g.blockedAtUnixMs}<i>·</i> <span use:tip={new Date(g.blockedAtUnixMs).toLocaleString()}>blocked {day(g.blockedAtUnixMs)}</span>{/if}
          {#if neverGranted(g)}<i>·</i> will ask again if unblocked{/if}
        </div>
      </div>
      <div class="ac-idle ac-idle--blocked"><b>Never lapses</b></div>
      <button class="mx-btn mx-btn--sm" type="button" disabled={saving} aria-busy={saving} onclick={() => unblock(g)}>
        {#if saving}<span class="mx-spin mx-spin--sm"></span>Unblocking…{:else}{neverGranted(g) ? "Unblock" : "Unblock…"}{/if}
      </button>
    </div>
  {:else}
    {@const lapse = grantLapse(g, now)}
    {@const cmd = cliAccess.lastCommands.get(g.normalizedLabel)}
    <div class="ac-tool">
      <span class="ac-glyph"><IconTerminal width="15" height="15" /></span>
      <div class="ac-tool__main">
        <div class="ac-tool__name"><span class="ac-tool__label" use:tip={g.label}>{g.label}</span>{@render sourceChip(g)}</div>
        <div class="ac-tool__meta">
          <span class="mx-chip">{formatScopeChip(g.scope)}</span>
          <i>·</i> <span use:tip={new Date(g.lastUsedAtUnixMs).toLocaleString()}>used {formatGrantTime(g.lastUsedAtUnixMs, now)}</span>
          {#if cmd}<i>·</i> last ran <code>{cmd}</code>{/if}
        </div>
      </div>
      <div class="ac-idle" class:is-soon={lapse.soon} use:tip={`Lapses ${day(lapse.atMs)} if unused — the next request asks again`}>
        <b>{lapse.label}</b><span class="ac-idle__bar" style:--left="{lapse.fraction * 100}%"><i></i></span>
      </div>
      <button
        class="mx-btn mx-btn--ghost mx-btn--danger mx-btn--sm"
        type="button"
        disabled={saving}
        aria-busy={saving}
        use:tip={"Revoke and refuse its future requests"}
        onclick={() => cliAccess.setGrantBlocked(g, true)}
      >
        {#if saving}<span class="mx-spin mx-spin--sm"></span>Blocking…{:else}Block{/if}
      </button>
    </div>
  {/if}
{/snippet}

<!-- The `?focus=cliAccess` deeplink target: the shell scrolls to it and calls
     `.focus({ preventScroll: true })`, so it must be a focusable element. -->
<div bind:this={c.agentAccessSection} class="agent-access" tabindex="-1">
  <SettingGroup id="settings-section-access" title="CLI Access" hint="One standing permission per tool. Unused for 30 days, it lapses." bare>
    <p class="ac-lede">Local tools and AI agents can read your searchable Mnema text — screen text, transcripts and timeline results, with the app and window each came from. Never media files, raw database rows or full web addresses.</p>
    <div class="ac-rows">
      <SettingRow label="mnema command">
        {#snippet below()}
          {#if installing}
            <p class="ac-desc">Linking <span class="ac-path">{cli?.installPath ?? "mnema"}</span>…</p>
          {:else if cliError}
            <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{cliError}</span>
          {:else if cli && !cli.installed}
            <span class="mx-inline" data-tone="warn"><IconAlert width="13" height="13" />Not installed at <span class="ac-path">{cli.installPath}</span> — install it before tools can ask for access.</span>
          {:else if cli && !cli.installDirInPath}
            <span class="mx-inline" data-tone="warn"><IconAlert width="13" height="13" />Installed at <span class="ac-path">{cli.installPath}</span>, but <span class="ac-path">{cli.installDir}</span> isn’t on PATH for this session.</span>
          {:else if cli}
            <p class="ac-desc">Installed at <span class="ac-path">{cli.installPath}</span>.</p>
          {:else}
            <p class="ac-desc">Checking the install…</p>
          {/if}
        {/snippet}
        {#snippet control()}
          <button class="mx-btn" type="button" disabled={installing || cliAccess.mnemaCliLoading} aria-busy={installing} onclick={() => cliAccess.installMnemaCli()}>
            {#if installing}<span class="mx-spin mx-spin--sm"></span>Installing…{:else}{#if cliError}<IconRetry width="13" height="13" />{/if}{cli?.installed ? "Reinstall CLI" : "Install CLI"}{/if}
          </button>
        {/snippet}
      </SettingRow>
    </div>

    {#if grants.length === 0}
      <div class="ac-sec">
        <div class="ac-sec__head"><b>Tools with access</b></div>
        <div class="ac-list">
          {#if grantError}
            <div class="mx-empty mx-empty--compact" data-tone="danger">
              <span class="mx-empty__glyph"><IconAlert width="14" height="14" /></span>
              <b class="mx-empty__title">Couldn’t load CLI Access permissions</b>
              <p class="mx-empty__text">{grantError} Tools keep the access they had; this list refreshes every 30 s.</p>
              <div class="mx-empty__acts">
                <button class="mx-btn mx-btn--sm" type="button" disabled={cliAccess.brokerGrantLoading} aria-busy={cliAccess.brokerGrantLoading} onclick={refresh}><IconRetry width="12" height="12" />Retry</button>
              </div>
            </div>
          {:else if cliAccess.brokerGrantLoading}
            <div aria-busy="true">
              <span class="mx-sr">Loading tools…</span>
              {#each [0, 1] as i (i)}
                <div class="mx-skel-row"><div class="mx-skel mx-skel--circle"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
              {/each}
            </div>
          {:else}
            <div class="mx-empty mx-empty--compact">
              <span class="mx-empty__glyph"><IconTerminal width="14" height="14" /></span>
              <b class="mx-empty__title">No tool has access</b>
              <p class="mx-empty__text">The first time an agent runs a command like <code>mnema search</code>, a window asks you. Tools you allow — or block — show up here.</p>
            </div>
          {/if}
        </div>
      </div>
    {:else}
      {#if grantError}
        <!-- Rows on screen are from the last good read (or a write just failed):
             say so rather than painting them as current. -->
        <span class="mx-inline ac-stale" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{grantError}</span>
      {/if}
      {#if active.length > 0}
        <div class="ac-sec">
          <div class="ac-sec__head"><b>Tools with access <span class="num mx-label">{active.length}</span></b><p class="ac-sec__hint">Each lapses 30 days after its last use.</p></div>
          <div class="ac-list">{#each active as g (g.id)}{@render toolRow(g)}{/each}</div>
        </div>
      {/if}
      {#if blocked.length > 0}
        <div class="ac-sec">
          <div class="ac-sec__head"><b>Blocked <span class="num mx-label">{blocked.length}</span></b><p class="ac-sec__hint">Refused before any window opens. Never lapses.</p></div>
          <div class="ac-list">{#each blocked as g (g.id)}{@render toolRow(g)}{/each}</div>
        </div>
      {/if}
    {/if}

    <div class="ac-sec">
      <div class="ac-sec__head">
        <b>Recent activity</b>
        {#if history.length > 0 || historyError}
          <button class="mx-btn mx-btn--ghost mx-btn--sm ac-sec__act" type="button" onclick={revealAuditFile}><IconFolder width="13" height="13" />Reveal log in Finder</button>
        {/if}
      </div>
      {#if historyError}
        <!-- Unconditional: a failed refresh after a good load leaves rows on
             screen, and painting stale rows as current is the same lie as "Nothing yet". -->
        <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{historyError}</span>
      {/if}
      {#if revealError}
        <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" />{revealError}</span>
      {/if}
      {#if history.length > 0}
        <!-- ponytail: last 20 rows, no paging. The whole file is one Reveal away. -->
        <ul class="ac-log" aria-label="Recent CLI activity">
          {#each history as event, index (`${event.timestampUnixMs}-${index}`)}
            <li>
              <b use:tip={event.toolIdentity}>{event.toolIdentity}</b>
              <code>{formatCommand(event.commandType)}</code>
              <span class:is-refused={!!formatOutcome(event.outcome)}>{formatActivityDetail(event)}</span>
              <span class="num" use:tip={new Date(event.timestampUnixMs).toLocaleString()}>{formatGrantTime(event.timestampUnixMs, now)}</span>
            </li>
          {/each}
        </ul>
      {:else if cliAccess.brokerHistoryLoading}
        <div aria-busy="true"><span class="mx-sr">Loading activity…</span><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div></div>
      {:else if !historyError}
        <p class="mx-body-sm ac-none">Nothing yet. Commands run by tools with access are recorded here.</p>
      {/if}
    </div>
  </SettingGroup>
</div>

<style>
  .agent-access:focus {
    outline: none;
  }

  .ac-lede {
    margin: 0 0 var(--s-3);
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
    max-width: 52ch;
  }

  .ac-rows {
    border-top: 1px solid var(--app-border);
  }

  .ac-desc {
    margin: 0;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  .ac-path {
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text);
  }

  /* Sub-sections under the CLI Access head (mockup `.ac-sec`). */
  .ac-sec {
    display: grid;
    gap: 0;
    margin-top: var(--s-5);
  }

  .ac-sec__head {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    padding-bottom: var(--s-2);
  }

  .ac-sec__head b {
    font: 500 var(--text-md) / 1.3 var(--font-sans);
    color: var(--app-text-strong);
  }

  .ac-sec__hint {
    margin: 0;
    font-size: var(--text-base);
    color: var(--app-text-subtle);
  }

  .ac-sec__act {
    margin-left: auto;
  }

  .ac-stale {
    margin-top: var(--s-4);
  }

  /* Tool rows: name + provenance · scope · last use · idle meter · action. */
  .ac-list {
    border: 1px solid var(--app-border);
    border-radius: var(--r-md);
    background: var(--app-surface);
  }

  .ac-list :global(.mx-skel-row) {
    padding: 12px 14px;
  }

  .ac-list code {
    font: 450 var(--text-sm) / 1 var(--font-mono);
    padding: 2px 5px;
    border-radius: var(--r-xs);
    background: var(--mx-wash);
    color: var(--app-text);
  }

  .ac-tool {
    display: grid;
    grid-template-columns: 32px minmax(0, 1fr) 132px auto;
    align-items: center;
    gap: var(--s-3);
    padding: 12px var(--s-3) 12px 14px;
    border-top: 1px solid var(--mx-hairline);
  }

  .ac-tool:first-child {
    border-top: 0;
  }

  .ac-tool > .mx-btn {
    justify-self: end;
  }

  .ac-glyph {
    width: 32px;
    height: 32px;
    display: grid;
    place-items: center;
    border-radius: var(--r-md);
    background: var(--mx-wash);
    color: var(--app-text-muted);
    box-shadow: inset 0 0 0 1px var(--mx-hairline);
  }

  .ac-tool.is-blocked .ac-glyph {
    color: var(--app-danger-text);
    background: var(--app-danger-bg);
    box-shadow: inset 0 0 0 1px var(--app-danger-border);
  }

  .ac-tool__main {
    display: grid;
    gap: 5px;
    min-width: 0;
  }

  .ac-tool__name {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    min-width: 0;
    font: 500 var(--text-md) / 1.2 var(--font-sans);
    color: var(--app-text-strong);
  }

  .ac-tool__label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ac-tool.is-blocked .ac-tool__name {
    color: var(--app-text-muted);
  }

  .ac-tool__meta {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font: 400 var(--text-base) / 1.3 var(--font-sans);
    color: var(--app-text-subtle);
  }

  .ac-tool__meta i {
    font-style: normal;
    color: var(--app-text-faint);
  }

  .ac-tool__meta code {
    padding: 0;
    background: none;
    color: var(--app-text-muted);
  }

  /* Idle meter: what is left of the 30-day idle window. Lapsing is benign
     disuse (ADR 0059), not a fault: "soon" only gets stronger ink, no warn. */
  .ac-idle {
    display: grid;
    gap: 5px;
  }

  .ac-idle b {
    font: 400 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-muted);
    white-space: nowrap;
  }

  .ac-idle.is-soon b {
    color: var(--app-text-strong);
  }

  .ac-idle--blocked b {
    color: var(--app-text-subtle);
  }

  .ac-idle__bar {
    height: 3px;
    border-radius: 2px;
    background: var(--mx-wash-strong);
    overflow: hidden;
  }

  .ac-idle__bar i {
    display: block;
    height: 100%;
    width: var(--left);
    border-radius: 2px;
    background: var(--chart-grey-4);
  }

  /* Activity (audit): one grid with subgrid rows so the columns line up. */
  .ac-log {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto auto;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ac-log li {
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    align-items: baseline;
    gap: var(--s-4);
    padding: 9px 2px;
    border-bottom: 1px solid var(--mx-hairline);
    font: 400 var(--text-base) / 1.3 var(--font-sans);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }

  .ac-log b {
    overflow: hidden;
    font-weight: 500;
    color: var(--app-text);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ac-log code {
    font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text);
  }

  /* WHEN a tool read your history is audit evidence: subtle, never faint. */
  .ac-log .num {
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    text-align: right;
    white-space: nowrap;
  }

  .ac-log .is-refused {
    color: var(--app-warn);
    font-weight: 500;
  }

  .ac-none {
    margin: 0;
  }
</style>
