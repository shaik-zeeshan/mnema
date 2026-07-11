<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import McpToolListModal from "./McpToolListModal.svelte";
  import McpConnectorPicker from "./McpConnectorPicker.svelte";
  import { activeToolCount } from "$lib/settings/state/mcp-tool-curation";
  import {
    deriveMcpConnectorRow,
    type McpRowBadge,
  } from "$lib/settings/state/mcp-connector-row";
  import type { McpServerConfig } from "$lib/types";
  import IconCheck from "~icons/lucide/check";

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;

  // Tool-list modal: the id of the connector being curated (null = closed), and
  // discovered tool counts keyed by id (populated on a successful list) so the
  // per-row caption can show "N tools · M active" once known.
  let toolModalServerId = $state<string | null>(null);
  let toolCounts = $state<Record<string, number>>({});
  const toolModalServer = $derived(
    rec.draftMcpServers.find((s) => s.id === toolModalServerId) ?? null,
  );

  // Write a new curation onto the matching draft connector. Mutating the draft
  // proxy rides the existing ai_runtime autosave (which serializes `enabledTools`
  // via `toMcpServerWire`) — no extra save call needed.
  const curate = (id: string | null, enabledTools: string[] | null) => {
    if (!id) return;
    const server = rec.draftMcpServers.find((s) => s.id === id);
    if (server) server.enabledTools = enabledTools;
  };

  const mcpSecretSavedById = $derived(aiRuntime.mcpSecretSavedById);
  const mcpOAuthStateById = $derived(aiRuntime.mcpOAuthStateById);
  const mcpOAuthErrors = $derived(aiRuntime.mcpOAuthErrors);

  // Fetch OAuth states on mount and keep them live: the backend fires
  // `mcp_authorization_changed` when a browser callback completes or a disconnect
  // happens, so a re-fetch flips the affected row (none→authorizing→authorized).
  // Co-located here (not the settings page) so it disposes when the panel
  // unmounts. slice 8b runs the Connect inline in the modal; this event wiring
  // is unchanged by that.
  $effect(() => {
    void aiRuntime.refreshMcpOAuthStates();
    let unlisten: (() => void) | undefined;
    let disposed = false;
    void listen("mcp_authorization_changed", () => {
      void aiRuntime.refreshMcpOAuthStates();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // Connect AND Reconnect open the in-modal connect flow (slice 8b) for the
  // connector id — the picker runs `beginMcpOAuth` and shows the browser-handoff
  // stages, flipping to Authorized on the same `mcp_authorization_changed` event
  // this panel already listens for. Disconnect stays a direct store action.
  const disconnectOAuth = (id: string) => void aiRuntime.disconnectMcpOAuth(id);

  // http+oauth connectors carry the auth mode as their row tag (Bearer / OAuth);
  // stdio keeps its transport tag. Matches the mockup's per-row `tag`.
  const rowTag = (server: McpServerConfig): string =>
    server.transport === "http" ? (server.authMode === "oauth" ? "OAuth" : "Bearer") : server.transport;

  // The tool-count "see tools" affordance only makes sense for a usable-ish row
  // (a saved bearer secret, or an authorized OAuth token). An unauthorized /
  // authorizing / reconnecting OAuth row shows an explanatory sub-line instead.
  const showToolCount = (badge: McpRowBadge): boolean =>
    badge === "secret" || badge === "none" || badge === "authorized";

  // The state sub-line copy (mockup `sub(c)`), only for the OAuth states that
  // carry no tool count.
  const subLine = (badge: McpRowBadge, enabled: boolean): string => {
    switch (badge) {
      case "authorized-muted":
        return "authorized · token kept — turn on to use it again";
      case "not-connected":
        return enabled
          ? "Enabled, but not authorized yet — approve in your browser to start using it."
          : "Not authorized yet — turn it on and Connect to start using it.";
      case "authorizing":
        return "Waiting for your browser… approve access in the tab that opened.";
      case "reconnect":
        return "Token expired — chat reports it as unavailable until you reconnect.";
      default:
        return "";
    }
  };

  // Node probe for stdio rows: one probe per settings visit, cached here.
  // undefined = not probed (or still in flight), string = found version,
  // null = Node missing → warn badge on every stdio row.
  let nodeVersion = $state<string | null | undefined>(undefined);
  if (rec.draftMcpServers.some((s) => s.transport === "stdio")) {
    void aiRuntime.checkNode().then((v) => {
      nodeVersion = v;
    });
  }

  // Picker modal: add mode (all null), edit/configure (editId), or the in-modal
  // OAuth connect/reconnect flow (connectId) for one existing connector.
  let pickerOpen = $state(false);
  let pickerEditId = $state<string | null>(null);
  let pickerConnectId = $state<string | null>(null);
  let pickerConnectMode = $state<"connect" | "reconnect">("connect");
  // One-shot flash for a just-added row (mockup row-flash). Plain local state,
  // never cached on the draft objects; cleared on animationend.
  let flashId = $state<string | null>(null);

  const closePicker = () => {
    pickerOpen = false;
    pickerEditId = null;
    pickerConnectId = null;
  };
  const openPicker = () => {
    pickerEditId = null;
    pickerConnectId = null;
    pickerOpen = true;
  };
  const openConfigure = (id: string) => {
    pickerEditId = id;
    pickerConnectId = null;
    pickerOpen = true;
  };
  const openConnect = (id: string, mode: "connect" | "reconnect") => {
    pickerEditId = null;
    pickerConnectId = id;
    pickerConnectMode = mode;
    pickerOpen = true;
  };

  const onPickerAdded = (id: string) => {
    flashId = id;
    // A just-added OAuth connector should immediately read "not connected" with
    // a Connect button — refresh so its row picks up the "none" state.
    void aiRuntime.refreshMcpOAuthStates();
    // First stdio connector may have been added via the picker after this
    // panel's mount-time probe was skipped — probe now so the warn badge can
    // show when Node is missing. (mcp_check_node is a cheap invoke; the
    // picker's own probe result is component-local.)
    if (nodeVersion === undefined && rec.draftMcpServers.some((s) => s.transport === "stdio")) {
      void aiRuntime.checkNode().then((v) => {
        nodeVersion = v;
      });
    }
  };
</script>

<SettingGroup
  title="MCP connectors"
  hint="Connect Model Context Protocol servers so chat can use their tools (GitHub, Notion, a filesystem, …). A connector works only when it's both enabled AND authorized — two separate things. Some servers sign in with OAuth (click Connect, approve in your browser); others take a pasted token. Either way the secret lives only in the macOS keychain — never in Mnema's settings."
>
  <SettingRow label="Connectors" full divider={false}>
    {#snippet control()}
      <div class="mcp-stack">
        {#if rec.draftMcpServers.length === 0}
          <p class="group-hint">No connectors yet. Add one, then Connect (OAuth) or enable it.</p>
        {:else}
          <ul class="mcp-list">
            {#each rec.draftMcpServers as server (server.id)}
              {@const view = deriveMcpConnectorRow({
                authMode: server.authMode,
                enabled: server.enabled,
                hasSecret: mcpSecretSavedById[server.id],
                oauthState: mcpOAuthStateById[server.id],
              })}
              {@const oauthError = mcpOAuthErrors[server.id]}
              {@const stateSub = subLine(view.badge, server.enabled)}
              <li
                class="mcp-row"
                class:mcp-row--off={view.dimmed}
                class:mcp-row--new={flashId === server.id}
                onanimationend={(e) => {
                  // Child animations (saved-badge-in) bubble too — only the
                  // row's own flash ending should clear the one-shot state.
                  if (e.target === e.currentTarget && flashId === server.id) flashId = null;
                }}
              >
                <div class="mcp-row__body">
                  <div class="mcp-row__title">
                    <span class="mcp-row__name">{server.label.trim() || server.id}</span>
                    <span class="mcp-row__tag">{rowTag(server)}</span>
                    {#if server.transport === "stdio" && nodeVersion === null}
                      <span class="badge badge--warn badge--sm">needs Node — install from nodejs.org</span>
                    {/if}
                    {#if view.badge === "secret"}
                      <span class="saved-badge"><IconCheck class="saved-badge__icon" aria-hidden="true" />secret in keychain</span>
                    {:else if view.badge === "authorized"}
                      <span class="badge badge--ok">✓ authorized</span>
                    {:else if view.badge === "authorized-muted"}
                      <span class="badge badge--muted">authorized</span>
                    {:else if view.badge === "not-connected"}
                      <span class="badge badge--warn">not connected</span>
                    {:else if view.badge === "authorizing"}
                      <span class="badge"><span class="mcp-spinner" aria-hidden="true"></span>authorizing…</span>
                    {:else if view.badge === "reconnect"}
                      <span class="badge badge--warn">needs reconnect</span>
                    {/if}
                  </div>
                  <div class="mcp-row__sub">
                    {#if stateSub}
                      <span class:mcp-row__note={view.badge === "reconnect"}>{stateSub}</span>
                    {/if}
                    {#if showToolCount(view.badge)}
                      <button
                        class="mcp-row__count"
                        type="button"
                        disabled={!server.enabled}
                        title={server.enabled ? "See and curate this connector's tools" : "Enable this connector to list its tools."}
                        onclick={() => { toolModalServerId = server.id; }}
                      >
                        {#if toolCounts[server.id] !== undefined}
                          {toolCounts[server.id]} tools · {activeToolCount(toolCounts[server.id], server.enabledTools)} active
                        {:else if server.enabledTools}
                          {server.enabledTools.length} active
                        {:else}
                          see tools
                        {/if}
                      </button>
                    {/if}
                  </div>
                  {#if oauthError}
                    <p class="error-text" role="alert">{oauthError}</p>
                  {/if}
                </div>
                <div class="mcp-row__meta">
                  {#if view.actions.connect}
                    <button class="btn btn--primary btn--sm" type="button" onclick={() => openConnect(server.id, "connect")}>
                      Connect
                    </button>
                  {/if}
                  {#if view.actions.reconnect}
                    <button class="btn btn--primary btn--sm" type="button" onclick={() => openConnect(server.id, "reconnect")}>
                      Reconnect
                    </button>
                  {/if}
                  {#if view.actions.disconnect}
                    <button class="btn btn--ghost btn--sm" type="button" onclick={() => disconnectOAuth(server.id)}>
                      Disconnect
                    </button>
                  {/if}
                  {#if view.actions.configure}
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      onclick={() => openConfigure(server.id)}
                    >
                      Configure
                    </button>
                  {/if}
                  <Switch
                    bind:checked={server.enabled}
                    ariaLabel="Enable {server.label.trim() || server.id}"
                  />
                </div>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="row-actions">
          <button class="btn btn--ghost btn--sm" type="button" onclick={openPicker}>
            + Add connector
          </button>
        </div>
        <p class="group-hint">Secrets are stored only in the macOS keychain — never in Mnema's settings, config, or save directory. Removing a connector deletes its secret right away.</p>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<McpToolListModal
  open={toolModalServerId !== null}
  server={toolModalServer}
  onClose={() => { toolModalServerId = null; }}
  onCurate={(enabledTools) => curate(toolModalServerId, enabledTools)}
  onToolsDiscovered={(id, count) => { toolCounts = { ...toolCounts, [id]: count }; }}
/>

<McpConnectorPicker
  open={pickerOpen}
  editId={pickerEditId}
  connectId={pickerConnectId}
  connectMode={pickerConnectMode}
  onClose={closePicker}
  onAdded={onPickerAdded}
  onToolsDiscovered={(id, count) => { toolCounts = { ...toolCounts, [id]: count }; }}
/>

<style>
  .mcp-stack {
    display: flex;
    flex-direction: column;
    gap: 12px;
    width: 100%;
  }
  .mcp-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .mcp-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--settings-border, rgba(255, 255, 255, 0.1));
    border-radius: 8px;
  }
  /* Just-added row flash (mockup row-flash), cleared on animationend. */
  .mcp-row--new {
    animation: mcp-row-flash 1.4s ease-out;
  }
  @keyframes mcp-row-flash {
    from {
      background: color-mix(in srgb, var(--app-accent) 10%, transparent);
    }
    to {
      background: transparent;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .mcp-row--new {
      animation: none;
    }
  }
  /* mockup is-off: a static/authorized connector switched off dims its content
     (title + sub) but keeps its actions/toggle fully usable. */
  .mcp-row--off .mcp-row__body {
    opacity: 0.5;
  }
  .mcp-row__body {
    display: flex;
    flex-direction: column;
    gap: 5px;
    min-width: 0;
  }
  .mcp-row__title {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .mcp-row__sub {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
    line-height: 1.4;
  }
  .mcp-row__note {
    color: var(--app-warn);
  }
  .mcp-row__name {
    font-weight: 600;
  }
  .mcp-row__tag {
    font-size: var(--text-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    padding: 2px 6px;
  }
  .mcp-row__count {
    padding: 0;
    border: 0;
    background: transparent;
    font-family: inherit;
    font-size: var(--text-sm);
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .mcp-row__count:hover:not(:disabled) {
    color: var(--app-text-strong);
    text-decoration: underline;
  }
  .mcp-row__count:disabled {
    cursor: default;
    opacity: 0.5;
  }
  .mcp-row__meta {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  /* Pending badge spinner (mockup `.spinner`). */
  .mcp-spinner {
    display: inline-block;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--app-border-hover);
    border-top-color: var(--app-accent);
    animation: mcp-spin 0.7s linear infinite;
  }
  @keyframes mcp-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .mcp-spinner {
      animation: none;
    }
  }
</style>
