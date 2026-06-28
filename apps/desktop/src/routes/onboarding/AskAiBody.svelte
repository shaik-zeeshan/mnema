<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import ModelPickerMenu from "$lib/insights/ModelPickerMenu.svelte";

  let { controller }: { controller: OnboardingController } = $props();

  // Onboarding connects the Reasoning Engine INLINE: the Settings → Intelligence
  // page lives in the main window, which doesn't open until onboarding completes,
  // so we can't deep-link there. The provider list / keys / default model are
  // owned by `controller.ai` (onboarding-ai store) and committed as the
  // `aiRuntime` domain at finish. The row's enable switch is owned by FeatureRow.
  // `controller.ai` / its `aiRuntime` store are stable references, but derive
  // them so reactive reads track correctly (and svelte-check stays quiet).
  const ai = $derived(controller.ai);
  const aiRuntime = $derived(controller.ai.aiRuntime);

  // Keychain key state (per provider instance id).
  const keySaved = $derived(controller.ai.aiRuntime.aiProviderKeySavedByProvider);
  const keySaving = $derived(controller.ai.aiRuntime.aiProviderKeySavingProvider);
  const keyErrors = $derived(controller.ai.aiRuntime.aiProviderKeyErrors);
  const keyInputs = $derived(controller.ai.aiRuntime.aiProviderKeyInputs);
  // Surface a failed keychain clear on provider removal (mirrors Settings →
  // Providers.svelte) — read-only access to the shared ai-runtime state.
  const removalError = $derived(controller.ai.aiRuntime.aiProviderRemovalError);

  const EXAMPLES = [
    "What was that error I hit in the terminal yesterday?",
    "Summarize the meeting I had this morning.",
    "Find the doc where I read about embeddings.",
  ];
</script>

<div class="group always-on">
  <div class="note">
    <b>Ask AI</b> answers questions about everything you've recorded — in plain
    language, grounded in your own redacted history. It stays off until you turn
    it on and connect a reasoning engine below.
  </div>
</div>

