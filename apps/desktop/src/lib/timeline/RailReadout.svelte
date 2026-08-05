<script lang="ts">
  // ── The position pill — one control, two jobs ─────────────────────────────
  // Direction 01 (Bento Native), page 02: the rail's readout IS the jump
  // control. It sits above the rail, centred at rest, and follows the pointer
  // while the rail is hovered so the time and the thing it reads out are never
  // in two different places. The chevron opens the bento jump panel
  // (`TimelineJumper`, which now renders popover-only and anchors to `el`).
  //
  // Rendering only: every value arrives pre-formatted from the dashboard, and
  // the app icon/name cross-slide on the app-identity key exactly as the old
  // rail tooltip did.
  import { fly } from "svelte/transition";

  interface Props {
    /** App display name, or null when the frame carries no app identity. */
    appLabel: string | null;
    appIconSrc: string | null;
    /** One-or-two letter stand-in when there is no icon asset. */
    appFallback: string;
    /** App identity — re-keys the icon/name cross-slide on an app boundary. */
    appKey: string;
    timeLabel: string;
    dateLabel: string;
    /** +1 = scrubbing toward older frames, -1 = toward newer. */
    flyDirection: 1 | -1;
    flyDurationMs: number;
    /** True while the pointer is over the rail: follow it instead of centring. */
    hovered: boolean;
    /** Pointer x within the rail-wrap, when hovered. */
    x: number | null;
    /** Jump-panel open state (chevron rotation + held background). */
    open: boolean;
    onToggle: () => void;
    el?: HTMLButtonElement | null;
  }

  let {
    appLabel,
    appIconSrc,
    appFallback,
    appKey,
    timeLabel,
    dateLabel,
    flyDirection,
    flyDurationMs,
    hovered,
    x,
    open,
    onToggle,
    el = $bindable(null),
  }: Props = $props();

  const FLY_OFFSET_PX = 9;
</script>

<button
  type="button"
  id="timeline-rail-readout"
  class="readout"
  class:readout--open={open}
  class:readout--following={hovered && x != null}
  style={hovered && x != null ? `left: ${x}px` : "left: 50%"}
  bind:this={el}
  onclick={onToggle}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-controls="timeline-jump-picker"
  aria-label={`${appLabel ?? "Timeline position"} — ${timeLabel} ${dateLabel}. Jump to date and time (J)`}
>
  {#if appLabel}
    <span class="readout__ic aicon aicon--lg" aria-hidden="true">
      {#key appKey}
        <span
          class="readout__ic-inner"
          in:fly={{ x: -flyDirection * FLY_OFFSET_PX, duration: flyDurationMs, opacity: 0 }}
          out:fly={{ x: flyDirection * FLY_OFFSET_PX, duration: flyDurationMs, opacity: 0 }}
        >
          {#if appIconSrc}
            <img src={appIconSrc} alt="" loading="lazy" />
          {:else}
            <span>{appFallback}</span>
          {/if}
        </span>
      {/key}
    </span>
    <span class="readout__app-stack">
      {#key appKey}
        <span
          class="readout__app"
          in:fly={{ x: -flyDirection * FLY_OFFSET_PX, duration: flyDurationMs, opacity: 0 }}
          out:fly={{ x: flyDirection * FLY_OFFSET_PX, duration: flyDurationMs, opacity: 0 }}
        >{appLabel}</span>
      {/key}
    </span>
  {/if}
  <span class="readout__time is-num">{timeLabel}</span>
  <span class="readout__date">{dateLabel}</span>
  <span class="readout__chev" aria-hidden="true">
    <svg viewBox="0 0 12 8" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
      <path d="M1.5 2 6 6.5 10.5 2" />
    </svg>
  </span>
</button>

<style>
  /* `.aicon` / `.is-num`: direction primitives (lib/bento/bento.css, +layout). */
  .readout {
    position: absolute;
    top: 2px;
    transform: translateX(-50%);
    z-index: 6;
    display: flex;
    align-items: center;
    gap: var(--s-8);
    height: 32px;
    padding: 0 var(--s-8) 0 var(--s-6);
    border: 0;
    border-radius: var(--r-lg);
    background: transparent;
    color: var(--app-text-strong);
    white-space: nowrap;
    cursor: pointer;
    transition: background-color var(--dur-quick) var(--ease);
  }

  .readout:hover,
  .readout--open {
    background: var(--app-surface);
  }

  .readout:focus-visible {
    outline: none;
    box-shadow: 0 0 0 3.5px var(--app-accent-glow);
  }

  /* While following the pointer the pill is a readout, not a target: the click
     that matters there belongs to the rail underneath it. */
  .readout--following {
    background: var(--mat-hud);
    backdrop-filter: blur(20px);
    box-shadow: 0 0 0 var(--hairline) var(--menu-edge);
    pointer-events: none;
  }

  .readout__ic {
    position: relative;
    overflow: hidden;
    display: grid;
    background: var(--app-surface-raised);
    color: var(--app-text-strong);
  }

  /* Outgoing + incoming icon share one grid cell so they cross-slide in place
     instead of stacking into two rows. */
  .readout__ic-inner {
    grid-area: 1 / 1;
    width: 100%;
    height: 100%;
    display: grid;
    place-items: center;
  }

  .readout__ic img {
    display: block;
    width: 100%;
    height: 100%;
    padding: 2px;
    box-sizing: border-box;
    border-radius: var(--r-sm);
    object-fit: contain;
  }

  .readout__app-stack {
    min-width: 0;
    display: grid;
    overflow: hidden;
  }

  .readout__app {
    grid-area: 1 / 1;
    font: var(--w-medium) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .readout__time {
    font: var(--w-semi) var(--t-display) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
  }

  .readout__date {
    font: var(--w-regular) var(--t-meta) / var(--lh-meta) var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .readout__chev {
    display: inline-flex;
    align-items: center;
    color: var(--app-text-subtle);
    transition: transform var(--dur-quick) var(--ease);
  }

  .readout__chev svg {
    width: 10px;
    height: 7px;
  }

  .readout--open .readout__chev {
    transform: rotate(180deg);
  }

  /* The pill is pure readout while it tracks the pointer — no affordance. */
  .readout--following .readout__chev {
    opacity: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .readout,
    .readout__chev {
      transition: none;
    }
  }
</style>
