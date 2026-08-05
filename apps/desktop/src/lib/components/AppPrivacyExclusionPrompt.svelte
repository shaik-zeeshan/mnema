<script lang="ts">
  import type { AppPrivacyExclusionController } from "$lib/app-privacy-exclusion.svelte";

  let {
    controller,
    onReview,
  }: {
    controller: AppPrivacyExclusionController;
    onReview: () => void;
  } = $props();
</script>

{#if controller.showSensitiveRecommendationPrompt}
  <section class="sensitive-prompt" aria-label="Sensitive capture recommendation">
    <div class="sensitive-prompt__main">
      <span class="sensitive-prompt__eyebrow">Sensitive Capture Protection</span>
      <strong>
        {controller.pendingRecommendedApps.length} recommended app exclusion{controller.pendingRecommendedApps.length === 1 ? "" : "s"} pending
      </strong>
      <p>Password managers and Apple Passwords can be excluded from future screen capture.</p>
    </div>
    <div class="sensitive-prompt__actions">
      <button
        class="btn btn--primary btn--sm"
        type="button"
        disabled={controller.commandInFlight || controller.promptActionInFlight}
        onclick={() => void controller.applyAllRecommendedPrivacyApps()}
      >
        Add exclusions
      </button>
      <button
        class="btn btn--ghost btn--sm"
        type="button"
        disabled={controller.promptActionInFlight}
        onclick={onReview}
      >
        Review
      </button>
      <button
        class="btn btn--ghost btn--sm"
        type="button"
        disabled={controller.promptActionInFlight}
        onclick={() => void controller.dismissSensitiveRecommendationPrompt()}
      >
        Dismiss
      </button>
    </div>
  </section>
{/if}

<style>
  .sensitive-prompt {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 14px;
    padding: 12px;
    border: 1px solid var(--app-warn-border);
    border-radius: 6px;
    background: var(--app-warn-bg);
  }

  .sensitive-prompt__main {
    flex: 999 1 260px;
    min-width: 0;
    display: grid;
    gap: 3px;
  }

  .sensitive-prompt__eyebrow {
    color: var(--app-warn);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
  }

  .sensitive-prompt__main strong {
    color: var(--app-text-strong);
    font-size: 12px;
    line-height: 1.25;
  }

  .sensitive-prompt__main p {
    margin: 0;
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.4;
  }

  .sensitive-prompt__actions {
    flex: 1 1 220px;
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    justify-content: flex-end;
  }

  /* `.btn` comes from the global design system (+layout.svelte). */
</style>