<!-- Action-needed callout: Ask AI is on but has no usable reasoning engine yet.
     The WHY (and whether to show it) is the store's single source of truth —
     don't re-derive the condition here. -->
{#if controller.draftAskAiEnabled && !ai.aiConfigReady}
  <div class="group">
    <div class="lock-callout">
      <div class="lock-callout-text">{ai.aiConfigMissing}</div>
    </div>
  </div>
{/if}

<!-- Soft hint for the false-green case: a key is saved but we couldn't list
     models to verify it, so config is nominally "ready" but unconfirmed. -->
{#if ai.aiUnverifiedNote}
  <p class="prov-hint warn">{ai.aiUnverifiedNote}</p>
{/if}

<div class="group always-on">
  <div class="group-title">Things you can ask</div>
  <ul class="askai-examples">
    {#each EXAMPLES as example}
      <li><span class="askai-prompt-mark">&gt;</span>{example}</li>
    {/each}
  </ul>
</div>

<div class="group">
  <div class="group-title">Connect a provider</div>
  <div class="ctl-label">
    <div class="desc">
      Cloud engines (Anthropic, OpenAI) use an API key you provide. Local engines
      (Ollama, Llamafile) run on this machine — no key, nothing leaves.
    </div>
  </div>

  {#if ai.draftAiProviders.length === 0}
    <div class="note muted">
      No engine connected yet. Add one below, then paste a key (cloud) or set the
      endpoint (local), and choose a default model.
    </div>
  {:else}
    <div class="prov-list">
      {#each ai.draftAiProviders as provider (provider.id)}
        <div class="prov-card">
          <div class="prov-head">
            <span class="prov-name">{ai.aiProviderInstanceLabel(provider)}</span>
            <span class="prov-tag" class:prov-tag--local={!ai.isCloudAiProviderKind(provider.kind)}>
              {ai.isCloudAiProviderKind(provider.kind) ? "cloud" : "local"}
            </span>
            {#if ai.isCloudAiProviderKind(provider.kind) && keySaved[provider.id]}
              <span class="prov-saved">✓ key saved</span>
            {/if}
            <button
              class="btn sm prov-remove"
              type="button"
              disabled={keySaving !== null || ai.aiProviderRemoving}
              onclick={() => ai.removeProvider(provider.id)}
            >
              Remove
            </button>
          </div>

          <label class="prov-label" for="ob-ai-label-{provider.id}">Label (optional)</label>
          <input
            id="ob-ai-label-{provider.id}"
            class="input prov-input"
            autocomplete="off"
            placeholder={ai.aiProviderKindLabel(provider.kind)}
            bind:value={provider.label}
          />

          {#if provider.kind === "openai_compatible"}
            <label class="prov-label" for="ob-ai-base-{provider.id}">Base URL</label>
            <input
              id="ob-ai-base-{provider.id}"
              class="input prov-input"
              class:is-error={provider.baseUrl.trim().length === 0}
              autocomplete="off"
              placeholder="https://api.fireworks.ai/inference/v1"
              bind:value={provider.baseUrl}
            />
            {#if provider.baseUrl.trim().length === 0}
              <p class="prov-hint warn">OpenAI-compatible providers need a base URL.</p>
            {/if}
          {:else if !ai.isCloudAiProviderKind(provider.kind)}
            <label class="prov-label" for="ob-ai-ep-{provider.id}">Endpoint (optional)</label>
            <input
              id="ob-ai-ep-{provider.id}"
              class="input prov-input"
              autocomplete="off"
              placeholder={ai.AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind] ?? "http://localhost"}
              bind:value={provider.baseUrl}
            />
            <p class="prov-hint">
              Leave empty for the default {ai.AI_LOCAL_DEFAULT_ENDPOINTS[provider.kind]}. No key, no egress.
            </p>
          {/if}

          {#if ai.isCloudAiProviderKind(provider.kind)}
            <label class="prov-label" for="ob-ai-key-{provider.id}">API key</label>
            <input
              id="ob-ai-key-{provider.id}"
              class="input prov-input"
              class:is-error={!!keyErrors[provider.id]}
              type="password"
              autocomplete="off"
              placeholder={keySaved[provider.id]
                ? "A key is saved — enter a new one to replace it"
                : "Paste your provider API key"}
              disabled={keySaving === provider.id}
              bind:value={
                () => keyInputs[provider.id] ?? "",
                (value) => aiRuntime.setProviderKeyInput(provider.id, value)
              }
            />
            <div class="prov-actions">
              <button
                class="btn sm accent"
                type="button"
                disabled={keySaving !== null || (keyInputs[provider.id] ?? "").trim().length === 0}
                onclick={() => aiRuntime.saveAiProviderKey(provider.id)}
              >
                {keySaving === provider.id ? "Saving…" : "Save key"}
              </button>
              <button
                class="btn sm"
                type="button"
                disabled={keySaving !== null || !keySaved[provider.id]}
                onclick={() => aiRuntime.clearAiProviderKey(provider.id)}
              >
                Clear
              </button>
            </div>
            {#if keyErrors[provider.id]}
              <p class="prov-hint err">{keyErrors[provider.id]}</p>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <div class="prov-add">
    {#each ai.AI_PROVIDER_KINDS as kind (kind)}
      <button
        class="btn sm"
        type="button"
        disabled={ai.aiProviderRemoving}
        title={ai.aiProviderKindDescription(kind)}
        onclick={() => ai.addProvider(kind)}
      >
        + {ai.aiProviderKindLabel(kind)}
      </button>
    {/each}
  </div>

  {#if removalError}
    <p class="prov-hint err">{removalError}</p>
  {/if}

  {#if ai.anyCloudConnected}
    <div class="note">
      <b>Cloud egress:</b> with a cloud provider connected and Ask AI on, redacted
      OCR and transcript text is sent to that provider over HTTPS, billed to your
      own key. Your frames, audio, and assembled dossier never leave this machine.
    </div>
  {/if}
</div>

<div class="group">
  <div class="group-title">Default model</div>
  <div class="prov-model">
    <ModelPickerMenu
      label={ai.aiModelValue || "Choose a default model"}
      title={ai.aiModelValue || "Choose a default model"}
      ariaLabel="Default model"
      block
      placeholder={ai.draftAiDefaultModel === null}
      disabled={ai.draftAiProviders.length === 0}
      modelPool={ai.modelLoader.pool}
      providers={ai.draftAiProviders}
      firstProvider={ai.draftAiDefaultModel?.provider ?? null}
      sentinelLabel="No default model"
      sentinelSelected={ai.draftAiDefaultModel === null}
      selectedProvider={ai.draftAiDefaultModel?.provider ?? null}
      selectedModel={ai.draftAiDefaultModel?.model ?? null}
      loading={ai.modelLoader.loading}
      failures={ai.modelFailureRows}
      onretry={() => void ai.modelLoader.load(ai.modelRetryTargets)}
      bind:open={ai.aiModelOpen}
      onopen={() => void ai.loadModels()}
      onselect={(engine) => { ai.draftAiDefaultModel = engine; }}
    />
  </div>
  {#if ai.modelsError}
    <p class="prov-hint warn">
      Couldn't list every provider's models — check the key, base URL, or endpoint
      above, then retry in the menu. You can still type any model id.
    </p>
    <p class="prov-hint err">{ai.modelsError}</p>
  {:else}
    <p class="prov-hint">
      One merged list across every connected engine. Open the menu to search, or
      type any model id and pick the provider to attribute it to.
    </p>
  {/if}
</div>

<div class="group">
  <div class="note">
    Your API key is stored only in the OS keychain — <b>never in a config file</b>.
    You can change any of this later in Settings → Intelligence.
  </div>
</div>

<style>
  /* Example prompt list — terminal-style "> question" lines. */
  .askai-examples {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .askai-examples li {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: var(--text-base);
    line-height: 1.5;
    color: var(--app-text);
  }
  .askai-prompt-mark {
    color: var(--app-accent);
    font-weight: 600;
    flex: 0 0 auto;
  }

  /* Connected-provider cards (bordered, stacked) + their field labels. The
     `.input` / `.btn` primitives come from the global onboarding stylesheet; the
     rules here are only the provider-row scaffolding the shared sheet lacks. */
  .prov-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .prov-card {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .prov-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 2px;
  }
  .prov-name {
    font-size: var(--text-base);
    font-weight: 540;
    color: var(--app-text-strong);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .prov-tag {
    flex: 0 0 auto;
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    padding: 2px 7px;
  }
  .prov-tag--local {
    color: var(--app-text-muted);
    background: var(--app-surface-active);
    border-color: var(--app-border-strong);
  }
  .prov-saved {
    flex: 0 0 auto;
    font-size: var(--text-xs);
    color: var(--app-accent);
  }
  .prov-remove {
    margin-left: auto;
  }
  .prov-label {
    font-size: var(--text-xs);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    margin-top: 4px;
  }
  .prov-input {
    width: 100%;
    box-sizing: border-box;
  }
  .prov-actions {
    display: flex;
    gap: 8px;
    margin-top: 4px;
  }
  .prov-add {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .prov-model {
    width: 100%;
  }
  .prov-hint {
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--app-text-subtle);
    margin: 0;
  }
  .prov-hint.warn {
    color: var(--app-warn);
  }
  .prov-hint.err {
    color: var(--app-danger);
  }
</style>
