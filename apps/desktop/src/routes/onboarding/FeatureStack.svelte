<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  // Import the global onboarding stylesheet ONCE here so its body-layout
  // primitives are available to Slice 4 body components (which live in separate
  // files and render with these class names). Mirrors how the settings shell
  // imports lib/settings/settings-*.css from its +page.svelte.
  import "./onboarding-ui.css";

  // The accordion chrome: welcome header + scrolling stack (rows via the
  // `children` snippet) + footer. PRESENTATIONAL — it holds no draft state.
  // The PARENT (Slice 3) owns which row is open and renders <FeatureRow> rows
  // into `children`.
  interface Props {
    eyebrow?: string;
    subtitle?: string;
    onCount: number;
    attentionCount: number;
    ctaLabel: string;
    ctaDisabled?: boolean;
    onFinish: () => void;
    secondaryLabel?: string;
    onSecondary?: () => void;
    children: Snippet;
  }

  let {
    eyebrow = "Set up mnema",
    subtitle = "Turn on what you want recorded and reasoned over. Required features are locked on — everything else is yours to flip. Open a row to fine-tune it.",
    onCount,
    attentionCount,
    ctaLabel,
    ctaDisabled = false,
    onFinish,
    secondaryLabel,
    onSecondary,
    children,
  }: Props = $props();

  let shellEl: HTMLElement;

  // ── Keyboard navigation (accordion) ──────────────────────────────────────
  // WKWebView does not reliably focus/keydown on <button> via per-element
  // handlers, so we listen on `window` in the CAPTURE phase and act only when
  // the active element is a row header (`[data-feature-head]`). The body's
  // Segmented/Select controls handle their own keys — we never touch them
  // because we early-return unless the focused element IS a header.
  function headers(): HTMLElement[] {
    if (!shellEl) return [];
    return [
      ...shellEl.querySelectorAll<HTMLElement>("[data-feature-head]"),
    ];
  }

  function onKeydown(event: KeyboardEvent) {
    const active = document.activeElement;
    // Only act when focus is on (or within) a row header.
    const head =
      active instanceof Element
        ? (active.closest("[data-feature-head]") as HTMLElement | null)
        : null;
    if (!head || !shellEl.contains(head)) return;

    const list = headers();
    const idx = list.indexOf(head);
    if (idx === -1) return;

    switch (event.key) {
      case "ArrowDown": {
        event.preventDefault();
        list[(idx + 1) % list.length]?.focus();
        break;
      }
      case "ArrowUp": {
        event.preventDefault();
        list[(idx - 1 + list.length) % list.length]?.focus();
        break;
      }
      case "Enter":
      case " ":
      case "Spacebar": {
        // Trigger expand for a focused-but-collapsed header. An already-open
        // header's click is a no-op (matches FeatureRow), so a synthetic click
        // is safe and routes through the same guard.
        event.preventDefault();
        head.click();
        break;
      }
      default:
        break;
    }
  }

  onMount(() => {
    window.addEventListener("keydown", onKeydown, true);
    return () => window.removeEventListener("keydown", onKeydown, true);
  });
</script>

<div class="onboarding-shell" bind:this={shellEl} role="application" aria-label="Mnema onboarding">
  <div class="head">
    <div class="eyebrow">{eyebrow}</div>
    <div class="subtitle">{subtitle}</div>
  </div>

  <div class="content">
    <div class="stack">
      {@render children()}
    </div>
  </div>

  <div class="footer">
    <div class="hint">
      <b>{onCount}</b> features on · {attentionCount} need attention
    </div>
    <div class="footer-actions">
      {#if secondaryLabel}
        <button type="button" class="secondary" onclick={() => onSecondary?.()}>
          {secondaryLabel}
        </button>
      {/if}
      <button type="button" class="cta" disabled={ctaDisabled} onclick={onFinish}>
        {ctaLabel}
      </button>
    </div>
  </div>
</div>
