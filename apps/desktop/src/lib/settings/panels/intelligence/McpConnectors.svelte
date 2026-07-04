<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Segmented from "$lib/components/Segmented.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import McpToolListModal from "./McpToolListModal.svelte";
  import { activeToolCount } from "$lib/settings/state/mcp-tool-curation";
  import IconCheck from "~icons/lucide/check";
  import IconTrash from "~icons/lucide/trash-2";

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;

  // Tool-list modal: the id of the connector being curated (null = closed), and
  // discovered tool counts keyed by id (populated on a successful list) so the
  // per-server caption can show "N tools · M active" once known.
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

  // Store-read aliases (keychain secret state, keyed by server id).
  const mcpSecretInputs = $derived(aiRuntime.mcpSecretInputs);
  const mcpSecretSavedById = $derived(aiRuntime.mcpSecretSavedById);
  const mcpSecretSavingId = $derived(aiRuntime.mcpSecretSavingId);
  const mcpSecretErrors = $derived(aiRuntime.mcpSecretErrors);

  const addMcpServer = () => c.addMcpServer();
  const removeMcpServer = (id: string) => void c.removeMcpServer(id);
  const saveMcpServerSecret = (id: string) => aiRuntime.saveMcpServerSecret(id);
  const clearMcpServerSecret = (id: string) => aiRuntime.clearMcpServerSecret(id);

  const TRANSPORT_OPTIONS = [
    { value: "stdio", label: "stdio (local process)" },
    { value: "http", label: "HTTP (remote)" },
  ];
</script>

<SettingGroup
  title="MCP connectors"
  hint="Connect Model Context Protocol servers so chat can use their tools (GitHub, Linear, a filesystem, …). Each connector is offered to the chat agent only while enabled. A connector's single secret is stored only in the macOS keychain — never in Mnema's settings."
