<script lang="ts">
  // The Custom connector full form (McpConnectorPicker step 2, no-preset body) —
  // split out of the picker for the 800-line cap. Fields bind DIRECTLY to
  // `model`, a $state proxy: the picker's scratch model in add mode, the LIVE
  // settings draft in edit mode (so field edits ride the ai_runtime autosave).
  // The one secret stays in the parent (`token`, bindable) — it is saved to the
  // keychain on submit, never onto the model.

  import Segmented from "$lib/components/Segmented.svelte";
  import IconCheck from "~icons/lucide/check";
  import type { McpAuthMode, McpServerConfig, McpTransport } from "$lib/types";

  interface Props {
    model: McpServerConfig;
    /** Edit mode: unchanged-token placeholder + keychain badge instead of the generic hint. */
    edit?: boolean;
    /** Edit mode: a secret for this connector already sits in the keychain. */
    secretSaved?: boolean;
    connecting?: boolean;
    submitError?: string | null;
    token: string;
  }

  let {
    model,
    edit = false,
    secretSaved = false,
    connecting = false,
    submitError = null,
    token = $bindable(),
  }: Props = $props();

  const TRANSPORT_OPTIONS = [
    { value: "stdio", label: "stdio (local process)" },
    { value: "http", label: "HTTP (remote)" },
  ];

  const AUTH_OPTIONS = [
    { value: "bearer", label: "Bearer secret" },
    { value: "oauth", label: "OAuth" },
  ];

  // OAuth http hides the pasted-secret field: authorization happens in the
  // browser via the connector's Connect flow (slice 8), not here.
  const oauthHttp = $derived(model.transport === "http" && model.authMode === "oauth");
</script>

<div class="field">
  <label class="field-label" for="mcp-picker-name">Name</label>
  <input id="mcp-picker-name" class="text-input" autocomplete="off" placeholder="e.g. GitHub" bind:value={model.label} />
</div>

<div class="field">
  <span class="field-label">Transport</span>
  <Segmented
    value={model.transport}
    onValueChange={(v) => (model.transport = v as McpTransport)}
    ariaLabel="Transport for the connector"
    options={TRANSPORT_OPTIONS}
  />
</div>

{#if model.transport === "stdio"}
  <div class="field">
    <label class="field-label" for="mcp-picker-command">Command</label>
    <input id="mcp-picker-command" class="text-input" autocomplete="off" placeholder="npx" bind:value={model.command} />
  </div>

  <div class="field">
    <span class="field-label">Arguments</span>
    {#each model.args as _, i (i)}
      <div class="inline-row">
        <input class="text-input" autocomplete="off" placeholder="argument" aria-label="Argument {i + 1}" bind:value={model.args[i]} />
        <button
          class="btn btn--ghost btn--sm"
          type="button"
          aria-label="Remove argument {i + 1}"
          onclick={() => { model.args = model.args.filter((_, idx) => idx !== i); }}
        >Remove</button>
      </div>
    {/each}
    <div class="row-actions">
      <button
        class="btn btn--ghost btn--sm"
        type="button"
        onclick={() => { model.args = [...model.args, ""]; }}
      >+ Argument</button>
    </div>
  </div>

  <div class="field">
    <span class="field-label">Environment variables</span>
    <p class="group-hint">Non-secret values only. The one secret below is delivered separately via the keychain.</p>
    {#each model.env as _, i (i)}
      <div class="inline-row">
        <input class="text-input" autocomplete="off" placeholder="NAME" aria-label="Env var {i + 1} name" bind:value={model.env[i].name} />
        <input class="text-input" autocomplete="off" placeholder="value" aria-label="Env var {i + 1} value" bind:value={model.env[i].value} />
        <button
          class="btn btn--ghost btn--sm"
          type="button"
          aria-label="Remove env var {i + 1}"
          onclick={() => { model.env = model.env.filter((_, idx) => idx !== i); }}
        >Remove</button>
      </div>
    {/each}
    <div class="row-actions">
      <button
        class="btn btn--ghost btn--sm"
        type="button"
        onclick={() => { model.env = [...model.env, { name: "", value: "" }]; }}
      >+ Variable</button>
    </div>
  </div>

  <div class="field">
    <label class="field-label" for="mcp-picker-secret-env">Secret delivered as env var</label>
    <input id="mcp-picker-secret-env" class="text-input" autocomplete="off" placeholder="e.g. GITHUB_TOKEN" bind:value={model.secretEnvName} />
    <p class="group-hint">The keychain secret below is injected into the child process under this env var name.</p>
  </div>
{:else}
  <div class="field">
    <label class="field-label" for="mcp-picker-url">URL</label>
    <input
      id="mcp-picker-url"
      class="text-input"
      class:text-input--empty={(model.url ?? "").trim().length === 0}
      autocomplete="off"
      placeholder="https://mcp.example.com/mcp"
      aria-invalid={(model.url ?? "").trim().length === 0}
      bind:value={model.url}
    />
    {#if (model.url ?? "").trim().length === 0}
      <p class="group-hint group-hint--warn">An HTTP connector needs a URL.</p>
    {/if}
  </div>

  <div class="field">
    <span class="field-label">Authorization</span>
    <Segmented
      value={model.authMode ?? "bearer"}
      onValueChange={(v) => (model.authMode = v as McpAuthMode)}
      ariaLabel="Authorization for the connector"
      options={AUTH_OPTIONS}
    />
    {#if oauthHttp}
      <p class="group-hint">Sign in through your browser when you add this connector — nothing is pasted here. Only the returned token is stored, in your macOS keychain.</p>
    {:else}
      <p class="group-hint">The keychain secret below is sent as an <code>Authorization: Bearer</code> header.</p>
    {/if}
  </div>
{/if}

{#if !oauthHttp}
  <div class="field">
    <label class="field-label" for="mcp-picker-secret">Secret (optional)</label>
    <input
      id="mcp-picker-secret"
      class="text-input"
      class:text-input--error={!!submitError}
      type="password"
      autocomplete="off"
      placeholder={edit ? "Unchanged — paste a new secret to replace it" : "Token, if the server needs one…"}
      disabled={connecting}
      bind:value={token}
    />
  </div>
  {#if edit && secretSaved}
    <p class="group-hint"><span class="saved-badge"><IconCheck class="saved-badge__icon" aria-hidden="true" />secret in keychain</span></p>
  {:else}
    <p class="group-hint">Stored only in the macOS keychain — never in Mnema's settings.</p>
  {/if}
{/if}

{#if submitError}
  <p class="error-text" role="alert">{submitError}</p>
{/if}

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field .text-input {
    width: 100%;
  }
  .inline-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .inline-row .text-input {
    flex: 1 1 auto;
    min-width: 0;
  }
</style>
