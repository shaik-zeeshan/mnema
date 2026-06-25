<script lang="ts">
  // Compact validation affordance: a small amber "!" badge that reveals its
  // messages in a tooltip on hover/focus, instead of a persistent box that
  // clutters the control. Renders nothing when there are no messages. Ships its
  // own styles so it looks the same in settings and onboarding.
  let { messages = [] }: { messages?: string[] } = $props();
</script>

{#if messages.length > 0}
  <button
    type="button"
    class="field-warning"
    aria-label={`Invalid input: ${messages.join(" ")}`}
  >
    <span class="field-warning__badge" aria-hidden="true">!</span>
    <span class="field-warning__tip" aria-hidden="true">
      {#each messages as message}
        <span class="field-warning__line">{message}</span>
      {/each}
    </span>
  </button>
{/if}

<style>
  .field-warning {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    padding: 0;
    border: none;
    background: none;
    cursor: help;
    outline: none;
  }

  .field-warning__badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: 1px solid var(--app-warn-border);
    border-radius: 50%;
    background: var(--app-warn-bg);
    color: var(--app-warn);
    font-size: 11px;
    font-weight: 800;
    line-height: 1;
    transition: border-color 0.12s, background 0.12s;
  }

  .field-warning:hover .field-warning__badge,
  .field-warning:focus-visible .field-warning__badge {
    border-color: var(--app-warn);
  }

  .field-warning:focus-visible {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--app-warn) 22%, transparent);
    border-radius: 50%;
  }

  .field-warning__tip {
    position: absolute;
    bottom: calc(100% + 8px);
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: max-content;
    max-width: 240px;
    padding: 8px 10px;
    border: 1px solid var(--app-warn-border);
    border-radius: 6px;
    background: var(--app-surface-raised);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    color: var(--app-warn);
    font-size: 10px;
    line-height: 1.45;
    text-align: left;
    opacity: 0;
    transform: translateY(4px);
    pointer-events: none;
    transition: opacity 0.12s, transform 0.12s;
  }

  .field-warning__tip::after {
    content: "";
    position: absolute;
    top: 100%;
    right: 7px;
    border: 5px solid transparent;
    border-top-color: var(--app-warn-border);
  }

  .field-warning:hover .field-warning__tip,
  .field-warning:focus-visible .field-warning__tip {
    opacity: 1;
    transform: translateY(0);
  }

  .field-warning__line {
    display: block;
  }
</style>
