<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import ModelPickerMenu from "$lib/insights/ModelPickerMenu.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import ReloadButton from "$lib/settings/ui/ReloadButton.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const aiRuntime = c.aiRuntime;

  // Re-exported constants the markup references verbatim.
  const AI_PROVIDER_KINDS = c.AI_PROVIDER_KINDS;
  const AI_LOCAL_DEFAULT_ENDPOINTS = c.AI_LOCAL_DEFAULT_ENDPOINTS;

  // Store-read aliases.
  const aiProviderKeySavedByProvider = $derived(aiRuntime.aiProviderKeySavedByProvider);
  const aiProviderKeySavingProvider = $derived(aiRuntime.aiProviderKeySavingProvider);
  const aiProviderKeyErrors = $derived(aiRuntime.aiProviderKeyErrors);
  const aiProviderKeyInputs = $derived(aiRuntime.aiProviderKeyInputs);
  const aiRuntimeStatus = $derived(aiRuntime.aiRuntimeStatus);
  const aiRuntimeStatusLoading = $derived(aiRuntime.aiRuntimeStatusLoading);
  const aiRuntimeStatusError = $derived(aiRuntime.aiRuntimeStatusError);
  const aiRuntimeTestRunning = $derived(aiRuntime.aiRuntimeTestRunning);
  const aiRuntimeTestResult = $derived(aiRuntime.aiRuntimeTestResult);
  const aiRuntimeTestError = $derived(aiRuntime.aiRuntimeTestError);

  // Controller derived selectors.
  const anyCloudAiProviderConnected = $derived(c.anyCloudAiProviderConnected);
  const aiModelValue = $derived(c.aiModelValue);
  const settingsModelLoader = $derived(c.settingsModelLoader);
  const settingsModelFailureRows = $derived(c.settingsModelFailureRows);
  const settingsModelRetryTargets = $derived(c.settingsModelRetryTargets);
  const settingsModelsError = $derived(c.settingsModelsError);

  // Controller helper functions.
  const isCloudAiProviderKind = (k: string) => c.isCloudAiProviderKind(k);
  const aiProviderKindLabel = (k: string) => c.aiProviderKindLabel(k);
  const aiProviderKindDescription = (k: Parameters<typeof c.aiProviderKindDescription>[0]) =>
    c.aiProviderKindDescription(k);
  const aiProviderInstanceLabel = (p: Parameters<typeof c.aiProviderInstanceLabel>[0]) =>
    c.aiProviderInstanceLabel(p);
  const addAiProvider = (k: Parameters<typeof c.addAiProvider>[0]) => c.addAiProvider(k);
  const removeAiProvider = (id: string) => c.removeAiProvider(id);
  const loadSettingsModels = () => c.loadSettingsModels();

  // Store action methods.
  const aiRuntimeReasonLabel = (reason: string | null | undefined) =>
    aiRuntime.aiRuntimeReasonLabel(reason);
  const loadAiRuntimeStatus = () => aiRuntime.loadAiRuntimeStatus();
  const saveAiProviderKey = (p: string) => aiRuntime.saveAiProviderKey(p);
  const clearAiProviderKey = (p: string) => aiRuntime.clearAiProviderKey(p);
  const runAiRuntimeTestConnection = () => aiRuntime.runAiRuntimeTestConnection();
</script>

<SettingGroup
  id="settings-section-intelligence"
  title="Providers"
  hint="Connect the AI providers Mnema can use and choose one global default model. Every AI feature inherits the default; a chat thread or feature override can refine it."
