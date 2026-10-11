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
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconPlus from "~icons/lucide/plus";
  import IconPlug from "~icons/lucide/plug";

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

  // The row's description line when no OAuth state sub-line applies.
  const hintFor = (badge: McpRowBadge, server: McpServerConfig): string =>
    badge === "secret"
      ? "Secret in keychain"
      : badge === "authorized"
        ? "Authorized"
        : server.transport === "stdio"
          ? "Runs on this Mac"
          : "No secret saved";

  // Node probe for stdio rows: one probe per settings visit, cached here.
  // undefined = not probed (or still in flight), string = found version,
  // null = Node missing → warn badge on every stdio row. An effect, not a
  // mount-time check: the connector list loads after mount, and a stdio
  // connector can also arrive later via the picker.
  let nodeVersion = $state<string | null | undefined>(undefined);
  let nodeProbed = false;
  $effect(() => {
    if (nodeProbed || !rec.draftMcpServers.some((s) => s.transport === "stdio")) return;
    nodeProbed = true;
    void aiRuntime.checkNode().then((v) => {
      nodeVersion = v;
    });
  });

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
  };
</script>

<SettingGroup title="MCP connectors" hint="Tools chat can use. A connector works only when it’s enabled and authorized.">
  {#if rec.draftMcpServers.length === 0}
    <div class="mcp-empty">
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconPlug width="14" height="14" /></span>
        <b class="mx-empty__title">No connectors yet</b>
        <p class="mx-empty__text">Add one — GitHub, Notion, a folder on this Mac — then Connect or enable it.</p>
        <div class="mx-empty__acts">
          <button class="mx-btn mx-btn--sm" type="button" onclick={openPicker}><IconPlus width="13" height="13" aria-hidden="true" />Add connector…</button>
        </div>
      </div>
    </div>
  {:else}
    {#each rec.draftMcpServers as server (server.id)}
      {@const view = deriveMcpConnectorRow({
        authMode: server.authMode,
        enabled: server.enabled,
        hasSecret: mcpSecretSavedById[server.id],
        oauthState: mcpOAuthStateById[server.id],
      })}
      {@const oauthError = mcpOAuthErrors[server.id]}
      {@const stateSub = subLine(view.badge, server.enabled)}
      {@const name = server.label.trim() || server.id}
      {@const needsNode = server.transport === "stdio" && nodeVersion === null}
      <div
        class="mcp-row"
        class:mcp-row--off={view.dimmed}
        class:mcp-row--new={flashId === server.id}
        onanimationend={(e) => {
          // Child animations bubble too — only the row's own flash ending
          // should clear the one-shot state.
          if (e.target === e.currentTarget && flashId === server.id) flashId = null;
        }}
      >
        <SettingRow label={name}>
          {#snippet labelExtra()}
            <span class="mx-chip">{rowTag(server)}</span>
            {#if needsNode}<span class="mx-chip mx-chip--warn">needs Node</span>{/if}
          {/snippet}
          {#snippet control()}
            {#if view.badge === "authorizing"}
              <span class="settings-state"><span class="mx-spin mx-spin--sm"></span>authorizing…</span>
            {:else if view.badge === "reconnect"}
              <span class="settings-state settings-state--warn">needs reconnect</span>
            {:else if view.badge === "not-connected"}
              <span class="settings-state">not connected</span>
            {/if}
            {#if view.actions.connect}
              <button class="mx-btn" type="button" onclick={() => openConnect(server.id, "connect")}>Connect</button>
            {/if}
            {#if view.actions.reconnect}
              <button class="mx-btn" type="button" onclick={() => openConnect(server.id, "reconnect")}>Reconnect</button>
            {/if}
            {#if view.actions.disconnect}
              <button class="mx-btn mx-btn--ghost" type="button" onclick={() => disconnectOAuth(server.id)}>Disconnect</button>
            {/if}
            {#if view.actions.configure}
              <button class="mx-btn mx-btn--ghost" type="button" onclick={() => openConfigure(server.id)}>Configure</button>
            {/if}
            <Switch bind:checked={server.enabled} ariaLabel="Enable {name}" />
          {/snippet}
          {#snippet below()}
              <span class="mx-inline mcp-hint">
                {#if needsNode}
                  Runs on this Mac and needs Node, which wasn’t found. Install it from nodejs.org.
                {:else if stateSub}
                  {stateSub}
                {:else}
                  {hintFor(view.badge, server)}
                {/if}
                {#if showToolCount(view.badge)}
                  ·
                  <button
                    class="mcp-count"
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
              </span>
              {#if oauthError}
                <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" aria-hidden="true" />{oauthError}</span>
              {/if}
          {/snippet}
        </SettingRow>
      </div>
    {/each}
    <SettingRow label="Add a connector" description="Secrets live only in the macOS keychain; removing a connector deletes its secret.">
      {#snippet control()}
        <button class="mx-btn" type="button" onclick={openPicker}><IconPlus width="13" height="13" aria-hidden="true" />Add connector…</button>
      {/snippet}
    </SettingRow>
  {/if}
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
  .mcp-row :global(.setting-row__control) {
    white-space: nowrap;
  }
  .mcp-empty {
    padding: var(--s-3) 0;
    border-bottom: 1px solid var(--mx-hairline);
  }
  .mcp-empty .mx-empty {
    max-width: none;
    padding: 0;
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
  /* mockup is-off: a static/authorized connector switched off dims its copy
     but keeps its actions and switch fully usable. */
  .mcp-row--off :global(.setting-row__text),
  .mcp-row--off .mcp-hint {
    opacity: 0.5;
  }
  .mcp-count {
    padding: 0;
    border: 0;
    background: transparent;
    font: inherit;
    color: inherit;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .mcp-count:hover:not(:disabled) {
    color: var(--app-text-strong);
    text-decoration: underline;
  }
  .mcp-count:disabled {
    cursor: default;
  }
</style>
