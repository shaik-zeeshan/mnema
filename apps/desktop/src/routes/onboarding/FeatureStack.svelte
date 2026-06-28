<script lang="ts">
  import type { Snippet } from "svelte";
  import { onMount } from "svelte";
  // Import the global onboarding stylesheet ONCE here so its body-layout
  // primitives are available to Slice 4 body components (which live in separate
  // files and render with these class names). Mirrors how the settings shell
  // imports lib/settings/settings-*.css from its +page.svelte.
  import "./onboarding-ui.css";
  import "./onboarding-body.css";

  // The accordion chrome: welcome header + scrolling stack (rows via the
  // `children` snippet) + footer. PRESENTATIONAL — it holds no draft state.
  // The PARENT (Slice 3) owns which row is open and renders <FeatureRow> rows
  // into `children`.
  interface Props {
    eyebrow?: string;
    subtitle?: string;
    onCount: number;
    attentionCount: number;
    // Names WHAT is blocking the CTA (mirrors the finale's `finaleBlockReason`),
    // so the disabled "Review & finish" isn't a mystery. Null when nothing blocks.
    blockReason?: string | null;
    // When present, the "N need attention" tally becomes a button that jumps to
    // (opens + scrolls to) the first attention row — making the count actionable.
    onJumpToAttention?: (() => void) | undefined;
    ctaLabel: string;
    ctaDisabled?: boolean;
    onFinish: () => void;
    secondaryLabel?: string;
    onSecondary?: () => void;
    // Surfaced from the controller; a failed load/permission/save would otherwise
    // be silent. Rendered as a `role="alert"` banner in the always-mounted footer.
    errorMessage?: string | null;
    children: Snippet;
  }

  let {
    eyebrow = "Set up mnema",
    subtitle = "Turn on what you want recorded and reasoned over. Required features are locked on — everything else is yours to flip. Open a row to fine-tune it.",
    onCount,
    attentionCount,
    blockReason = null,
    onJumpToAttention = undefined,
    ctaLabel,
    ctaDisabled = false,
    onFinish,
    secondaryLabel,
    onSecondary,
    errorMessage = null,
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
    // The enable Switch (`.switch-track`, a bits-ui `role="switch"` button) sits
    // inside the header. It must own its own Space/Enter/Arrow keys, so bail out
    // before the header navigation below ever sees them — otherwise the header
    // would expand/collapse (or move focus) instead of toggling the feature.
    if (active instanceof Element && active.closest(".switch-track")) return;
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
        // Toggle the focused header (matches FeatureRow): a collapsed row opens,
        // an already-open row collapses. The synthetic click routes through the
        // same guard, so keyboard and pointer behavior stay identical.
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

<div class="onboarding-shell" bind:this={shellEl} aria-label="Mnema onboarding">
  <!-- The eyebrow doubles as this phase's focus/announce target: the page moves
       focus here on the welcome→configure transition (tabindex=-1) and screen
       readers read it as the new region's heading. -->
  <div class="head">
    <div class="eyebrow" tabindex="-1" data-ob-phase-heading>{eyebrow}</div>
    <div class="subtitle">{subtitle}</div>
  </div>

  <div class="content">
    <div class="stack">
      {@render children()}
    </div>
  </div>

  {#if errorMessage}
    <div class="stack-error" role="alert">{errorMessage}</div>
  {/if}

  <div class="footer">
    <div class="hint">
      <span><b>{onCount}</b> features on</span>
      {#if attentionCount > 0}
        <span class="hint-sep" aria-hidden="true">·</span>
        {#if onJumpToAttention}
          <!-- Actionable tally: jumps to (opens + scrolls to) the first blocking
               row, so the disabled CTA's blocker is one click away. -->
          <button type="button" class="hint-jump" onclick={() => onJumpToAttention?.()}>
            {attentionCount} need attention →
          </button>
        {:else}
          <span class="hint-attn">{attentionCount} need attention</span>
        {/if}
      {/if}
      {#if blockReason}
        <!-- Names WHAT is blocking the CTA (mirrors the finale's block hint), so
             the disabled "Review & finish" isn't a terse unexplained count. When
             there is a jump target but no attention chip above (e.g. an invalid
             custom resolution/bitrate, which never enters the attention tally),
             the reason itself becomes the actionable "jump to the blocker" link so
             that case isn't left with a reason but no way to reach the row. -->
        {#if onJumpToAttention && attentionCount === 0}
          <button
            type="button"
            class="hint-jump hint-reason-jump"
            onclick={() => onJumpToAttention?.()}
          >
            {blockReason} →
          </button>
        {:else}
          <span class="hint-reason" role="alert">{blockReason}</span>
        {/if}
      {/if}
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

<style>
  /* Failure banner above the footer — flat row, terminal/green danger tokens, so
     a failed load/permission/save isn't silent. Lives outside the two-column
     footer flex to avoid disturbing its hint/actions layout. */
  .stack-error {
    flex: 0 0 auto;
    margin: 0 24px 12px;
    padding: 10px 14px;
    font-size: var(--text-sm);
    line-height: 1.5;
    color: var(--app-danger);
    background: var(--app-danger-bg);
    border: 1px solid var(--app-danger-border);
    border-radius: 8px;
  }

  /* Footer hint now carries up to three parts (count · attention · block reason),
     so flex-wrap them with a tight gap instead of the prior single-line text. The
     base font/color come from the global `.footer .hint` rule. */
  .hint {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px 8px;
  }
  .hint-sep {
    color: var(--app-text-subtle);
  }
  .hint-attn {
    color: var(--app-warn);
  }
  /* The attention tally as an actionable jump button — warn-toned so it reads as
     "something to fix", underlined on hover/focus to advertise it's clickable. */
  .hint-jump {
    font: inherit;
    font-size: 11px;
    color: var(--app-warn);
    background: transparent;
    border: none;
    border-radius: 4px;
    padding: 2px 4px;
    margin: -2px -2px -2px -4px;
    cursor: pointer;
    transition: color 0.15s ease;
  }
  .hint-jump:hover {
    color: var(--app-warn-strong);
    text-decoration: underline;
  }
  .hint-jump:active {
    color: var(--app-warn-strong);
    transform: translateY(1px);
  }
  .hint-jump:focus-visible {
    outline: none;
    text-decoration: underline;
    box-shadow: var(--app-ring);
  }
  /* Names the blocker — full-width below the count line so it never squeezes the
     CTA. Muted (it's guidance, not a failure — the danger banner is separate). */
  .hint-reason {
    flex-basis: 100%;
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  /* Block reason rendered as a jump link (the custom-value case, which has no
     attention chip to carry the jump). Full-width like `.hint-reason`, warn-toned
     + underline-on-hover like `.hint-jump` so it reads as the actionable target. */
  .hint-reason-jump {
    flex-basis: 100%;
    text-align: left;
    line-height: 1.5;
    margin: -2px -4px;
    padding: 2px 4px;
  }
</style>