>
  <SettingRow label="Connectors" full divider={false}>
    {#snippet control()}
      <div class="mcp-stack">
        {#if rec.draftMcpServers.length === 0}
          <p class="group-hint">No connectors yet. Add one below, fill in how to reach it, then enable it.</p>
        {:else}
          <ul class="mcp-list">
            {#each rec.draftMcpServers as server (server.id)}
              <li class="mcp-row">
                <div class="mcp-row__head">
                  <span class="mcp-row__name">{server.label.trim() || server.id}</span>
                  <span class="mcp-row__tag">{server.transport}</span>
                  {#if mcpSecretSavedById[server.id]}
                    <span class="saved-badge"><IconCheck class="saved-badge__icon" aria-hidden="true" />secret in keychain</span>
                  {/if}
                  <div class="mcp-row__head-actions">
                    <Switch
                      bind:checked={server.enabled}
                      ariaLabel="Enable {server.label.trim() || server.id}"
                    />
                    <button
                      class="btn btn--danger btn--sm"
                      type="button"
                      onclick={() => removeMcpServer(server.id)}
                    >
                      <IconTrash aria-hidden="true" />
                      Remove
                    </button>
                  </div>
                </div>

                <label class="field-label" for="mcp-label-{server.id}">Name</label>
                <input
                  id="mcp-label-{server.id}"
                  class="text-input"
                  autocomplete="off"
                  placeholder="e.g. GitHub"
                  bind:value={server.label}
                />

                <span class="field-label">Transport</span>
                <Segmented
                  bind:value={server.transport}
                  ariaLabel="Transport for {server.label.trim() || server.id}"
                  options={TRANSPORT_OPTIONS}
                />

                {#if server.transport === "stdio"}
                  <label class="field-label" for="mcp-command-{server.id}">Command</label>
                  <input
                    id="mcp-command-{server.id}"
                    class="text-input"
                    autocomplete="off"
                    placeholder="npx"
                    bind:value={
                      () => server.command ?? "",
                      (v) => { server.command = v; }
                    }
                  />

                  <span class="field-label">Arguments</span>
                  {#each server.args as _, i (i)}
                    <div class="mcp-inline-row">
                      <input
                        class="text-input"
                        autocomplete="off"
                        placeholder="argument"
                        aria-label="Argument {i + 1}"
                        bind:value={server.args[i]}
                      />
                      <button
                        class="btn btn--ghost btn--sm"
                        type="button"
                        aria-label="Remove argument {i + 1}"
                        onclick={() => { server.args = server.args.filter((_, idx) => idx !== i); }}
                      >
                        Remove
                      </button>
                    </div>
                  {/each}
                  <div class="row-actions">
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      onclick={() => { server.args = [...server.args, ""]; }}
                    >
                      + Argument
                    </button>
                  </div>

                  <span class="field-label">Environment variables</span>
                  <p class="group-hint">Non-secret values only. The one secret below is delivered separately via the keychain.</p>
                  {#each server.env as _, i (i)}
                    <div class="mcp-inline-row">
                      <input
                        class="text-input"
                        autocomplete="off"
                        placeholder="NAME"
                        aria-label="Env var {i + 1} name"
                        bind:value={server.env[i].name}
                      />
                      <input
                        class="text-input"
                        autocomplete="off"
                        placeholder="value"
                        aria-label="Env var {i + 1} value"
                        bind:value={server.env[i].value}
                      />
                      <button
                        class="btn btn--ghost btn--sm"
                        type="button"
                        aria-label="Remove env var {i + 1}"
                        onclick={() => { server.env = server.env.filter((_, idx) => idx !== i); }}
                      >
                        Remove
                      </button>
                    </div>
                  {/each}
                  <div class="row-actions">
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      onclick={() => { server.env = [...server.env, { name: "", value: "" }]; }}
                    >
                      + Variable
                    </button>
                  </div>

                  <label class="field-label" for="mcp-secret-env-{server.id}">Secret delivered as env var</label>
                  <input
                    id="mcp-secret-env-{server.id}"
                    class="text-input"
                    autocomplete="off"
                    placeholder="e.g. GITHUB_TOKEN"
                    bind:value={
                      () => server.secretEnvName ?? "",
                      (v) => { server.secretEnvName = v; }
                    }
                  />
                  <p class="group-hint">The keychain secret below is injected into the child process under this env var name.</p>
                {:else}
                  <label class="field-label" for="mcp-url-{server.id}">URL</label>
                  <input
                    id="mcp-url-{server.id}"
                    class="text-input"
                    class:text-input--empty={(server.url ?? "").trim().length === 0}
                    autocomplete="off"
                    placeholder="https://mcp.example.com/mcp"
                    aria-invalid={(server.url ?? "").trim().length === 0}
                    bind:value={
                      () => server.url ?? "",
                      (v) => { server.url = v; }
                    }
                  />
                  {#if (server.url ?? "").trim().length === 0}
                    <p class="group-hint group-hint--warn">An HTTP connector needs a URL.</p>
                  {/if}
                  <p class="group-hint">The keychain secret below is sent as an <code>Authorization: Bearer</code> header.</p>
                {/if}

                <label class="field-label" for="mcp-secret-{server.id}">Secret</label>
                <input
                  id="mcp-secret-{server.id}"
                  class="text-input"
                  class:text-input--error={!!mcpSecretErrors[server.id]}
                  type="password"
                  autocomplete="off"
                  placeholder={mcpSecretSavedById[server.id] ? "A secret is saved — enter a new one to replace it" : "Optional bearer token / secret"}
                  aria-invalid={!!mcpSecretErrors[server.id]}
                  aria-describedby={mcpSecretErrors[server.id] ? `mcp-secret-error-${server.id}` : undefined}
                  disabled={mcpSecretSavingId === server.id}
                  bind:value={
                    () => mcpSecretInputs[server.id] ?? "",
                    (v) => { aiRuntime.setMcpSecretInput(server.id, v); }
                  }
                />
                <div class="row-actions">
                  <button
                    class="btn btn--ghost btn--sm"
                    type="button"
                    disabled={mcpSecretSavingId !== null || (mcpSecretInputs[server.id] ?? "").trim().length === 0}
                    aria-busy={mcpSecretSavingId === server.id}
                    onclick={() => saveMcpServerSecret(server.id)}
                  >
                    {#if mcpSecretSavingId === server.id}<ButtonSpinner />Saving{:else}Save secret{/if}
                  </button>
                  <button
                    class="btn btn--ghost btn--sm"
                    type="button"
                    disabled={mcpSecretSavingId !== null || !mcpSecretSavedById[server.id]}
                    onclick={() => clearMcpServerSecret(server.id)}
                  >
                    Clear
                  </button>
                </div>
                {#if mcpSecretErrors[server.id]}
                  <p class="error-text" id="mcp-secret-error-{server.id}" role="alert">{mcpSecretErrors[server.id]}</p>
                {:else if (mcpSecretInputs[server.id] ?? "").trim().length > 0 && mcpSecretSavingId !== server.id}
                  <p class="group-hint group-hint--warn">Unsaved secret — click <strong>Save secret</strong> to store it in the keychain.</p>
                {/if}

                <span class="field-label">Tools</span>
                <div class="mcp-tools-row">
                  <button
                    class="btn btn--ghost btn--sm"
                    type="button"
                    disabled={!server.enabled}
                    onclick={() => { toolModalServerId = server.id; }}
                  >
                    See tool list
                  </button>
                  {#if !server.enabled}
                    <span class="group-hint">Enable this connector to list its tools.</span>
                  {:else if toolCounts[server.id] !== undefined}
                    <span class="mcp-tools-row__count">{toolCounts[server.id]} tools · {activeToolCount(toolCounts[server.id], server.enabledTools)} active</span>
                  {:else if server.enabledTools}
                    <span class="mcp-tools-row__count">{server.enabledTools.length} active — open the list to load all tools</span>
                  {:else}
                    <span class="group-hint">Not connected yet — open the list to see its tools.</span>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}

        <div class="row-actions">
          <button class="btn btn--ghost btn--sm" type="button" onclick={addMcpServer}>
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
    gap: 12px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .mcp-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 12px;
    border: 1px solid var(--settings-border, rgba(255, 255, 255, 0.1));
    border-radius: 8px;
  }
  .mcp-row__head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .mcp-row__name {
    font-weight: 600;
  }
  .mcp-row__head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-left: auto;
  }
  .mcp-inline-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .mcp-inline-row .text-input {
    flex: 1 1 auto;
    min-width: 0;
  }
  .mcp-tools-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .mcp-tools-row__count {
    font-size: 11px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
</style>
