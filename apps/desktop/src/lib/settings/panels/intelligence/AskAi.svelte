<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import ModelPickerMenu from "$lib/insights/ModelPickerMenu.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import { ASK_AI_DEFAULT_TOOL_CALL_LIMIT } from "$lib/settings/state/recording.svelte";

  const c = getSettingsController();
  const rec = c.rec;
  const askAi = c.askAi;

  // Store-read aliases.
  const askAiAvailable = $derived(askAi.askAiAvailable);
  const askAiAvailabilityLoading = $derived(askAi.askAiAvailabilityLoading);
  const askAiAvailabilityError = $derived(askAi.askAiAvailabilityError);

  // Controller derived selectors.
  const askAiStatusDetail = $derived(c.askAiStatusDetail);
  const settingsModelLoader = $derived(c.settingsModelLoader);
  const settingsModelFailureRows = $derived(c.settingsModelFailureRows);
  const settingsModelRetryTargets = $derived(c.settingsModelRetryTargets);
  const settingsModelsError = $derived(c.settingsModelsError);

  // Controller helper functions.
  const askAiModelLabel = (value: string) => c.askAiModelLabel(value);
  const loadSettingsModels = () => c.loadSettingsModels();

  // Store action methods.
  const loadAskAiAvailability = () => askAi.loadAskAiAvailability();
</script>

<SettingGroup
  id="settings-section-askAi"
  title="Ask AI"
  hint="Let Quick Recall and Insights Chat answer questions over your redacted capture history. Uses the global default model unless overridden below."
>
  <SettingRow
    label="Enable Ask AI"
    description="Allow Quick Recall and Insights Chat to answer questions over your redacted capture history. Off by default."
    full
  >
    {#snippet control()}
      <div class="ask-ai-stack">
        <Switch bind:checked={rec.draftAskAiEnabled} />
        <div class="privacy-disclosure">
          <p>Ask AI can answer with redacted screen text, audio transcripts, and timeline results from your retained history after redaction.</p>
          <p>Questions and the redacted context needed to answer them run through the providers configured above — a cloud provider with your own key, or a local model that never leaves this machine.</p>
        </div>
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow
    label="Limit tool calls per question"
    description="Cap how many follow-up searches Ask AI can run for one question. Off means no cap."
    full
  >
    {#snippet control()}
      <div class="ask-ai-stack">
        <Switch bind:checked={rec.draftAskAiLimitToolCalls} />
        {#if rec.draftAskAiLimitToolCalls}
          <label class="field-label" for="ask-ai-max-tool-calls">Max tool calls per question</label>
          <input
            id="ask-ai-max-tool-calls"
            class="text-input"
            type="number"
            min="1"
            max="500"
            step="1"
            bind:value={rec.draftAskAiMaxToolCalls}
          />
          <p class="group-hint">
            Each tool call is one brokered query into your redacted capture history. A lower cap bounds how much a single answer can pull; the default is {ASK_AI_DEFAULT_TOOL_CALL_LIMIT}.
          </p>
        {:else}
          <p class="group-hint group-hint--warn">
            No cap: a single question can issue unlimited brokered queries into your retained capture history.
          </p>
        {/if}
      </div>
    {/snippet}
  </SettingRow>

  <SettingRow label="Model override" full divider={false}>
    {#snippet control()}
      <div class="ask-ai-stack">
        <ModelPickerMenu
          label={askAiModelLabel(rec.draftAskAiModel)}
          title={askAiModelLabel(rec.draftAskAiModel)}
          ariaLabel="Ask AI model override"
          block
          disabled={!rec.draftAskAiEnabled}
          modelPool={settingsModelLoader.pool}
          providers={rec.draftAiProviders}
          firstProvider={rec.draftAiDefaultModel?.provider ?? null}
          sentinelLabel="Global default model"
          sentinelTitle="Follows the default model chosen in Providers"
          sentinelSelected={rec.draftAskAiModel === ""}
          selectedProvider={null}
          selectedModel={rec.draftAskAiModel === "" ? null : rec.draftAskAiModel}
          exactIdPerProvider={false}
          loading={settingsModelLoader.loading}
          failures={settingsModelFailureRows}
          onretry={() => void settingsModelLoader.load(settingsModelRetryTargets)}
          bind:open={c.askAiModelOpen}
          onopen={() => void loadSettingsModels()}
          onselect={(engine) => { rec.draftAskAiModel = engine ? engine.model : ""; }}
        />
        {#if settingsModelsError}
          <p class="group-hint group-hint--warn">
            Could not list models — check the providers above (key/base URL or endpoint). You can still type any model id.
          </p>
        {:else}
          <p class="group-hint">
            Optional override for Quick Recall and Chat. "Global default model" follows the default chosen in Providers; a pinned chat thread still wins over this.
          </p>
        {/if}
        <div class="model-status" class:model-status--available={askAiAvailable}>
          <div>
            <div class="model-status__title">Ask AI {askAiAvailable ? "available" : "unavailable"}</div>
            <div class="model-status__meta">{askAiStatusDetail}</div>
          </div>
          <span class="model-status__pill">{askAiAvailable ? "available" : "unavailable"}</span>
        </div>
        {#if askAiAvailabilityError}
          <p class="error-text">{askAiAvailabilityError}</p>
        {/if}
        <div class="row-actions">
          <button class="btn btn--ghost btn--sm" type="button" disabled={askAiAvailabilityLoading} onclick={() => { void loadAskAiAvailability(); void loadSettingsModels(); }}>
            {askAiAvailabilityLoading ? "Checking" : "Refresh"}
          </button>
        </div>
      </div>
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  /* Full-width rows stack a toggle/picker over its disclosure, hints, and the
     bordered status sub-block; the primitives only gap whole rows. */
  .ask-ai-stack {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: 100%;
  }
</style>
