<script lang="ts">
  // McpOAuthConnectPanel — the in-modal OAuth connect flow (Plan: MCP OAuth,
  // slice 8b), split out of McpConnectorPicker (800-line cap split). Owns ONE
  // connect flow's lifecycle: the stage derivation from the live store, the
  // attempt latches, and the Connect/Retry/Done handlers. Rendered only while
  // its parent decides the panel is showing (`showMcpOAuthConnectPanel`), so its
  // attempt state resets naturally on each open via mount/unmount.
  //
  // Two entry doors share this ONE flow: (1) a hosted-OAuth preset picked from
  // the catalog (`selected`, added + persisted once on Connect, then authorized
  // in place), and (2) row Connect/Reconnect on an EXISTING connector
  // (`connectServer`). The lede + chips + the presentational McpOAuthConnect
  // stage block all live here; the parent owns only the modal shell + routing.
  import { humanizeError } from "$lib/format-error";
  import McpOAuthConnect from "./McpOAuthConnect.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import type { McpPreset } from "$lib/settings/state/mcp-presets";
  import { deriveMcpOAuthStage } from "$lib/settings/state/mcp-oauth-stage";
  import type { McpServerConfig } from "$lib/types";

  interface Props {
    /** The existing connector being (re)connected from its row (null on the
     *  catalog-add path, where `selected` names a not-yet-added preset). */
    connectServer: McpServerConfig | null;
    /** The chosen catalog preset (the add-on-connect path); null for a row. */
    selected: McpPreset | null;
    /** Row Connect ("connect", `none`) vs Reconnect ("reconnect") — panel verb. */
    connectMode: "connect" | "reconnect";
    /** The preset URL (Advanced-overridable), for the chip path on the add path. */
    advUrl: string;
    /** Parent-computed preset overrides applied when the preset is added here. */
    overrides: Partial<McpServerConfig>;
    /** A brand-new connector was authorized (catalog-add path) — flash its row. */
    onAdded: (id: string) => void;
    /** Close the modal (Cancel, or after a row Connect/Reconnect authorized). */
    onClose: () => void;
  }

  let { connectServer, selected, connectMode, advUrl, overrides, onAdded, onClose }: Props = $props();

  const c = getSettingsController();
  const aiRuntime = c.aiRuntime;

  const mcpOAuthStateById = $derived(aiRuntime.mcpOAuthStateById);
  const mcpOAuthErrors = $derived(aiRuntime.mcpOAuthErrors);

  let oauthAttempted = $state(false);
  let oauthSawAuthorizing = $state(false);
  let submitError = $state<string | null>(null);
  // The id minted when a catalog OAuth preset is added at Connect-click (null for
  // the row path, where `connectServer` already names an existing connector).
  let addedOauthId = $state<string | null>(null);

  // The id whose live state drives the stage (row id, or the just-added preset id).
  const oauthConnectId = $derived(connectServer?.id ?? addedOauthId);
  const oauthState = $derived(oauthConnectId ? mcpOAuthStateById[oauthConnectId] : undefined);
  const oauthHasError = $derived(!!(oauthConnectId && mcpOAuthErrors[oauthConnectId]));
  const oauthStage = $derived(
    deriveMcpOAuthStage({
      state: oauthState,
      attempted: oauthAttempted,
      hasError: oauthHasError,
      sawAuthorizing: oauthSawAuthorizing,
    }),
  );

  // Latch "we reached authorizing" so a later fall-back to none/reconnect reads
  // as a denial (not begin-still-in-flight). Write-only flag — no derived cycle.
  $effect(() => {
    if (oauthState === "authorizing") oauthSawAuthorizing = true;
  });

  const oauthVerb = $derived<"Connect" | "Reconnect">(
    connectMode === "reconnect" ? "Reconnect" : "Connect",
  );
  const oauthLabel = $derived(
    connectServer ? connectServer.label.trim() || connectServer.id : (selected?.label ?? ""),
  );
  const oauthLede = $derived(
    connectServer
      ? `Chat can use ${oauthLabel}'s tools once you approve access in your browser.`
      : (selected?.lede ?? ""),
  );
  const oauthPath = $derived(
    (connectServer?.url ?? (selected ? advUrl : "")).trim().replace(/^https?:\/\//, ""),
  );

  // Connect: ensure the connector exists (a catalog preset is added + persisted
  // ONCE — the backend resolves `mcp_oauth_begin` by id from settings), then
  // begin the browser flow. begin records failures into mcpOAuthErrors[id] (no
  // throw), so `oauthStage` flips to "denied" on its own.
  async function oauthConnectStart(): Promise<void> {
    oauthAttempted = true;
    oauthSawAuthorizing = false;
    let id = oauthConnectId;
    if (!id && selected) {
      try {
        id = c.addMcpServerFromPreset(selected, overrides);
        addedOauthId = id;
        await c.flushAiRuntimeSave();
      } catch (err) {
        submitError = humanizeError(err);
        console.error("[McpOAuthConnectPanel] OAuth add-before-connect failed", err);
        // Roll back the orphan draft; drop back to idle so the user can retry.
        if (id) await c.removeMcpServer(id, { confirm: false });
        addedOauthId = null;
        oauthAttempted = false;
        return;
      }
    }
    if (!id) return;
    await aiRuntime.beginMcpOAuth(id);
    await aiRuntime.refreshMcpOAuthStates();
  }

  function oauthRetry(): void {
    oauthAttempted = false;
    oauthSawAuthorizing = false;
  }

  // Done (authorized). For a catalog-add the connector is brand-new → tell the
  // parent so it flashes the row + refreshes. For a row Connect/Reconnect the row
  // already exists and flipped on the same event, so just close.
  function oauthDone(): void {
    if (addedOauthId) onAdded(addedOauthId);
    onClose();
  }
</script>

<!-- OAuth connect panel: catalog-add of a hosted-OAuth preset, or a row's
     Connect/Reconnect. Stage is derived from the live store. -->
<div class="connect">
  {#if oauthLede}<p class="connect__lede">{oauthLede}</p>{/if}
  <div class="chip-row">
    <span class="chip">HTTP</span>
    {#if oauthPath}<span class="chip chip--path">{oauthPath}</span>{/if}
    <span class="chip chip--oauth">OAuth</span>
  </div>
  <McpOAuthConnect
    stage={oauthStage}
    label={oauthLabel}
    verb={oauthVerb}
    onConnect={() => void oauthConnectStart()}
    onCancel={onClose}
    onDone={oauthDone}
    onRetry={oauthRetry}
  />
  {#if submitError}
    <p class="error-text" role="alert">{submitError}</p>
  {/if}
</div>

<style>
  .connect {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .connect__lede {
    margin: 0;
    font-size: 11px;
    line-height: 1.5;
    letter-spacing: 0.01em;
    color: var(--app-text-muted);
  }
  .chip-row {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .chip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border-strong);
    border-radius: 5px;
    padding: 3px 7px;
  }
  .chip--path {
    text-transform: none;
    letter-spacing: 0.01em;
    overflow-wrap: anywhere;
  }
  .chip--oauth {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
</style>
