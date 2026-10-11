<script lang="ts">
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import ModelPickerMenu from "$lib/chat/ModelPickerMenu.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ProviderRow from "./ProviderRow.svelte";
  import { runtimeReasonTone } from "./provider-row-state";
  import type { AiProviderKind } from "$lib/types";
  import IconCheck from "~icons/lucide/check";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconInfo from "~icons/lucide/info";
  import IconPlus from "~icons/lucide/plus";
  import IconSparkles from "~icons/lucide/sparkles";

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;

  /** Presence + status + model list, after the vault credential changed. */
  function refreshAfterChatgptChange(): void {
    void aiRuntime.handleChatgptConnectionChange();
    void c.loadSettingsModels();
  }

  // A ChatGPT sign-in lands up to 15 minutes after the click, by which time the
  // provider row (and its ChatgptConnect) may well be collapsed. This panel
  // outlives the row, so it owns the outcome — otherwise the badge and the
  // model picker stay stale until the page is reloaded.
  onMount(() => {
    let unlisten: (() => void) | null = null;
    let destroyed = false;
    void listen<{ connected: boolean }>("chatgpt_login_update", (event) => {
      if (event.payload.connected) refreshAfterChatgptChange();
    }).then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });
    return () => {
      destroyed = true;
      unlisten?.();
    };
  });

  const status = $derived(aiRuntime.aiRuntimeStatus);
  const statusLoading = $derived(aiRuntime.aiRuntimeStatusLoading);
  const testResult = $derived(aiRuntime.aiRuntimeTestResult);
  const testError = $derived(aiRuntime.aiRuntimeTestError);
  const testRunning = $derived(aiRuntime.aiRuntimeTestRunning);
  const reasonLabel = (reason: string | null | undefined) => aiRuntime.aiRuntimeReasonLabel(reason);

  // "Nothing is ready yet" = setup is incomplete. A provider that is set up but
  // failing (reconnect, unreachable) speaks on its own row instead.
  const notReady = $derived(
    rec.draftAiEnabled &&
      !statusLoading &&
      !!status &&
      !status.available &&
      /^(no_providers|no_default_model|provider_not_connected|no_provider_key)/i.test(status.reason ?? ""),
  );

  // One provider row open at a time; a just-added instance opens itself.
  let openId = $state<string | null>(null);
  let addOpen = $state(false);
  let addAnchor = $state<HTMLElement | null>(null);

  function addProvider(kind: AiProviderKind): void {
    addOpen = false;
    const before = rec.draftAiProviders.length;
    c.addAiProvider(kind);
    if (rec.draftAiProviders.length > before) openId = rec.draftAiProviders.at(-1)?.id ?? null;
  }

  const kindList = c.AI_PROVIDER_KINDS.map((k) => c.aiProviderKindLabel(k)).join(", ") + ".";
</script>

<svelte:window
  onclick={(e) => {
    if (addOpen && addAnchor && !addAnchor.contains(e.target as Node)) addOpen = false;
  }}
  onkeydown={(e) => {
    if (addOpen && e.key === "Escape") addOpen = false;
  }}
/>

{#snippet addMenu(small: boolean)}
  <div class="mx-pop-anchor" bind:this={addAnchor}>
    <button
      class="mx-btn"
      class:mx-btn--sm={small}
      type="button"
      aria-haspopup="menu"
      aria-expanded={addOpen}
      disabled={c.aiProviderRemoving}
      onclick={() => (addOpen = !addOpen)}
    ><IconPlus width="13" height="13" aria-hidden="true" />Add provider…</button>
    {#if addOpen}
      <div class="mx-pop" data-open role="menu" aria-label="Add provider">
        <ul class="mx-menu">
          {#each c.AI_PROVIDER_KINDS as kind (kind)}
            <li
              role="menuitem"
              tabindex="0"
              title={c.aiProviderKindDescription(kind)}
              onclick={() => addProvider(kind)}
              onkeydown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  addProvider(kind);
                }
              }}
            >
              {c.aiProviderKindLabel(kind)}
              <span class="mx-menu__meta">{c.isCloudAiProviderKind(kind) ? "cloud" : "local"}</span>
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </div>
{/snippet}

