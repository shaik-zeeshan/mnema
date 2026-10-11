<script lang="ts">
  // One connected provider instance (ADR 0035): a compact row — status word +
  // Manage — that expands in place to its label, endpoint and key fields.
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ChatgptConnect from "$lib/components/ChatgptConnect.svelte";
  import { baseUrlHost } from "$lib/settings/state/ai-providers";
  import { providerRowState } from "./provider-row-state";
  import type { AiProviderConfig } from "$lib/types";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconCloudOff from "~icons/lucide/cloud-off";
  import IconInfo from "~icons/lucide/info";

  let {
    provider,
    open,
    ontoggle,
    onchatgptchange,
  }: {
    /** The draft instance — a `$state` proxy, so field binds autosave. */
    provider: AiProviderConfig;
    open: boolean;
    ontoggle: (open: boolean) => void;
    onchatgptchange: () => void;
  } = $props();

  const c = getSettingsController();
  const aiRuntime = c.aiRuntime;

  const cloud = $derived(c.isCloudAiProviderKind(provider.kind));
  const keySaved = $derived(!!aiRuntime.aiProviderKeySavedByProvider[provider.id]);
  const keyInput = $derived(aiRuntime.aiProviderKeyInputs[provider.id] ?? "");
  const keyError = $derived(aiRuntime.aiProviderKeyErrors[provider.id]);
  const savingId = $derived(aiRuntime.aiProviderKeySavingProvider);
  const listFailure = $derived(c.settingsModelFailureRows.find((f) => f.provider === provider.id));
  const reason = $derived(aiRuntime.aiRuntimeStatus?.reason ?? null);
  const state = $derived(
    providerRowState({
      provider,
      cloud,
      keySaved,
      runtimeReason: reason,
      listFailed: !!listFailure,
      listed: c.settingsModelLoader.pool.some((m) => m.provider === provider.id),
    }),
  );
  const needsBaseUrl = $derived(provider.kind === "openai_compatible" && provider.baseUrl.trim().length === 0);
  const keyKind = $derived(cloud && provider.kind !== "chatgpt");
  // Pinned open while something here still needs the user: a typed key only
  // reaches the vault on an explicit Save key, so collapsing would hide the
  // unsaved warning; a key error or a missing base URL likewise.
  const pinned = $derived(keyInput.trim().length > 0 || !!keyError || needsBaseUrl);
  const expanded = $derived(open || pinned);
  const host = $derived(
    baseUrlHost(provider.baseUrl || (cloud ? "" : (c.AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? ""))),
  );
  const description = $derived(
    expanded
      ? keyKind
        ? keySaved
          ? "A key is saved — paste a new one to replace it. It goes to the encrypted vault, never a config file."
          : "Paste an API key. It goes to the encrypted vault, never a config file."
        : undefined
      : provider.kind === "chatgpt"
        ? undefined
        : keyKind
          ? [keySaved ? "Key in the encrypted vault" : "No key yet", host].filter(Boolean).join(" · ")
          : `${host} · no key, nothing leaves this Mac`,
  );
  // The expanded fields and ChatGPT's connect block (device code) need the
  // full width; every other row is plain: copy left, state + Manage right.
  const stacked = $derived(expanded || provider.kind === "chatgpt");
  const reasonForThis = $derived(
    reason && /^(needs_reconnect|provider_unreachable):/i.test(reason) &&
      reason.slice(reason.indexOf(":") + 1) === provider.id
      ? reason
      : null,
  );
</script>

