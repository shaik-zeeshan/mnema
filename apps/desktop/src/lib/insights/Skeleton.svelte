<script lang="ts">
  // Skeleton — the shared loading placeholder primitive for all Insights
  // surfaces (Overview / Subjects / SubjectDetail / Context). A token-driven,
  // subtly-shimmering block that mirrors the shape of the real content so the
  // swap from loading → loaded causes no layout shift.
  //
  // The shimmer reuses the app's existing loading idiom (the quick-recall
  // skeleton sweep): a faint surface base with a translating gradient sheen,
  // built entirely from `var(--app-*)` tokens. Under `prefers-reduced-motion`
  // the sweep is disabled and a gentle opacity pulse stands in.
  //
  // Props:
  //   width   — CSS length (default "100%").
  //   height  — CSS length (defaults per variant).
  //   radius  — CSS length (defaults per variant).
  //   variant — "block" (default) | "text" | "circle".
  //   muted   — render dimmer (for secondary/below-the-fold rows).

  interface Props {
    width?: string;
    height?: string;
    radius?: string;
    variant?: "block" | "text" | "circle";
    muted?: boolean;
  }

  let {
    width,
    height,
    radius,
    variant = "block",
    muted = false,
  }: Props = $props();

  // Per-variant sizing defaults, overridable by explicit props.
  const resolvedHeight = $derived(
    height ?? (variant === "text" ? "11px" : variant === "circle" ? "28px" : "100%"),
  );
  const resolvedWidth = $derived(
    width ?? (variant === "circle" ? resolvedHeight : "100%"),
  );
  const resolvedRadius = $derived(
    radius ??
      (variant === "circle" ? "50%" : variant === "text" ? "5px" : "7px"),
  );
</script>

<span
  class="skeleton"
  class:skeleton--muted={muted}
  style="width:{resolvedWidth}; height:{resolvedHeight}; border-radius:{resolvedRadius};"
  aria-hidden="true"
></span>

<style>
  .skeleton {
    display: block;
    position: relative;
    overflow: hidden;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    flex: 0 0 auto;
  }
  .skeleton--muted {
    opacity: 0.55;
  }

  /* Shimmer sweep — a faint highlight translating left→right. Token-driven via
     color-mix over --app-text-subtle so it tracks the theme. */
  .skeleton::after {
    content: "";
    position: absolute;
    inset: 0;
    transform: translateX(-100%);
    background: linear-gradient(
      90deg,
      transparent,
      color-mix(in srgb, var(--app-text-subtle) 16%, transparent),
      transparent
    );
    animation: insights-skeleton-shimmer 1.4s ease-in-out infinite;
  }

  @keyframes insights-skeleton-shimmer {
    100% {
      transform: translateX(100%);
    }
  }

  /* Reduced motion: drop the sweep, fall back to a gentle opacity pulse. */
  @media (prefers-reduced-motion: reduce) {
    .skeleton::after {
      animation: none;
    }
    .skeleton {
      animation: insights-skeleton-pulse 1.6s ease-in-out infinite;
    }
    @keyframes insights-skeleton-pulse {
      0%,
      100% {
        opacity: 0.5;
      }
      50% {
        opacity: 0.85;
      }
    }
  }
</style>