{#snippet modelPicker()}
  <div class="pv-picker pv-ctl">
    <ModelPickerMenu
      label={c.aiModelValue || "Choose a default model"}
      title={c.aiModelValue || "Choose a default model"}
      ariaLabel="Global default model"
      block
      placeholder={rec.draftAiDefaultModel === null}
      disabled={rec.draftAiProviders.length === 0}
      modelPool={c.settingsModelLoader.pool}
      providers={rec.draftAiProviders}
      firstProvider={rec.draftAiDefaultModel?.provider ?? null}
      sentinelLabel="No default model"
      sentinelSelected={rec.draftAiDefaultModel === null}
      selectedProvider={rec.draftAiDefaultModel?.provider ?? null}
      selectedModel={rec.draftAiDefaultModel?.model ?? null}
      loading={c.settingsModelLoader.loading}
      failures={c.settingsModelFailureRows}
      onretry={() => void c.settingsModelLoader.load(c.settingsModelRetryTargets)}
      bind:open={c.aiModelOpen}
      onopen={() => void c.loadSettingsModels()}
      onselect={(engine) => {
        rec.draftAiDefaultModel = engine;
        // The last test-connection line named the previously-tested model;
        // changing the default would leave it misrepresenting the config.
        aiRuntime.resetTestResult();
      }}
    />
  </div>
{/snippet}

{#snippet modelsError()}
  <span class="mx-inline pv-tight" data-tone="warn" role="alert">
    <IconInfo width="13" height="13" aria-hidden="true" />Couldn’t list every provider’s models — you can still type any model id.
  </span>
{/snippet}

<SettingGroup id="settings-section-intelligence" title="Providers" hint="One global default model; every AI feature inherits it.">
  {#if notReady}
    <div class="pv-notice">
      <div class="mx-notice" data-tone="warn" role="status">
        <span class="mx-notice__icon"><IconInfo width="15" height="15" /></span>
        <b class="mx-notice__title">AI features are on, but nothing is ready yet</b>
        <p class="mx-notice__text">{reasonLabel(status?.reason)} Connect a provider and choose a default model below.</p>
      </div>
    </div>
  {/if}

  <SettingRow
    label="Enable AI features"
    description="The master switch for everything AI in Mnema. While off, nothing is sent to any AI model."
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftAiEnabled}
        ariaLabel="Enable AI features"
        onCheckedChange={(checked) => {
          // A "Connection succeeded" line must never sit next to a disabled
          // master switch, so clear it when AI features are turned off.
          if (!checked) aiRuntime.resetTestResult();
        }}
      />
    {/snippet}
  </SettingRow>

  {#if rec.draftAiProviders.length === 0}
    <div class="pv-empty">
      <div class="mx-empty mx-empty--compact">
        <span class="mx-empty__glyph"><IconSparkles width="14" height="14" /></span>
        <b class="mx-empty__title">No providers connected yet</b>
        <p class="mx-empty__text">Connect one, then choose a default model.</p>
        <div class="mx-empty__acts">{@render addMenu(true)}</div>
      </div>
    </div>
  {:else}
    {#each rec.draftAiProviders as provider (provider.id)}
      <ProviderRow
        {provider}
        open={openId === provider.id}
        ontoggle={(open) => (openId = open ? provider.id : null)}
        onchatgptchange={refreshAfterChatgptChange}
      />
    {/each}
    {#if aiRuntime.aiProviderRemovalError}
      <div class="pv-notice">
        <span class="mx-inline" data-tone="danger" role="alert">
          <IconAlert width="13" height="13" aria-hidden="true" />{aiRuntime.aiProviderRemovalError}
        </span>
      </div>
    {/if}
    <SettingRow label="Add a provider" description={kindList}>
      {#snippet control()}{@render addMenu(false)}{/snippet}
    </SettingRow>
  {/if}

  <SettingRow
    label="Global default model"
    description="Used by every AI feature unless it sets its own override."
    full={!!c.settingsModelsError}
    aside={c.settingsModelsError ? modelPicker : undefined}
    control={c.settingsModelsError ? modelsError : modelPicker}
  />

  <SettingRow label="AI runtime" full>
    {#snippet aside()}
      <div class="pv-ctl">
        <button
          class="mx-btn"
          type="button"
          disabled={testRunning || rec.draftAiDefaultModel === null}
          aria-busy={testRunning}
          onclick={() => aiRuntime.runAiRuntimeTestConnection()}
        >
          {#if testRunning}<span class="mx-spin mx-spin--sm"></span>Testing…{:else}Test connection{/if}
        </button>
      </div>
    {/snippet}
    {#snippet control()}
      <div class="pv-runtime" aria-live="polite">
        {#if testRunning}
          <span class="mx-inline"><span class="mx-spin mx-spin--sm"></span>Testing the default model…</span>
        {:else if testError}
          <span class="mx-inline" data-tone="danger" role="alert">
            <IconAlert width="13" height="13" aria-hidden="true" />Test connection failed — {reasonLabel(testError)}
          </span>
        {:else if testResult}
          <span class="mx-inline" data-tone={testResult.ok ? "ok" : "danger"}>
            {#if testResult.ok}<IconCheck width="13" height="13" aria-hidden="true" />{:else}<IconAlert width="13" height="13" aria-hidden="true" />{/if}
            {testResult.message || "Connection succeeded."} {c.aiProviderLabelById(testResult.provider)} · {testResult.model || "(none)"}
          </span>
          {#if testResult.rawJson}
            <details class="pv-raw">
              <summary>Raw response</summary>
              <pre>{testResult.rawJson}</pre>
            </details>
          {/if}
        {:else if statusLoading && !status}
          <span class="mx-inline"><span class="mx-spin mx-spin--sm"></span>Checking providers…</span>
        {:else if status?.available}
          <span class="mx-inline" data-tone="ok">
            <IconCheck width="13" height="13" aria-hidden="true" />Default model {status.defaultModel
              ? `${c.aiProviderLabelById(status.defaultModel.provider)} · ${status.defaultModel.model}`
              : "(none)"} is configured and reachable.
          </span>
        {:else if status}
          <span class="mx-inline" data-tone={runtimeReasonTone(status.reason)}>
            {#if runtimeReasonTone(status.reason) === "danger"}<IconAlert width="13" height="13" aria-hidden="true" />{:else}<IconInfo width="13" height="13" aria-hidden="true" />{/if}
            Isn’t ready — {reasonLabel(status.reason)}
          </span>
        {/if}
        {#if aiRuntime.aiRuntimeStatusError}
          <span class="mx-inline" data-tone="danger" role="alert">
            <IconAlert width="13" height="13" aria-hidden="true" />{aiRuntime.aiRuntimeStatusError}
          </span>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Beside a stacked row's copy, centre the controls on the label + first
     hint line (mockup) without letting them push that line down. */
  .pv-ctl {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    white-space: nowrap;
  }

  :global(.setting-row__aside > .pv-ctl) {
    height: 20px;
    margin: 11px 0 -11px;
  }
  /* A full-width notice / empty state between rows, on the rows' left edge. */
  .pv-notice,
  .pv-empty {
    padding: var(--s-3) 0;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .pv-empty .mx-empty {
    max-width: none;
    padding: 0;
  }

  .pv-picker {
    width: 280px;
  }

  /* Inline lines read as the row's second description line. */
  .pv-tight,
  .pv-runtime {
    margin-top: -8px;
  }

  .pv-runtime {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }

  .pv-raw summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--app-text-subtle);
  }

  .pv-raw pre {
    margin: 6px 0 0;
    max-height: 160px;
    overflow: auto;
    padding: var(--s-2) var(--s-3);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    font: 400 var(--text-sm)/1.4 var(--font-mono);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
