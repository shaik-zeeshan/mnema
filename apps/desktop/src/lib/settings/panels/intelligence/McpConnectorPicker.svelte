<script lang="ts">
  // McpConnectorPicker — the two-step "Add connector" modal (Plan: MCP
  // Connector Preset Picker, slices 3+4; mockup a-modal-grid.html). Step 1
  // (catalog grid) lives in McpConnectorCatalog.svelte; step 2 here: "Connect
  // <Service>" (lede, chips, ONE token field, collapsed ADVANCED, Node status
  // line for local presets) or today's full blank form for Custom.
  //
  // ADD CONNECTOR verifies before finishing: add draft → flush the ai_runtime
  // autosave (mcp_list_server_tools reads persisted config; the 450ms debounce
  // would race it) → save the pasted secret → list tools. Success seeds the
  // tool count, calls onAdded(id), closes. Failure shows the error inline and
  // ROLLS BACK the just-added draft + secret — no orphans survive a bad token.
  //
  // Prop-driven: the parent controls `open` (+ `editId` for edit mode) and
  // receives onAdded / onToolsDiscovered. Edit mode (slice 6): `editId` opens
  // straight on the step-2 body for an EXISTING connector — eyebrow "EDIT
  // CONNECTOR", unchanged-token placeholder, REMOVE (existing confirm +
  // keychain deletion) / SAVE footer. SAVE re-verifies only when a new token
  // was pasted; field edits bind the live draft and ride autosave. The Custom
  // full form lives in McpConnectorCustomForm.svelte (800-line cap split).
  // Overlay / focus / ESC / backdrop scaffold mirrors McpToolListModal.

  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { humanizeError } from "$lib/format-error";
  import { trapTabKey } from "$lib/keyboard";
  import IconCheck from "~icons/lucide/check";
  import ButtonSpinner from "$lib/settings/ui/ButtonSpinner.svelte";
  import McpConnectorCatalog from "./McpConnectorCatalog.svelte";
  import McpConnectorCustomForm from "./McpConnectorCustomForm.svelte";
  import McpOAuthConnect from "./McpOAuthConnect.svelte";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import { MCP_PRESETS, type McpPreset } from "$lib/settings/state/mcp-presets";
  import { deriveMcpOAuthStage } from "$lib/settings/state/mcp-oauth-stage";
  import { newMcpServerId } from "$lib/settings/state/ai-providers";
  import type { McpServerConfig } from "$lib/types";

  interface McpToolDescriptor {
    name: string;
    description: string | null;
  }

  interface Props {
    open: boolean;
    onClose: () => void;
    /** A connector was added AND verified (or added disabled, Node missing). */
    onAdded: (id: string) => void;
    /** Verified tool count, so the new row can show "N tools" immediately. */
    onToolsDiscovered?: (id: string, count: number) => void;
    /** Open in edit mode for this existing connector (skips the catalog). */
    editId?: string | null;
    /**
     * Open straight on the OAuth connect panel for this EXISTING http+oauth
     * connector (row Connect / Reconnect, slice 8b) — no catalog, no add step.
     */
    connectId?: string | null;
    /** "connect" (fresh, `none`) or "reconnect" (`reconnect`) — the panel eyebrow/verb. */
    connectMode?: "connect" | "reconnect";
  }

  let {
    open,
    onClose,
    onAdded,
    onToolsDiscovered,
    editId = null,
    connectId = null,
    connectMode = "connect",
  }: Props = $props();

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;

  // External links open via the opener plugin (About-panel convention).
  const openExternal = (url: string) =>
    void openUrl(url).catch((err) => console.error("[McpConnectorPicker] open url failed", err));

  // ─── Step state ────────────────────────────────────────────────────────────
  let step = $state<"catalog" | "connect">("catalog");
  /** The chosen preset; null while step === "connect" means the Custom form. */
  let selected = $state<McpPreset | null>(null);
  let connecting = $state(false);
  let removing = $state(false);
  let submitError = $state<string | null>(null);

  // Step-2 shared secret input (preset token or Custom's optional secret).
  let token = $state("");

  // Preset ADVANCED overrides (seeded from the preset on select; in edit mode
  // seeded from the draft and written back through syncAdvToDraft).
  let advOpen = $state(false);
  let advName = $state("");
  let advUrl = $state("");
  let advCommand = $state("");
  let advArgs = $state("");

  // Custom form scratch model (add mode). Edit mode passes the live draft to
  // McpConnectorCustomForm instead, so field edits ride autosave.
  const emptyCustomModel = (): McpServerConfig => ({
    id: "",
    label: "",
    enabled: true,
    transport: "http",
    command: "",
    args: [],
    env: [],
    url: "",
    secretEnvName: "",
    enabledTools: null,
  });
  let customModel = $state<McpServerConfig>(emptyCustomModel());

  // ─── Edit mode (slice 6) ─────────────────────────────────────────────────────
  // The LIVE draft proxy being edited (null = add mode). Held directly — not
  // re-found by id — so REMOVE doesn't flip the body back to add mode during
  // the await between draft removal and close. Field reads stay reactive (the
  // object is a proxy out of rec.draftMcpServers).
  let editServer = $state.raw<McpServerConfig | null>(null);
  const edit = $derived(editServer !== null);
  const secretSaved = $derived(!!(editServer && aiRuntime.mcpSecretSavedById[editServer.id]));

  // Map an existing connector back to its catalog preset (lede/chips/secret
  // label): the draft id is the preset id or its slugger-suffixed variant
  // ("github", "github-2", …) AND the transport matches. No match = the
  // Custom full-form body (custom/legacy connectors).
  function presetForServer(s: McpServerConfig): McpPreset | null {
    return (
      MCP_PRESETS.find(
        (p) =>
          s.transport === (p.kind === "hosted" ? "http" : "stdio") &&
          (s.id === p.id || new RegExp(`^${p.id}-\\d+$`).test(s.id)),
      ) ?? null
    );
  }

  function startEdit(id: string): void {
    const s = rec.draftMcpServers.find((x) => x.id === id);
    if (!s) return; // stale id → fall back to the add-mode catalog
    editServer = s;
    selected = presetForServer(s);
    step = "connect";
    token = "";
    submitError = null;
    advOpen = false;
    advName = s.label;
    advUrl = s.url ?? "";
    advCommand = s.command ?? "";
    advArgs = s.args.join(" ");
    if (selected?.kind === "local") probeNode();
  }

  // Edit mode: ADVANCED inputs write through to the live draft on input, so
  // field edits ride the ai_runtime autosave (SAVE without a token just closes).
  function syncAdvToDraft(): void {
    const s = editServer;
    if (!s) return; // add mode: overrides are applied at submit instead
    s.label = advName;
    if (s.transport === "http") {
      s.url = advUrl.trim();
    } else {
      s.command = advCommand.trim();
      const args = advArgs.trim();
      s.args = args ? args.split(/\s+/) : [];
    }
  }

  // ─── Node probe (local presets) — once per component, cached ────────────────
  // undefined = not probed yet, string = found version, null = missing.
  let nodeVersion = $state<string | null | undefined>(undefined);
  let nodeProbed = false;
  function probeNode(): void {
    if (nodeProbed) return;
    nodeProbed = true;
    void aiRuntime.checkNode().then((v) => {
      nodeVersion = v;
    });
  }

  // ─── Step transitions ────────────────────────────────────────────────────────
  function selectPreset(p: McpPreset): void {
    selected = p;
    step = "connect";
    submitError = null;
    token = "";
    advOpen = false;
    advName = p.label;
    advUrl = p.url ?? "";
    advCommand = p.command ?? "";
    advArgs = (p.args ?? []).join(" ");
    if (p.kind === "local") probeNode();
    void tick().then(focusFirstField);
  }

  function selectCustom(): void {
    selected = null;
    step = "connect";
    submitError = null;
    token = "";
    customModel = emptyCustomModel();
    void tick().then(focusFirstField);
  }

  function backToCatalog(): void {
    if (connecting) return;
    step = "catalog";
    selected = null;
    submitError = null;
    token = "";
    void tick().then(focusFirstField);
  }

  function requestClose(): void {
    if (connecting || removing) return;
    onClose();
  }

  // ─── Step 2 derivations ──────────────────────────────────────────────────────
  const title = $derived.by(() => {
    if (connectServer) return `Connect ${oauthLabel}`;
    if (step === "catalog") return "Pick a service";
    if (edit && editServer) return editServer.label.trim() || editServer.id;
    return selected ? `Connect ${selected.label}` : "Custom connector";
  });
  const eyebrow = $derived.by(() => {
    if (connectServer) return connectMode === "reconnect" ? "Reconnect" : "Connect";
    return edit ? "Edit connector" : "Add connector";
  });
  // A hosted OAuth preset (e.g. Notion): no token field, no verify — it signs in
  // through the browser via the row's Connect flow (slice 8a). slice 8b runs the
  // Connect inline here instead.
  const oauthPreset = $derived(selected?.kind === "hosted" && selected.authMode === "oauth");
  const chipTransport = $derived(selected?.kind === "hosted" ? "HTTP" : "STDIO");
  const chipPath = $derived(
    selected?.kind === "hosted"
      ? advUrl.trim().replace(/^https?:\/\//, "")
      : `${advCommand.trim()} ${advArgs.trim()}`.trim(),
  );

  // ─── In-modal OAuth connect flow (slice 8b) ─────────────────────────────────
  // Two entry doors share ONE derivation: (1) a hosted-OAuth preset picked from
  // the catalog (added on Connect, then authorized in place), and (2) row
  // Connect/Reconnect on an EXISTING connector (`connectId`). The stage is
  // derived from the live store — the `mcp_authorization_changed` → refresh that
  // McpConnectors runs updates `mcpOAuthStateById`, which flips this panel.
  const mcpOAuthStateById = $derived(aiRuntime.mcpOAuthStateById);
  const mcpOAuthErrors = $derived(aiRuntime.mcpOAuthErrors);

  let oauthAttempted = $state(false);
  let oauthSawAuthorizing = $state(false);
  // The id minted when a catalog OAuth preset is added at Connect-click (null for
  // the row path, where `connectId` already names an existing connector).
  let addedOauthId = $state<string | null>(null);

  // The existing connector being (re)connected from its row.
  const connectServer = $derived(
    connectId ? (rec.draftMcpServers.find((s) => s.id === connectId) ?? null) : null,
  );
  // Show the OAuth connect panel when: opened for a row connector, OR step-2 for
  // a hosted-OAuth preset (Custom-OAuth still rides the form + submit()).
  const oauthConnect = $derived(!!connectServer || (step === "connect" && oauthPreset));
  // The id whose live state drives the stage (row id, or the just-added preset id).
  const oauthConnectId = $derived(connectId ?? addedOauthId);
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
    (connectServer?.url ?? (oauthPreset ? advUrl : "")).trim().replace(/^https?:\/\//, ""),
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
        id = c.addMcpServerFromPreset(selected, presetOverrides(selected));
        addedOauthId = id;
        await c.flushAiRuntimeSave();
      } catch (err) {
        submitError = humanizeError(err);
        console.error("[McpConnectorPicker] OAuth add-before-connect failed", err);
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

  const addDisabled = $derived.by(() => {
    if (connecting || removing) return true;
    if (edit) return false; // SAVE without a token is a plain close
    if (selected) return !!selected.secretLabel && token.trim() === "";
    return customModel.transport === "http"
      ? (customModel.url ?? "").trim() === ""
      : (customModel.command ?? "").trim() === "";
  });

  // ─── Draft building ──────────────────────────────────────────────────────────
  function presetOverrides(p: McpPreset): Partial<McpServerConfig> {
    const o: Partial<McpServerConfig> = {};
    // Adding while Node is missing is allowed, but the connector starts
    // disabled (its row carries the warn badge until Node is found).
    if (p.kind === "local" && nodeVersion === null) o.enabled = false;
    const name = advName.trim();
    if (name && name !== p.label) o.label = name;
    if (p.kind === "hosted") {
      const url = advUrl.trim();
      if (url && url !== (p.url ?? "")) o.url = url;
    } else {
      const command = advCommand.trim();
      if (command && command !== (p.command ?? "")) o.command = command;
      const args = advArgs.trim();
      // ponytail: naive whitespace split — preset args carry no quoted values.
      if (args !== (p.args ?? []).join(" ")) o.args = args ? args.split(/\s+/) : [];
    }
    return o;
  }

  function buildCustomDraft(): McpServerConfig {
    const m = customModel;
    const label = m.label.trim();
    const stdio = m.transport === "stdio";
    return {
      id: newMcpServerId(
        label,
        rec.draftMcpServers.map((s) => s.id),
      ),
      label,
      enabled: true,
      transport: stdio ? "stdio" : "http",
      // http auth mode (ADR 0051). Slice 7 lets the form pick OAuth; carry it
      // onto the draft so it persists as http+oauth (else the backend never
      // lists it for OAuth and its Connect flow is unreachable).
      authMode: stdio ? undefined : (m.authMode ?? "bearer"),
      command: stdio ? (m.command ?? "").trim() || null : null,
      args: stdio ? m.args.filter((a) => a.trim() !== "") : [],
      env: stdio ? m.env.filter((e) => e.name.trim() !== "") : [],
      url: stdio ? null : (m.url ?? "").trim() || null,
      secretEnvName: stdio ? (m.secretEnvName ?? "").trim() || null : null,
      enabledTools: null,
    };
  }

  // ─── Validate-on-add (slice 4) ───────────────────────────────────────────────
  async function submit(): Promise<void> {
    if (connecting || addDisabled) return;
    connecting = true;
    submitError = null;
    let id: string | null = null;
    const secret = token.trim();
    // slice 8b: the in-modal Connect flow will authorize here before closing.
    // For now an OAuth connector lands in "needs authorization" — no pasted
    // secret, and no tool-list verify (an unauthorized connector holds no token,
    // so listing tools would fail). The user Connects from its row.
    const oauth = selected
      ? selected.authMode === "oauth"
      : customModel.transport === "http" && customModel.authMode === "oauth";
    // A local preset added while Node is missing can't list tools either — skip
    // the verify and land it disabled instead of failing the add.
    const skipVerify = oauth || (selected?.kind === "local" && nodeVersion === null);
    try {
      id = selected
        ? c.addMcpServerFromPreset(selected, presetOverrides(selected))
        : c.addMcpServerDraft(buildCustomDraft());
      await c.flushAiRuntimeSave();
      if (secret && !oauth) {
        aiRuntime.setMcpSecretInput(id, secret);
        await aiRuntime.saveMcpServerSecret(id);
        // saveMcpServerSecret records failures instead of throwing.
        const secretError = aiRuntime.mcpSecretErrors[id];
        if (secretError) throw new Error(secretError);
      }
      if (!skipVerify) {
        const tools = await invoke<McpToolDescriptor[]>("mcp_list_server_tools", { id });
        onToolsDiscovered?.(id, tools.length);
      }
      onAdded(id);
      onClose();
    } catch (err) {
      submitError = humanizeError(err);
      console.error(`[McpConnectorPicker] verify-on-add failed for connector "${id}"`, err);
      if (id) {
        // Roll back: no orphan draft, no orphan keychain secret. Silent remove
        // (the user never saw the connector land, so no confirm dialog).
        aiRuntime.setMcpSecretInput(id, "");
        await c.removeMcpServer(id, { confirm: false });
      }
    } finally {
      connecting = false;
    }
  }

  // ─── Edit-mode SAVE / REMOVE (slice 6) ──────────────────────────────────────
  // SAVE with a pasted token = save secret + re-verify (same path as add,
  // including the flush). Failure shows the error inline and stays open — NO
  // rollback: the connector already existed. Without a token, field edits
  // already rode autosave, so SAVE is a plain close.
  async function saveEdit(): Promise<void> {
    const s = editServer;
    if (!s || connecting || removing) return;
    const secret = token.trim();
    if (!secret) {
      onClose();
      return;
    }
    connecting = true;
    submitError = null;
    try {
      await c.flushAiRuntimeSave();
      aiRuntime.setMcpSecretInput(s.id, secret);
      await aiRuntime.saveMcpServerSecret(s.id);
      // saveMcpServerSecret records failures instead of throwing.
      const secretError = aiRuntime.mcpSecretErrors[s.id];
      if (secretError) throw new Error(secretError);
      // A disabled connector (or stdio while Node is missing) can't list
      // tools — the secret is saved; skip the verify instead of failing it.
      const skipVerify = !s.enabled || (s.transport === "stdio" && nodeVersion === null);
      if (!skipVerify) {
        const tools = await invoke<McpToolDescriptor[]>("mcp_list_server_tools", { id: s.id });
        onToolsDiscovered?.(s.id, tools.length);
      }
      onClose();
    } catch (err) {
      submitError = humanizeError(err);
      console.error(`[McpConnectorPicker] re-verify failed for connector "${s.id}"`, err);
    } finally {
      connecting = false;
    }
  }

  // REMOVE goes through the existing confirm dialog + immediate keychain
  // deletion; a cancelled dialog leaves the draft in place, so only close when
  // the connector is actually gone.
  async function removeEdit(): Promise<void> {
    const s = editServer;
    if (!s || connecting || removing) return;
    removing = true;
    try {
      await c.removeMcpServer(s.id);
      if (!rec.draftMcpServers.some((x) => x.id === s.id)) onClose();
    } finally {
      removing = false;
    }
  }

  // ─── Overlay focus / open-close mirror (McpToolListModal pattern) ────────────
  let panelEl = $state<HTMLDivElement | null>(null);
  let bodyEl = $state<HTMLDivElement | null>(null);
  let opener: HTMLElement | null = null;
  let wasOpen = false;

  function focusFirstField(): void {
    const inputs = bodyEl?.querySelectorAll<HTMLInputElement>("input") ?? [];
    for (const el of inputs) {
      if (el.offsetParent !== null) {
        el.focus();
        return;
      }
    }
    panelEl?.focus();
  }

  $effect(() => {
    if (open && !wasOpen) {
      opener = document.activeElement as HTMLElement | null;
      oauthAttempted = false;
      oauthSawAuthorizing = false;
      addedOauthId = null;
      if (editId) startEdit(editId);
      panelEl?.focus();
      void tick().then(focusFirstField);
    } else if (!open && wasOpen) {
      const trigger = opener;
      opener = null;
      step = "catalog";
      selected = null;
      editServer = null;
      token = "";
      submitError = null;
      connecting = false;
      removing = false;
      oauthAttempted = false;
      oauthSawAuthorizing = false;
      addedOauthId = null;
      void tick().then(() => trigger?.focus());
    }
    wasOpen = open;
  });
</script>

<svelte:window
  onkeydown={(e) => {
    if (!open) return;
    if (trapTabKey(e, panelEl)) return;
    if (e.key === "Escape") {
      if (connecting || removing) return;
      // OAuth connect (catalog-add or row) and edit dismiss straight out; only a
      // non-oauth catalog-add step steps back to the grid.
      if (step === "connect" && !edit && !oauthConnect) backToCatalog();
      else onClose();
    }
  }}
/>

{#if open}
  <div
    class="cat-modal"
    role="presentation"
    onpointerdown={(e) => {
      if (e.target === e.currentTarget) requestClose();
    }}
  >
    <div
      bind:this={panelEl}
      class="cat-modal__panel"
      role="dialog"
      aria-modal="true"
      aria-labelledby="mcp-picker-title"
      tabindex="-1"
    >
      <header class="cat-modal__header">
        <div>
          <p class="cat-modal__eyebrow">{eyebrow}</p>
          <h2 id="mcp-picker-title">{title}</h2>
        </div>
        <button
          type="button"
          class="cat-modal__close"
          aria-label="Close connector picker"
          disabled={connecting || removing}
          onclick={requestClose}>×</button
        >
      </header>

      <div class="cat-modal__body" bind:this={bodyEl}>
        {#if oauthConnect}
          <!-- OAuth connect panel: catalog-add of a hosted-OAuth preset, or a
               row's Connect/Reconnect. Stage is derived from the live store. -->
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
        {:else if step === "catalog"}
          <McpConnectorCatalog onPick={selectPreset} onPickCustom={selectCustom} />
        {:else}
          <div class="connect">
            {#if selected}
              {@const p = selected}
              <p class="connect__lede">{p.lede}</p>
              <div class="chip-row">
                <span class="chip">{chipTransport}</span>
                <span class="chip chip--path">{chipPath}</span>
              </div>

              {#if p.kind === "local"}
                {#if nodeVersion === undefined}
                  <p class="group-hint">Checking for Node…</p>
                {:else if nodeVersion === null}
                  <p class="group-hint group-hint--warn">
                    Runs locally and needs Node, which wasn't found.
                    <button
                      type="button"
                      class="link-inline"
                      onclick={() => openExternal("https://nodejs.org")}>Install it from nodejs.org →</button
                    >
                  </p>
                {:else}
                  <p class="group-hint node-ok">
                    Runs locally via Node — found <code>node {nodeVersion}</code> ✓
                  </p>
                {/if}
              {/if}

              {#if p.secretLabel}
                <div class="field">
                  <label class="field-label" for="mcp-picker-token">{p.secretLabel}</label>
                  <input
                    id="mcp-picker-token"
                    class="text-input"
                    class:text-input--error={!!submitError}
                    type="password"
                    autocomplete="off"
                    placeholder={edit
                      ? `Unchanged — paste a new ${p.secretLabel.toLowerCase()} to replace it`
                      : `Paste your ${p.secretLabel.toLowerCase()}…`}
                    disabled={connecting}
                    aria-invalid={!!submitError}
                    aria-describedby={submitError ? "mcp-picker-error" : undefined}
                    bind:value={token}
                  />
                  {#if submitError}
                    <p class="error-text" id="mcp-picker-error" role="alert">{submitError}</p>
                  {/if}
                  {#if p.helpUrl}
                    <button
                      type="button"
                      class="link-inline field-help"
                      onclick={() => p.helpUrl && openExternal(p.helpUrl)}>Create one →</button
                    >
                  {/if}
                </div>
                {#if edit && secretSaved}
                  <p class="group-hint"><span class="saved-badge"><IconCheck class="saved-badge__icon" aria-hidden="true" />secret in keychain</span></p>
                {:else}
                  <p class="group-hint">Stored only in the macOS keychain — never in Mnema's settings.</p>
                {/if}
              {/if}

              <div class="adv">
                <button
                  type="button"
                  class="adv__toggle"
                  aria-expanded={advOpen}
                  onclick={() => (advOpen = !advOpen)}
                >
                  <span>Advanced</span>
                  <span class="adv__chev" class:adv__chev--open={advOpen} aria-hidden="true">›</span>
                </button>
                {#if advOpen}
                  <div class="adv__body">
                    <div class="field">
                      <label class="field-label" for="mcp-picker-adv-name">Name</label>
                      <input id="mcp-picker-adv-name" class="text-input" autocomplete="off" bind:value={advName} oninput={syncAdvToDraft} />
                    </div>
                    {#if p.kind === "hosted"}
                      <div class="field">
                        <label class="field-label" for="mcp-picker-adv-url">URL</label>
                        <input id="mcp-picker-adv-url" class="text-input" autocomplete="off" bind:value={advUrl} oninput={syncAdvToDraft} />
                      </div>
                    {:else}
                      <div class="field">
                        <label class="field-label" for="mcp-picker-adv-command">Command</label>
                        <input id="mcp-picker-adv-command" class="text-input" autocomplete="off" bind:value={advCommand} oninput={syncAdvToDraft} />
                      </div>
                      <div class="field">
                        <label class="field-label" for="mcp-picker-adv-args">Arguments</label>
                        <input id="mcp-picker-adv-args" class="text-input" autocomplete="off" placeholder="space-separated" bind:value={advArgs} oninput={syncAdvToDraft} />
                      </div>
                    {/if}
                  </div>
                {/if}
              </div>

              {#if submitError && !p.secretLabel}
                <p class="error-text" role="alert">{submitError}</p>
              {/if}
            {:else}
              <!-- Custom full form (add: scratch model; edit: the LIVE draft). -->
              <McpConnectorCustomForm
                model={editServer ?? customModel}
                {edit}
                {secretSaved}
                {connecting}
                {submitError}
                bind:token
              />
            {/if}

            <div class="connect__footer">
              {#if edit}
                <button
                  class="btn btn--danger btn--sm"
                  type="button"
                  disabled={connecting || removing}
                  onclick={() => void removeEdit()}
                >
                  Remove
                </button>
              {:else}
                <button
                  class="btn btn--ghost btn--sm"
                  type="button"
                  disabled={connecting}
                  onclick={backToCatalog}
                >
                  Back
                </button>
              {/if}
              <button
                class="btn btn--primary"
                type="button"
                disabled={addDisabled}
                aria-busy={connecting}
                onclick={() => void (edit ? saveEdit() : submit())}
              >
                {#if connecting}<ButtonSpinner />Connecting…{:else if edit}Save{:else}Add connector{/if}
              </button>
            </div>
          </div>
        {/if}
      </div>
    </div>
  </div>
{/if}

<style>
  /* ---- Overlay + panel (mirrors McpToolListModal / CategoryDetailModal) ---- */
  .cat-modal {
    position: fixed;
    inset: 0;
    z-index: 2000;
    display: grid;
    place-items: center;
    padding: 24px;
    background: var(--app-overlay-bg);
    backdrop-filter: blur(10px);
  }
  .cat-modal__panel {
    width: min(560px, 100%);
    max-height: min(720px, calc(100vh - 48px));
    display: flex;
    flex-direction: column;
    border: 1px solid var(--app-border-strong);
    border-radius: 18px;
    background: var(--app-surface-raised);
    box-shadow: var(--app-shadow-popover);
  }
  .cat-modal__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 18px 18px 12px;
  }
  .cat-modal__eyebrow {
    margin: 0 0 2px;
    font-size: 10.5px;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .cat-modal__header h2 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
    overflow-wrap: anywhere;
  }
  .cat-modal__close {
    flex: 0 0 auto;
    width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--text-lg);
    line-height: 1;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .cat-modal__close:hover:not(:disabled),
  .cat-modal__close:focus-visible {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
    outline: none;
  }
  .cat-modal__close:focus-visible {
    box-shadow: var(--app-ring);
  }
  .cat-modal__close:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .cat-modal__body {
    overflow-y: auto;
    padding: 0 18px 18px;
  }

  /* ---- Step 2: connect (step-1 styles live in McpConnectorCatalog) ---- */
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
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field .text-input {
    width: 100%;
  }
  .node-ok {
    color: var(--app-accent);
  }
  .link-inline {
    padding: 0;
    border: 0;
    background: transparent;
    font: inherit;
    color: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .field-help {
    width: fit-content;
    font-size: 10.5px;
    color: var(--app-text-muted);
    text-decoration: none;
  }
  .field-help:hover {
    color: var(--app-text);
    text-decoration: underline;
  }

  /* ---- ADVANCED disclosure ---- */
  .adv {
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface);
    overflow: hidden;
  }
  .adv__toggle {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 12px;
    background: transparent;
    border: 0;
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    cursor: pointer;
  }
  .adv__toggle:hover {
    color: var(--app-text-strong);
  }
  .adv__chev {
    font-size: 11px;
    transition: transform 0.15s;
  }
  .adv__chev--open {
    transform: rotate(90deg);
  }
  .adv__body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 2px 12px 14px;
    border-top: 1px solid var(--app-border);
  }

  .connect__footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding-top: 4px;
  }

  @media (prefers-reduced-motion: reduce) {
    .cat-modal__close,
    .adv__chev {
      transition: none;
    }
  }
</style>
