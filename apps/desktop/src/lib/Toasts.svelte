<script lang="ts">
  import { fade } from "svelte/transition";
  import { dismissToast, toasts } from "$lib/toast.svelte";

  // Errors keep an explicit Dismiss (they never expire on their own); a timed
  // info/success toast only grows a button row when it carries an action.
  function hasActions(tone: string, hasAction: boolean): boolean {
    return tone === "error" || hasAction;
  }
</script>

{#if toasts.visible.length > 0}
  <div class="toast-stack">
    {#each toasts.visible as item (item.id)}
      <div
        class="toast toast--{item.tone}"
        role={item.tone === "error" ? "alert" : "status"}
        aria-live={item.tone === "error" ? "assertive" : "polite"}
        out:fade={{ duration: 150 }}
      >
        <div class="toast__title"><span class="toast__dot"></span>{item.title}</div>
        {#if item.message}
          <div class="toast__message">{item.message}</div>
        {/if}
        {#if hasActions(item.tone, item.action !== undefined)}
          <div class="toast__actions">
            {#if item.action}
              <button
                type="button"
                class="btn btn--sm btn--push"
                onclick={() => {
                  void item.action?.run();
                  dismissToast(item.id);
                }}>{item.action.label}</button
              >
            {/if}
            <button type="button" class="btn btn--sm btn--ghost" onclick={() => dismissToast(item.id)}>
              Dismiss
            </button>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

<style>
  /* system.css §6: bottom-right, stacked, overlays content, never reflows.
     `fixed` rather than the sheet's `absolute` because the stack must clear
     every route's own scroll container, not the nearest positioned ancestor. */
  .toast-stack {
    position: fixed;
    right: var(--pad-window);
    bottom: var(--pad-window);
    z-index: 900;
    display: grid;
    gap: var(--gap-row);
    justify-items: end;
    pointer-events: none;
  }
  /* Bento Native `.toast`: 320px, `--r-xl`, an opaque raised surface and the
     popover shadow — a toast genuinely floats, so a shadow is legitimate here
     (it is not the elevation-by-shadow the direction bans on flat chrome). No
     ring: the shadow alone separates it, and an outline would read as a card. */
  .toast {
    pointer-events: auto;
    display: grid;
    gap: var(--gap-label);
    width: 320px;
    max-width: calc(100vw - 2 * var(--pad-window));
    padding: var(--s-12);
    border-radius: var(--r-xl);
    background: var(--app-surface-raised);
    box-shadow: var(--shadow-popover);
  }
  .toast__title {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .toast__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: 0 0 auto;
    background: var(--app-info);
  }
  .toast--success .toast__dot {
    background: var(--app-accent);
  }
  .toast--error .toast__dot {
    background: var(--app-danger);
  }
  .toast__message {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    letter-spacing: var(--ls-meta);
    color: var(--app-text-muted);
  }
  .toast__actions {
    display: flex;
    gap: var(--s-8);
    margin-top: var(--s-4);
  }
</style>
