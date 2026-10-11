<script lang="ts">
  import { getSettingsController } from "$lib/settings/state/controller.svelte";
  import ModelPickerMenu from "$lib/chat/ModelPickerMenu.svelte";
  import Switch from "$lib/components/Switch.svelte";
  import Stepper from "$lib/components/Stepper.svelte";
  import SettingGroup from "$lib/settings/ui/SettingGroup.svelte";
  import SettingRow from "$lib/settings/ui/SettingRow.svelte";
  import {
    ASK_AI_DEFAULT_TOOL_CALL_LIMIT,
    ASK_AI_MAX_TOOL_CALL_LIMIT,
  } from "$lib/settings/state/recording.svelte";
  import IconCheck from "~icons/lucide/check";
  import IconInfo from "~icons/lucide/info";
  import IconAlert from "~icons/lucide/triangle-alert";

  const c = getSettingsController();
  const rec = c.rec;
  const askAi = c.askAi;

  // Readiness is only worth saying while Ask AI is on and a check has landed.
  const checked = $derived(rec.draftAskAiEnabled && !askAi.askAiAvailabilityLoading && askAi.askAiAvailability !== null);
  const unavailable = $derived(checked && !askAi.askAiAvailable);
  const enableLine = $derived((checked && askAi.askAiAvailable) || !!askAi.askAiAvailabilityError);

  const goToProviders = () =>
    document.getElementById("settings-section-intelligence")?.scrollIntoView({ behavior: "smooth", block: "start" });
</script>

{#snippet enableSwitch()}
  <Switch bind:checked={rec.draftAskAiEnabled} ariaLabel="Enable Ask AI" />
{/snippet}

{#snippet enableStatus()}
  {#if askAi.askAiAvailabilityError}
    <span class="mx-inline" data-tone="danger" role="alert"><IconAlert width="13" height="13" aria-hidden="true" />{askAi.askAiAvailabilityError}</span>
  {:else}
    <span class="mx-inline" data-tone="ok"><IconCheck width="13" height="13" aria-hidden="true" />{c.askAiStatusDetail}</span>
  {/if}
{/snippet}

{#snippet modelPicker()}
  <div class="aa-picker">
    <ModelPickerMenu
      label={c.askAiModelLabel(rec.draftAskAiModel)}
      title={c.askAiModelLabel(rec.draftAskAiModel)}
      ariaLabel="Ask AI model override"
      block
      disabled={!rec.draftAskAiEnabled}
      modelPool={c.settingsModelLoader.pool}
      providers={rec.draftAiProviders}
      firstProvider={rec.draftAiDefaultModel?.provider ?? null}
      sentinelLabel="Global default model"
      sentinelTitle="Follows the default model chosen in Providers"
      sentinelSelected={rec.draftAskAiModel === ""}
      selectedProvider={null}
      selectedModel={rec.draftAskAiModel === "" ? null : rec.draftAskAiModel}
      exactIdPerProvider={false}
      loading={c.settingsModelLoader.loading}
      failures={c.settingsModelFailureRows}
      onretry={() => void c.settingsModelLoader.load(c.settingsModelRetryTargets)}
      bind:open={c.askAiModelOpen}
      onopen={() => void c.loadSettingsModels()}
      onselect={(engine) => { rec.draftAskAiModel = engine ? engine.model : ""; }}
    />
  </div>
{/snippet}

{#snippet modelsError()}
  <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" aria-hidden="true" />Couldn’t list models — you can still type any model id.</span>
{/snippet}

{#snippet capControls()}
  {#if rec.draftAskAiLimitToolCalls}
    <Stepper
      id="ask-ai-max-tool-calls"
      bind:value={
        () => String(rec.draftAskAiMaxToolCalls),
        (v) => { rec.draftAskAiMaxToolCalls = parseInt(v, 10) || ASK_AI_DEFAULT_TOOL_CALL_LIMIT; }
      }
      min={1}
      max={ASK_AI_MAX_TOOL_CALL_LIMIT}
      step={1}
      disabled={!rec.draftAskAiEnabled}
    />
  {/if}
  <Switch bind:checked={rec.draftAskAiLimitToolCalls} disabled={!rec.draftAskAiEnabled} ariaLabel="Limit tool calls per question" />
{/snippet}

{#snippet noCap()}
  <span class="mx-inline" data-tone="warn"><IconInfo width="13" height="13" aria-hidden="true" />No cap: one question can run unlimited searches over your retained history.</span>
{/snippet}

<SettingGroup id="settings-section-askAi" title="Ask AI">
  {#if unavailable}
    <div class="aa-notice">
      <div class="mx-notice" data-tone="warn" role="status">
        <span class="mx-notice__icon"><IconInfo width="15" height="15" /></span>
        <b class="mx-notice__title">Ask AI isn’t ready yet</b>
        <p class="mx-notice__text">{c.askAiStatusDetail}</p>
        <div class="mx-notice__acts">
          <button class="mx-btn mx-btn--ghost mx-btn--sm" type="button" onclick={goToProviders}>Go to Providers</button>
        </div>
      </div>
    </div>
  {/if}

  <SettingRow
    label="Enable Ask AI"
    description="Allow Quick Recall and Chat to answer questions over your redacted capture history."
    control={enableSwitch}
    below={enableLine ? enableStatus : undefined}
  />

  <SettingRow
    label="Model override"
    description="Optional override for Quick Recall and Chat. A pinned chat thread still wins."
    control={modelPicker}
    below={c.settingsModelsError ? modelsError : undefined}
  />

  <SettingRow
    label="Limit tool calls per question"
    description="Cap how many follow-up searches Ask AI can run for one question."
    control={capControls}
    below={rec.draftAskAiLimitToolCalls ? undefined : noCap}
  />

  <SettingRow
    label="Fetch pages you visited"
    description="Lets Ask AI re-request a page you visited (its address minus any secrets) to check its current state."
  >
    {#snippet control()}
      <Switch
        bind:checked={rec.draftAskAiWebFetchEnabled}
        disabled={!rec.draftAskAiEnabled}
        ariaLabel="Fetch pages you visited"
      />
    {/snippet}
  </SettingRow>
</SettingGroup>

<style>
  .aa-notice {
    padding: var(--s-3) 0;
    border-bottom: 1px solid var(--mx-hairline);
  }

  .aa-picker {
    width: 280px;
  }
</style>
