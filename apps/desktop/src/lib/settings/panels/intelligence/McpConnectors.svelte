<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import McpToolListModal from "./McpToolListModal.svelte";
  import McpConnectorPicker from "./McpConnectorPicker.svelte";
  import { activeToolCount } from "$lib/settings/state/mcp-tool-curation";
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

  // Node probe for stdio rows: one probe per settings visit, cached here.
  // undefined = not probed (or still in flight), string = found version,
  // null = Node missing → warn badge on every stdio row.
  let nodeVersion = $state<string | null | undefined>(undefined);
  if (rec.draftMcpServers.some((s) => s.transport === "stdio")) {
    void aiRuntime.checkNode().then((v) => {
      nodeVersion = v;
    });
  }

  // Picker modal: add mode (editId null) or edit mode for one connector.
  let pickerOpen = $state(false);
  let pickerEditId = $state<string | null>(null);
  // One-shot flash for a just-added row (mockup row-flash). Plain local state,
  // never cached on the draft objects; cleared on animationend.
  let flashId = $state<string | null>(null);

  const openPicker = () => {
    pickerEditId = null;
    pickerOpen = true;
  };
  const openConfigure = (id: string) => {
    pickerEditId = id;
    pickerOpen = true;
  };

  const onPickerAdded = (id: string) => {
    flashId = id;
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
  hint="Connect Model Context Protocol servers so chat can use their tools (GitHub, Linear, a filesystem, …). Each connector is offered to the chat agent only while enabled. A connector's single secret is stored only in the macOS keychain — never in Mnema's settings."
>
  <SettingRow label="Connectors" full divider={false}>
    {#snippet control()}
      <div class="mcp-stack">
        {#if rec.draftMcpServers.length === 0}
          <p class="group-hint">No connectors yet. Add one below, then enable it.</p>
        {:else}
          <ul class="mcp-list">
            {#each rec.draftMcpServers as server (server.id)}
              <li
                class="mcp-row"
                class:mcp-row--new={flashId === server.id}
                onanimationend={(e) => {
                  // Child animations (saved-badge-in) bubble too — only the
                  // row's own flash ending should clear the one-shot state.
                  if (e.target === e.currentTarget && flashId === server.id) flashId = null;
                }}
              >
                <div class="mcp-row__main">
                  <span class="mcp-row__name">{server.label.trim() || server.id}</span>
                  <span class="mcp-row__tag">{server.transport}</span>
                  {#if mcpSecretSavedById[server.id]}
                    <span class="saved-badge"><IconCheck class="saved-badge__icon" aria-hidden="true" />secret in keychain</span>
                  {/if}
                  {#if server.transport === "stdio" && nodeVersion === null}
                    <span class="badge badge--warn badge--sm">needs Node — install from nodejs.org</span>
                  {/if}
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
                </div>
                <div class="mcp-row__meta">
                  <button
                    class="btn btn--ghost btn--sm"
                    type="button"
                    onclick={() => openConfigure(server.id)}
                  >
                    Configure
                  </button>
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
  onClose={() => { pickerOpen = false; pickerEditId = null; }}
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
  .mcp-row__main {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    min-width: 0;
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
</style>
