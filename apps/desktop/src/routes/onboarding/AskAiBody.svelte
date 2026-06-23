<script lang="ts">
  import type { OnboardingController } from "./onboarding.svelte";
  import { openSettings } from "$lib/surface-windows";

  let { controller }: { controller: OnboardingController } = $props();

  // Onboarding deliberately does NOT duplicate Reasoning Engine configuration.
  // Provider / model / API-key state is not part of `RecordingSettings` (and so
  // not on the OnboardingController), and full config lives in Settings →
  // Intelligence. This body explains what Ask AI does, previews the available
  // engines, and deep-links into Settings to finish setup; the row's enable
  // switch is owned by FeatureRow, not rendered here.
  const enabled = $derived(controller.draftAskAiEnabled);

  const openIntelligenceSettings = (): void => {
    void openSettings("intelligence");
  };

  // Informational only — these mirror the providers the Reasoning Engine
  // supports. The real picker (with model lists + keychain) lives in Settings.
  const ENGINES = [
    { name: "Anthropic", kind: "Cloud", sub: "Claude, with your own API key." },
    { name: "OpenAI", kind: "Cloud", sub: "GPT models, with your own API key." },
    { name: "Ollama", kind: "Local", sub: "Runs on this machine — nothing leaves." },
    { name: "Llamafile", kind: "Local", sub: "Single-file local model server." },
  ] as const;

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
    it on and pick an engine.
  </div>
</div>

<div class="group always-on">
  <div class="group-title">Things you can ask</div>
  <ul class="askai-examples">
    {#each EXAMPLES as example}
      <li><span class="askai-prompt-mark">&gt;</span>{example}</li>
    {/each}
  </ul>
</div>

<div class="group always-on">
  <div class="group-title">Reasoning engine</div>
  <div class="askai-engines">
    {#each ENGINES as engine}
      <div class="engine">
        <div class="engine-head">
          <span class="engine-name">{engine.name}</span>
          <span class="engine-tag" class:engine-tag--local={engine.kind === "Local"}>
            {engine.kind}
          </span>
        </div>
        <div class="engine-sub">{engine.sub}</div>
      </div>
    {/each}
  </div>
  <span class="kbd-hint">
    Cloud engines use a key you provide. Local engines never leave this machine.
  </span>
</div>

<div class="group">
  <div class="note">
    Your API key is stored only in the OS keychain — <b>never in a config file</b>.
  </div>
  <div class="ctl">
    <div class="ctl-label">
      <div class="name">Configure the engine</div>
      <div class="desc">
        {#if enabled}
          Ask AI is on — choose a provider, model, and add credentials to finish.
        {:else}
          Turn Ask AI on above, then pick a provider and add credentials.
        {/if}
      </div>
    </div>
    <div class="ctl-field">
      <button class="btn accent" type="button" onclick={openIntelligenceSettings}>
        Set up in Settings →
      </button>
    </div>
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
    font-size: 12px;
    line-height: 1.5;
    color: var(--app-text);
  }
  .askai-prompt-mark {
    color: var(--app-accent);
    font-weight: 600;
    flex: 0 0 auto;
  }

  /* Engine preview grid — informational cards, not interactive controls. */
  .askai-engines {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .engine {
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .engine-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .engine-name {
    font-size: 12px;
    font-weight: 540;
    color: var(--app-text-strong);
  }
  .engine-tag {
    font-size: 9px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    padding: 2px 7px;
    flex: 0 0 auto;
  }
  .engine-tag--local {
    color: var(--app-text-muted);
    background: var(--app-surface-active);
    border-color: var(--app-border-strong);
  }
  .engine-sub {
    font-size: 11px;
    line-height: 1.45;
    color: var(--app-text-muted);
  }

  @media (max-width: 520px) {
    .askai-engines {
      grid-template-columns: 1fr;
    }
  }
</style>