{#snippet ctl()}
  <div class="pr-ctl">
    <span class="settings-state {state.tone ? `settings-state--${state.tone}` : ''}">{state.text}</span>
    {#if !pinned}
      <button class="mx-btn" type="button" aria-expanded={expanded} onclick={() => ontoggle(!expanded)}>
        {expanded ? "Done" : "Manage"}
      </button>
    {/if}
  </div>
{/snippet}

{#snippet lines()}
  {#if reasonForThis}
    <span class="mx-inline" data-tone={reasonForThis.toLowerCase().startsWith("needs_") ? "danger" : "warn"} role="status">
      {#if reasonForThis.toLowerCase().startsWith("needs_")}<IconAlert width="13" height="13" aria-hidden="true" />{:else}<IconCloudOff width="13" height="13" aria-hidden="true" />{/if}
      {aiRuntime.aiRuntimeReasonLabel(reasonForThis)}
    </span>
  {:else if listFailure && state.text === "not reachable"}
    <span class="mx-inline" data-tone="warn" role="status">
      <IconCloudOff width="13" height="13" aria-hidden="true" />
      {cloud
        ? `Couldn’t list this provider’s models — ${listFailure.reason}.`
        : `The local endpoint could not be reached. Start ${c.aiProviderKindLabel(provider.kind)}, then retry.`}
      <button
        class="mx-btn mx-btn--ghost mx-btn--sm"
        type="button"
        disabled={c.settingsModelLoader.loading}
        onclick={() => void c.settingsModelLoader.load(c.settingsModelRetryTargets)}
      >Retry</button>
    </span>
  {/if}
{/snippet}

<SettingRow
  label={c.aiProviderInstanceLabel(provider)}
  {description}
  full={stacked}
  aside={stacked ? ctl : undefined}
  control={stacked ? body : ctl}
  below={lines}
>
  {#snippet labelExtra()}<span class="mx-chip">{cloud ? "cloud" : "local"}</span>{/snippet}
</SettingRow>

{#snippet body()}
  <div class="pr-body">
    {#if provider.kind === "chatgpt"}
      <!-- The one cloud kind with no key field: an OAuth device-code login
           stores a token set in the same vault slot, so key presence doubles
           as "connected". -->
      <ChatgptConnect providerId={provider.id} connected={keySaved} onchange={onchatgptchange} />
    {/if}

    {#if expanded}
      <div class="pr-fields">
        <label class="mx-label" for="ai-provider-label-{provider.id}">Label</label>
        <input
          id="ai-provider-label-{provider.id}"
          class="mx-input"
          autocomplete="off"
          placeholder={c.aiProviderKindLabel(provider.kind)}
          bind:value={provider.label}
        />

        {#if provider.kind === "openai_compatible"}
          <label class="mx-label" for="ai-provider-base-url-{provider.id}">Base URL</label>
          <div class="pr-col">
            <input
              id="ai-provider-base-url-{provider.id}"
              class="mx-input pr-mono"
              autocomplete="off"
              placeholder="https://api.fireworks.ai/inference/v1"
              aria-invalid={needsBaseUrl}
              bind:value={provider.baseUrl}
              oninput={() => aiRuntime.resetTestResult()}
            />
            {#if needsBaseUrl}
              <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" aria-hidden="true" />OpenAI-compatible providers need a base URL.</span>
            {/if}
          </div>
        {:else if !cloud}
          <label class="mx-label" for="ai-provider-endpoint-{provider.id}">Endpoint</label>
          <input
            id="ai-provider-endpoint-{provider.id}"
            class="mx-input pr-mono"
            autocomplete="off"
            placeholder={c.AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
            bind:value={provider.baseUrl}
            oninput={() => aiRuntime.resetTestResult()}
          />
        {/if}

        {#if keyKind}
          <label class="mx-label" for="ai-provider-key-{provider.id}">API key</label>
          <div class="pr-col">
            <div class="pr-keyfield">
              <input
                id="ai-provider-key-{provider.id}"
                class="mx-input pr-mono"
                type="password"
                autocomplete="off"
                placeholder={keySaved ? "A key is saved — enter a new one to replace it" : "Paste your provider API key"}
                aria-invalid={!!keyError}
                aria-describedby={keyError ? `ai-provider-key-error-${provider.id}` : undefined}
                disabled={savingId === provider.id}
                bind:value={() => keyInput, (value) => aiRuntime.setProviderKeyInput(provider.id, value)}
              />
              <button
                class="mx-btn"
                type="button"
                disabled={savingId !== null || keyInput.trim().length === 0}
                aria-busy={savingId === provider.id}
                onclick={() => aiRuntime.saveAiProviderKey(provider.id)}
              >
                {#if savingId === provider.id}<span class="mx-spin mx-spin--sm"></span>Saving…{:else}Save key{/if}
              </button>
              <button
                class="mx-btn mx-btn--ghost"
                type="button"
                disabled={savingId !== null || !keySaved}
                onclick={() => aiRuntime.clearAiProviderKey(provider.id)}
              >Clear</button>
            </div>
            {#if keyError}
              <span class="mx-inline" data-tone="danger" id="ai-provider-key-error-{provider.id}" role="alert">
                <IconAlert width="13" height="13" aria-hidden="true" />{keyError}
              </span>
            {:else if keyInput.trim().length > 0 && savingId !== provider.id}
              <span class="mx-inline" data-tone="warn">
                <IconInfo width="13" height="13" aria-hidden="true" />Unsaved API key — click <b>Save key</b> to store it in the vault.
              </span>
            {/if}
          </div>
        {/if}

        <span></span>
        <div>
          <button
            class="mx-btn mx-btn--danger mx-btn--sm"
            type="button"
            disabled={savingId !== null || c.aiProviderRemoving}
            onclick={() => void c.removeAiProvider(provider.id)}
          >Remove provider</button>
        </div>
      </div>
    {/if}
  </div>
{/snippet}

<style>
  .pr-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    white-space: nowrap;
  }

  .pr-body {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: var(--s-2);
    width: 100%;
    min-width: 0;
  }

  .pr-mono {
    font-family: var(--font-mono);
    font-size: var(--text-base);
  }

  .pr-fields {
    display: grid;
    grid-template-columns: 76px minmax(0, 1fr);
    align-items: center;
    gap: var(--s-2) var(--s-3);
    width: 100%;
    max-width: 640px;
    padding-top: var(--s-1);
  }

  .pr-col {
    display: grid;
    gap: 6px;
    min-width: 0;
  }

  .pr-keyfield {
    display: flex;
    gap: var(--s-2);
    min-width: 0;
  }

  .pr-keyfield .mx-input {
    flex: 1;
    min-width: 0;
  }
</style>
