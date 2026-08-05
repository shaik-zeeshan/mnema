<script lang="ts">
  import { fade } from "svelte/transition";
  import type { Toast } from "$lib/toast";

  let { toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void } = $props();

  // Motion per system.css §4: fade in instant, fade out --dur-out (150ms);
  // both collapse to 0 under reduced motion.
  const reducedMotion =
    typeof window !== "undefined" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const outDuration = reducedMotion ? 0 : 150;
</script>

<div
  class="toast"
  class:toast--danger={toast.kind === "danger"}
  role={toast.kind === "danger" ? "alert" : "status"}
  in:fade={{ duration: 0 }}
  out:fade={{ duration: outDuration }}
>
  <div class="toast__t">
    <span class="toast__dot toast__dot--{toast.kind}" aria-hidden="true"></span>
    <span class="toast__title">{toast.message}{#if toast.count > 1}&nbsp;<span class="toast__count">×{toast.count}</span>{/if}</span>
    <button
      type="button"
      class="btn btn--ghost btn--sm btn--icon toast__x"
      aria-label="Dismiss"
      onclick={() => onDismiss(toast.id)}
    >
      <svg width="10" height="10" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
        <path d="M2.5 2.5 9.5 9.5" />
        <path d="M9.5 2.5 2.5 9.5" />
      </svg>
    </button>
  </div>
  {#if toast.detail}
    <div class="toast__m">{toast.detail}</div>
  {/if}
  {#if toast.action}
    <div class="toast__row">
      <button type="button" class="btn btn--sm btn--push" onclick={toast.action.run}>
        {toast.action.label}
      </button>
    </div>
  {/if}
</div>

<style>
  /* Toast internals per design frame 14; the .toast base (surface, shadow,
     344px, grid) is global in +layout.svelte. */
  .toast__t {
    display: flex;
    align-items: center;
    gap: var(--gap-inline);
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .toast--danger .toast__t {
    color: var(--app-danger);
  }
  .toast__title {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .toast__count {
    color: var(--app-text-subtle);
    font-weight: var(--w-regular);
  }
  .toast__m {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
    overflow-wrap: anywhere;
  }
  .toast__row {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-top: var(--s-4);
  }
  .toast__x {
    margin-left: auto;
    color: var(--app-text-subtle);
  }
  .toast__dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .toast__dot--success {
    background: var(--app-accent);
  }
  .toast__dot--info {
    background: var(--app-info);
  }
  .toast__dot--danger {
    background: var(--app-danger);
  }
</style>
