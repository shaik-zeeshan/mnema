<script lang="ts">
  // Thin wrapper over the global `.btn` primitive (+layout.svelte, system.css
  // §6). It only composes class names — all styling lives in the global CSS.
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";

  interface Props extends HTMLButtonAttributes {
    /** Visual variant. "primary" is the design's push button. */
    variant?: "default" | "primary" | "ghost" | "danger";
    /** Control height; omit for the default 28px. */
    size?: "sm" | "lg";
    /** Square icon-only button (composable with size="sm"). */
    icon?: boolean;
    /** In-flight state: aria-busy + a leading spinner. */
    busy?: boolean;
    children?: Snippet;
  }

  let {
    variant = "default",
    size,
    icon = false,
    busy = false,
    type = "button",
    class: klass = "",
    children,
    ...rest
  }: Props = $props();
</script>

<button
  {...rest}
  {type}
  class="btn{variant === 'default' ? '' : ` btn--${variant}`}{size
    ? ` btn--${size}`
    : ''}{icon ? ' btn--icon' : ''} {klass}"
  aria-busy={busy || undefined}
>
  {#if busy}
    <span class="btn-spinner" aria-hidden="true"></span>
  {/if}
  {@render children?.()}
</button>

<style>
  /* Self-contained spinner (the settings ButtonSpinner depends on a keyframe
     that only ships with the settings page CSS). */
  .btn-spinner {
    width: 9px;
    height: 9px;
    flex: 0 0 auto;
    border-radius: 50%;
    border: 1.5px solid currentColor;
    border-top-color: transparent;
    opacity: 0.85;
    animation: btn-spin 0.6s linear infinite;
  }
  @keyframes btn-spin {
    to {
      transform: rotate(360deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .btn-spinner {
      animation-duration: 2.4s;
    }
  }
</style>
