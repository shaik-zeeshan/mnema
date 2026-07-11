<script lang="ts">
  // Persistent, inline validation feedback for the custom resolution/bitrate
  // fields. A hover-only tooltip hid hard errors behind a pointer gesture (and
  // was invisible to keyboard/touch), so the message is rendered as standing
  // helper text in the SAME danger color family as the field's red border —
  // no more amber/red mismatch. Exposes a stable `id` so the offending field
  // can point at it via aria-describedby/aria-errormessage. Renders nothing
  // when there are no messages. Ships its own styles so it looks identical in
  // settings and onboarding.
  let { messages = [], id }: { messages?: string[]; id?: string } = $props();
</script>

{#if messages.length > 0}
  <p class="field-warning" {id} role="alert" aria-live="polite">
    <span class="field-warning__badge" aria-hidden="true">!</span>
    <span class="field-warning__text">
      {#each messages as message, index}
        {#if index > 0}<br />{/if}{message}
      {/each}
    </span>
  </p>
{/if}

<style>
  .field-warning {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    color: var(--app-danger);
    font-size: 10px;
    line-height: 1.45;
    text-align: left;
  }

  .field-warning__badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 16px;
    height: 16px;
    margin-top: 0.5px;
    border: 1px solid var(--app-danger-border);
    border-radius: 50%;
    background: var(--app-danger-bg);
    color: var(--app-danger);
    font-size: 10px;
    font-weight: 800;
    line-height: 1;
  }

  .field-warning__text {
    min-width: 0;
    padding-top: 1px;
  }
</style>