>
  <SettingRow
    label="Enable AI features"
    description="The master switch for everything AI in Mnema. While off, nothing is sent to any AI model. Off by default."
    full
    divider={false}
  >
    {#snippet control()}
      <div class="prov-stack">
        <Switch bind:checked={rec.draftAiEnabled} />
        <div class="privacy-disclosure">
          <p>A cloud provider receives redacted capture text over HTTPS to reason about it — continuous outbound egress and per-token cost billed to your own key.</p>
          <p>A local runtime (Ollama or Llamafile) runs entirely on this machine — nothing is sent anywhere and no API key is needed.</p>
        </div>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup title="Connected providers">
  <SettingRow label="Providers" full divider={false}>
    {#snippet control()}
      <div class="prov-stack">
        {#if rec.draftAiProviders.length === 0}
          <p class="group-hint">No providers connected yet. Connect one below, then choose a default model.</p>
        {:else}
          <ul class="provider-list">
            {#each rec.draftAiProviders as provider (provider.id)}
              <li class="provider-row">
                <div class="provider-row__head">
                  <span class="provider-row__name">{aiProviderInstanceLabel(provider)}</span>
                  <span class="provider-row__tag">{isCloudAiProviderKind(provider.kind) ? "cloud" : "local"}</span>
                  {#if isCloudAiProviderKind(provider.kind) && aiProviderKeySavedByProvider[provider.id]}
                    <span class="saved-badge">✓ key in keychain</span>
                  {/if}
                  <button
                    class="btn btn--ghost btn--sm provider-row__remove"
                    type="button"
                    onclick={() => removeAiProvider(provider.id)}
                  >
                    Remove
                  </button>
                </div>
                <label class="field-label" for="ai-provider-label-{provider.id}">Label</label>
                <input
                  id="ai-provider-label-{provider.id}"
                  class="text-input"
                  autocomplete="off"
                  placeholder={aiProviderKindLabel(provider.kind)}
                  bind:value={provider.label}
                />
                {#if provider.kind === "openai_compatible"}
                  <label class="field-label" for="ai-provider-base-url-{provider.id}">Base URL</label>
                  <input
                    id="ai-provider-base-url-{provider.id}"
                    class="text-input"
                    autocomplete="off"
                    placeholder="https://api.fireworks.ai/inference/v1"
                    bind:value={provider.baseUrl}
                  />
                  {#if provider.baseUrl.trim().length === 0}
                    <p class="group-hint group-hint--warn">OpenAI-compatible providers need a base URL.</p>
                  {/if}
                {:else if !isCloudAiProviderKind(provider.kind)}
                  <label class="field-label" for="ai-provider-endpoint-{provider.id}">Endpoint</label>
                  <input
                    id="ai-provider-endpoint-{provider.id}"
                    class="text-input"
                    autocomplete="off"
                    placeholder={AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
                    bind:value={provider.baseUrl}
                  />
                  <p class="group-hint">Leave empty to use the default endpoint {AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind]}. No key, no egress.</p>
                {/if}
                {#if isCloudAiProviderKind(provider.kind)}
                  <label class="field-label" for="ai-provider-key-{provider.id}">API key</label>
                  <input
                    id="ai-provider-key-{provider.id}"
                    class="text-input"
                    type="password"
                    autocomplete="off"
                    placeholder={aiProviderKeySavedByProvider[provider.id] ? "A key is saved — enter a new one to replace it" : "Paste your provider API key"}
                    disabled={aiProviderKeySavingProvider === provider.id}
                    bind:value={
                      () => aiProviderKeyInputs[provider.id] ?? "",
                      (value) => {
                        aiRuntime.setProviderKeyInput(provider.id, value);
                      }
                    }
                  />
                  <div class="row-actions">
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      disabled={aiProviderKeySavingProvider !== null || (aiProviderKeyInputs[provider.id] ?? "").trim().length === 0}
                      onclick={() => saveAiProviderKey(provider.id)}
                    >
                      {aiProviderKeySavingProvider === provider.id ? "Saving" : "Save key"}
                    </button>
                    <button
                      class="btn btn--ghost btn--sm"
                      type="button"
                      disabled={aiProviderKeySavingProvider !== null || !aiProviderKeySavedByProvider[provider.id]}
                      onclick={() => clearAiProviderKey(provider.id)}
                    >
                      Clear
                    </button>
                  </div>
                  {#if aiProviderKeyErrors[provider.id]}
                    <p class="error-text">{aiProviderKeyErrors[provider.id]}</p>
                  {/if}
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
        <div class="row-actions">
          {#each AI_PROVIDER_KINDS as kind (kind)}
            <button
              class="btn btn--ghost btn--sm"
              type="button"
              title={aiProviderKindDescription(kind)}
              onclick={() => addAiProvider(kind)}
            >
              + {aiProviderKindLabel(kind)}
            </button>
          {/each}
        </div>
        <p class="group-hint">Cloud keys are stored only in the macOS keychain — never in Mnema's settings, config, or save directory. One key per provider instance, shared by every feature. Add a kind more than once to connect several instances (e.g. two OpenAI-compatible servers).</p>
        {#if anyCloudAiProviderConnected}
          <div class="cloud-egress-disclosure" role="note">
            <span class="cloud-egress-disclosure__icon" aria-hidden="true">⚠</span>
            <div class="cloud-egress-disclosure__body">
              <strong>Cloud egress consent</strong>
              <p>
                With a cloud provider connected and AI features on, <em>redacted</em> OCR and
                transcript text is sent to that provider over HTTPS, billed to your own key.
                Your on-device data — frames, audio, and the assembled dossier — never leaves
                this machine. The master switch above is your explicit opt-in, separate from
                the per-feature toggles below.
              </p>
            </div>
          </div>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup title="Default model">
  <SettingRow label="Global default model" full divider={false}>
    {#snippet control()}
      <div class="prov-stack">
        <ModelPickerMenu
          label={aiModelValue || "Choose a default model"}
          title={aiModelValue || "Choose a default model"}
          ariaLabel="Global default model"
          block
          placeholder={rec.draftAiDefaultModel === null}
          disabled={rec.draftAiProviders.length === 0}
          modelPool={settingsModelLoader.pool}
          providers={rec.draftAiProviders}
          firstProvider={rec.draftAiDefaultModel?.provider ?? null}
          sentinelLabel="No default model"
          sentinelSelected={rec.draftAiDefaultModel === null}
          selectedProvider={rec.draftAiDefaultModel?.provider ?? null}
          selectedModel={rec.draftAiDefaultModel?.model ?? null}
          loading={settingsModelLoader.loading}
          failures={settingsModelFailureRows}
          onretry={() => void settingsModelLoader.load(settingsModelRetryTargets)}
          bind:open={c.aiModelOpen}
          onopen={() => void loadSettingsModels()}
          onselect={(engine) => { rec.draftAiDefaultModel = engine; }}
        />
        {#if settingsModelsError}
          <p class="group-hint group-hint--warn">
            Could not list every provider's models — check keys/base URLs/endpoints above, then use Retry in the menu. You can still type any model id.
          </p>
          <p class="error-text">{aiRuntimeReasonLabel(settingsModelsError)}</p>
        {:else}
          <p class="group-hint">
            One merged list across every connected provider. Open the menu to search, or type any model id and pick the provider to attribute it to.
          </p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup title="Status">
  <SettingRow label="AI runtime" full divider={false}>
    {#snippet control()}
      <div class="prov-stack">
        <div class="model-status" class:model-status--available={aiRuntimeStatus?.available}>
          <div>
            <div class="model-status__title">AI {aiRuntimeStatus?.available ? "ready" : "unavailable"}</div>
            <div class="model-status__meta">
              {#if aiRuntimeStatusLoading}
                Checking providers…
              {:else if aiRuntimeStatus?.available}
                Default model {aiRuntimeStatus.defaultModel
                  ? `${aiProviderKindLabel(aiRuntimeStatus.defaultModel.provider)} · ${aiRuntimeStatus.defaultModel.model}`
                  : "(none)"} is configured and reachable.
              {:else}
                {aiRuntimeReasonLabel(aiRuntimeStatus?.reason)}
              {/if}
            </div>
          </div>
          <span class="model-status__pill">{aiRuntimeStatus?.available ? "available" : "unavailable"}</span>
        </div>
        {#if aiRuntimeStatusError}
          <p class="error-text">{aiRuntimeStatusError}</p>
        {/if}
        <div class="row-actions">
          <ReloadButton
            onclick={loadAiRuntimeStatus}
            busy={aiRuntimeStatusLoading}
            title="Refresh"
            label="Refresh AI runtime status"
          />
          <button
            class="btn btn--ghost btn--sm"
            type="button"
            disabled={aiRuntimeTestRunning || rec.draftAiDefaultModel === null}
            onclick={runAiRuntimeTestConnection}
          >
            {aiRuntimeTestRunning ? "Testing" : "Test connection"}
          </button>
        </div>
        <p class="group-hint">
          Test connection runs one structured round trip against the global default model — it
          verifies that provider's key/endpoint even while AI features are off.
        </p>
        {#if aiRuntimeTestResult}
          <div class="cleanup-result" aria-live="polite">
            <strong>{aiRuntimeTestResult.message || "Connection succeeded."}</strong>
            <p>Provider: {aiProviderKindLabel(aiRuntimeTestResult.provider)} · Model: {aiRuntimeTestResult.model || "(none)"}</p>
            {#if aiRuntimeTestResult.rawJson}
              <pre class="ai-runtime-raw">{aiRuntimeTestResult.rawJson}</pre>
            {/if}
          </div>
        {/if}
        {#if aiRuntimeTestError}
          <p class="group-hint group-hint--warn">Test connection failed.</p>
          <p class="error-text">{aiRuntimeTestError}</p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Full-width rows stack the toggle/picker over disclosures, the bordered
     provider cards, status, and test-result sub-blocks; the primitives only
     gap whole rows, so each control slot manages its own internal rhythm. */
  .prov-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
