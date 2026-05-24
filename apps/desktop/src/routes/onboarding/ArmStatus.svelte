<script lang="ts">
  // A pending -> armed pill. Reuses the `.chip[data-on]` / `.welcome__pulse`
  // glow vocabulary so a bay coming online reads as the recorder arming a
  // subsystem. Driven by a single boolean from the parent's real draft state.
  let {
    armed,
    pendingLabel = "Standby",
    armedLabel = "Armed",
  }: {
    armed: boolean;
    pendingLabel?: string;
    armedLabel?: string;
  } = $props();
</script>

<span class="arm" class:arm--on={armed} role="status" aria-live="polite">
  <span class="arm__dot" aria-hidden="true"></span>
  <span class="arm__label">{armed ? armedLabel : pendingLabel}</span>
</span>

<style>
  .arm {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    padding: 3px 11px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface);
    color: var(--app-text-faint);
    font-size: 9px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    white-space: nowrap;
    transition: color 0.18s, border-color 0.18s, background 0.18s;
  }
  .arm__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    opacity: 0.5;
  }
  .arm--on {
    color: var(--app-accent);
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
  }
  .arm--on .arm__dot {
    opacity: 1;
    background: var(--app-accent);
    box-shadow: 0 0 0 0 var(--app-accent-glow);
    animation: arm-pulse 1.9s ease-out infinite;
  }
  @keyframes arm-pulse {
    0% { box-shadow: 0 0 0 0 var(--app-accent-glow); }
    70% { box-shadow: 0 0 0 7px transparent; }
    100% { box-shadow: 0 0 0 0 transparent; }
  }

  @media (prefers-reduced-motion: reduce) {
    /* Keep the armed state legible via the static glow instead of the pulse. */
    .arm--on .arm__dot {
      animation: none;
      box-shadow: 0 0 5px var(--app-accent-glow);
    }
  }
</style>
