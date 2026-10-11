<!--
  The onboarding AI step's default-model readout (split out of AiSetup.svelte):
  a button that opens a menu of every model a connected engine listed, grouped
  by provider instance. Writes `ai.draftAiDefaultModel` directly.
-->
<script lang="ts">
  import IconChevron from "~icons/lucide/chevron-down";
  import { aiVerificationWord, type AiVerification } from "$lib/onboarding/ai-readiness";
  import type { AiEngineRef, AiProviderConfig } from "$lib/types";

  let {
    ai,
    providers,
  }: {
    ai: {
      draftAiDefaultModel: AiEngineRef | null;
      aiVerifications: Record<string, AiVerification>;
      aiProviderInstanceLabel: (provider: AiProviderConfig) => string;
    };
    providers: AiProviderConfig[];
  } = $props();

  let open = $state(false);

  function modelsOf(id: string): string[] {
    const verification = ai.aiVerifications[id];
    return verification?.status === "live" ? verification.models : [];
  }
  /** Why a provider's group is empty. */
  function emptyPoolReason(id: string): string {
    const verification = ai.aiVerifications[id];
    return verification?.status === "error" ? verification.reason : "no models listed yet";
  }
  function isDefault(providerId: string, model: string): boolean {
    const ref = ai.draftAiDefaultModel;
    return !!ref && ref.provider === providerId && ref.model === model;
  }
  function pick(providerId: string | null, model: string | null): void {
    ai.draftAiDefaultModel = providerId && model ? { provider: providerId, model } : null;
    open = false;
  }
</script>

<div class="mx-pop-anchor">
  <button
    class="mx-btn"
    type="button"
    aria-haspopup="listbox"
    aria-expanded={open}
    disabled={providers.length === 0}
    onclick={() => (open = !open)}
  >
    {ai.draftAiDefaultModel?.model ?? "Choose a default model"}<IconChevron width="13" height="13" />
  </button>
  <div class="mx-pop" data-open={open || undefined} role="listbox" aria-label="Default model">
    <ul class="mx-menu">
      <li role="option" aria-selected={ai.draftAiDefaultModel === null} onclick={() => pick(null, null)}
        onkeydown={(e) => e.key === "Enter" && pick(null, null)} tabindex="0">
        No default model<span class="mx-menu__meta">AI stays off</span>
      </li>
      {#each providers as provider (provider.id)}
        <li class="grp" aria-hidden="true">
          {ai.aiProviderInstanceLabel(provider)}<span class="mx-menu__meta">{aiVerificationWord(ai.aiVerifications[provider.id])}</span>
        </li>
        {#each modelsOf(provider.id) as model (model)}
          <li role="option" aria-selected={isDefault(provider.id, model)} tabindex="0"
            onclick={() => pick(provider.id, model)}
            onkeydown={(e) => e.key === "Enter" && pick(provider.id, model)}>
            {model}
          </li>
        {:else}
          <li class="grp empty" aria-hidden="true">{emptyPoolReason(provider.id)}</li>
        {/each}
      {/each}
    </ul>
  </div>
</div>

<style>
  .mx-pop {
    min-width: 280px;
    max-height: 320px;
    overflow: auto;
  }
  .mx-menu li[aria-selected="true"] {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .mx-menu li.grp {
    cursor: default;
    font: 500 var(--text-xs) / 1 var(--font-mono);
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    background: none;
  }
  .mx-menu li.grp.empty {
    text-transform: none;
    letter-spacing: 0;
  }
</style>
