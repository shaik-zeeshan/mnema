<script lang="ts" module>
  import { getContext, setContext } from "svelte";
  import type { Snippet } from "svelte";

  // Lets a body component (rendered INSIDE the inert `.body-inner` via the `body`
  // snippet) hoist its unmet-prerequisite callout OUT of the inert subtree, so the
  // "Grant access" / "Turn on…" UNLOCK button is actually clickable. Without this
  // the one control that promises to unlock the feature is itself inert. The body
  // registers its callout snippet here; FeatureRow renders it as a SIBLING of
  // `.body-inner` (never inert). Keyed by a Symbol so it can't collide.
  const LOCK_CALLOUT_KEY = Symbol("feature-row-lock-callout");

  interface LockCalloutSlot {
    set: (snippet: Snippet | null) => void;
  }

  // Body components call this ONCE at init (it reads context, which Svelte only
  // allows during component initialization) and get back a setter to register or
  // clear their lock-callout snippet from an `$effect`. A no-op fallback keeps
  // bodies usable if ever rendered outside a FeatureRow.
  export function useLockCalloutSlot(): (snippet: Snippet | null) => void {
    const slot = getContext<LockCalloutSlot | undefined>(LOCK_CALLOUT_KEY);
    return (snippet) => slot?.set(snippet);
  }
</script>

<script lang="ts">
  import type { IconName } from "$lib/settings/groups";
  import { SECTION_ICONS } from "$lib/settings/section-icons";
  import Switch from "$lib/components/Switch.svelte";
  import IconChevron from "~icons/lucide/chevron-right";

  const IconLock = SECTION_ICONS.lock;

  // One capability row of the onboarding accordion. PRESENTATIONAL only — it
  // owns no `open`/`enabled` state. The PARENT controls `open` (so one-open-at-
  // a-time is the parent's job) and `enabled`; the row just calls `onToggle` /
  // `onExpand`. Styling lives in the global `onboarding-ui.css` (imported once by
  // FeatureStack); the only thing scoped here is the header `id` plumbing.
  interface Props {
    // The feature id, stamped onto the row as `data-feature-id` so the controller
    // can jump-scroll to a specific row (the footer's "jump to first attention").
    featureId?: string;
    icon: IconName;
    name: string;
    eyebrow: string;
    sub: string;
    required?: boolean;
    enabled: boolean;
    open: boolean;
    attention?: boolean;
    // The toggle is disabled (can't be turned ON) until a prerequisite is met;
    // `lockReason` is the human "why" shown in the collapsed status block.
    toggleDisabled?: boolean;
    lockReason?: string | null;
    // Live model-download status for this feature's row. When `running`, a
    // compact "Downloading… N%" badge is shown (even while open) so a download
    // started here stays visible after navigating to another feature.
    download?: { running: boolean; percent: number | null } | null;
    onToggle: () => void;
    onExpand: () => void;
    body?: Snippet;
  }

  let {
    featureId,
    icon,
    name,
    eyebrow,
    sub,
    required = false,
    enabled,
    open,
    attention = false,
    toggleDisabled = false,
    lockReason = null,
    download = null,
    onToggle,
    onExpand,
    body,
  }: Props = $props();

  // The feature's chip glyph, resolved from the shared Lucide map.
  const RowIcon = $derived(SECTION_ICONS[icon]);

  // Stable ids so the header button can `aria-controls` the body region and the
  // body region can be `aria-labelledby` the row title (a11y). Slug from `name`
  // (unique across FEATURES) so screen readers announce the right pairing.
  const slug = $derived(name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/(^-|-$)/g, ""));
  const bodyId = $derived(`feature-row-body-${slug}`);
  const titleId = $derived(`feature-row-title-${slug}`);

  // The icon chip tints to accent when the row is open OR the feature is armed
  // (required rows are always armed; optional rows when enabled).
  let armed = $derived(required || enabled);

  // Header click: toggle this row open/closed, and never react when the click
  // landed on the toggle. The shared Switch renders `.switch-track` (bits-ui),
  // so a click inside it is isolated here — guarantees the enable toggle and the
  // expand/collapse stay independent even though the Switch sits in the header.
  // `onExpand` routes to the controller, which toggles (re-click collapses).
  function onHeadClick(event: MouseEvent) {
    const target = event.target as Element | null;
    if (target?.closest(".switch-track")) return;
    onExpand();
  }

  function onSwitchChange() {
    if (required) return; // locked — never toggles
    if (toggleDisabled) return; // prerequisite unmet — enabling is gated
    onToggle();
  }

  // The body's hoisted lock-callout (registered via `useLockCalloutSlot`),
  // rendered OUTSIDE the inert `.body-inner` so its unlock button is clickable.
  let lockCallout = $state<Snippet | null>(null);
  setContext<LockCalloutSlot>(LOCK_CALLOUT_KEY, {
    set: (snippet) => (lockCallout = snippet),
  });
</script>

<section
  class="row"
  class:open
  class:is-on={armed}
  class:needs-attention={required && attention}
  class:disabled-feature={open && !enabled && !required}
  data-feature-row
  data-feature-id={featureId}
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- A `role="button"` DIV, not a real <button>: the enable Switch is a nested
       bits-ui `role="switch"` button and a <button>-in-<button> is invalid HTML
       (and lets header keys steal the Switch's Space/Enter). Keyboard handling
       for the header is centralized in FeatureStack's window capture-phase
       listener (WKWebView does not reliably focus/keydown on per-element button
       handlers); it calls `head.click()`, which fires onclick on this div too. -->
  <div
    class="row-head"
    data-feature-head
    role="button"
    tabindex="0"
    aria-expanded={open}
    aria-controls={bodyId}
    onclick={onHeadClick}
  >
    <div class="icon-chip"><RowIcon aria-hidden="true" /></div>
    <div class="row-titlewrap">
      <div class="row-eyebrow">{eyebrow}</div>
      <div class="row-name" id={titleId}>{name}</div>
      <div class="row-sub">{sub}</div>
    </div>

    {#if required}
      <span
        class="row-status row-status--req"
        title={attention
          ? "Required — a permission still needs to be granted."
          : "Required — always on"}
      >
        <span class="lock-ico"><IconLock aria-hidden="true" /></span>Required
      </span>
      {#if attention}
        <!-- A required feature can still need setup (e.g. an ungranted capture
             permission). Without this the row read identically to a satisfied
             required row — the warn chip + row accent (`needs-attention`) make
             the outstanding action visible. Kept shown even when the row is open
             so it stays legible while the user resolves it. -->
        <span
          class="row-attn row-attn--req"
          title="A required permission still needs to be granted before recording can start."
        >
          <span class="attn-dot"></span>Needs setup
        </span>
      {/if}
    {:else}
      <span class="row-status">
        {#if download?.running}
          <!-- A live download takes precedence over the On/Off + Needs-setup /
               lock labels: the row should read "Downloading N%" while fetching.
               Kept shown even when open, to confirm continuity. percent may be
               null (unknown totalBytes) — show an indeterminate "Downloading…"
               (the pulsing dot alone carries progress) rather than a misleading
               "0%" that reads as a stalled/failed fetch. -->
          <span class="row-dl"
            ><span class="dl-dot"></span>{download.percent === null
              ? "Downloading…"
              : `Downloading ${download.percent}%`}</span
          >
        {:else}
          <span class="status-dot" class:on={enabled}></span>{enabled
            ? "On"
            : "Off"}
          {#if attention}
            <!-- Warn chip (active blocker): the feature is ON but something it
                 needs is unresolved. Distinct from the muted lock chip below via
                 warn color + an explicit title (the two "not ready" states read
                 alike at a glance otherwise). -->
            <span class="row-attn" title="This feature is on but needs setup before it can run."
              ><span class="attn-dot"></span>Needs setup</span
            >
          {:else if !enabled && lockReason}
            <!-- Muted lock chip (gated/optional): the feature is OFF and can't be
                 turned on until a prerequisite is met — quieter than the warn
                 chip because nothing is actively broken yet. -->
            <span class="row-lock" title="Locked: {lockReason}"
              ><span class="lock-ico"><IconLock aria-hidden="true" /></span><span class="row-lock-text"
                >{lockReason}</span
              ></span
            >
          {/if}
        {/if}
      </span>
    {/if}

    <div class="switch-wrap">
      {#if required}
        <Switch checked={true} disabled={true} />
      {:else}
        <Switch
          checked={enabled}
          disabled={toggleDisabled}
          onCheckedChange={onSwitchChange}
          ariaLabel={`Enable ${name}`}
        />
      {/if}
    </div>

    <!-- Static expand affordance: a muted chevron that rotates open (mirrors the
         nested AdvancedReveal disclosure), so a collapsed row signals it expands
         without relying on hover/cursor hints alone. Decorative — the header's
         aria-expanded already conveys state to assistive tech. -->
    <span class="row-chevron" class:row-chevron--open={open} aria-hidden="true">
      <IconChevron />
    </span>
  </div>

  <!-- `hidden` when collapsed so a closed row exposes no empty labelled region. -->
  <div class="row-body" id={bodyId} role="region" aria-labelledby={titleId} hidden={!open}>
    {#if open}
      <!-- The unmet-prerequisite callout (registered by the body via
           `useLockCalloutSlot`) is rendered HERE, as a sibling of `.body-inner`
           and OUTSIDE its `inert` subtree, so its "Grant access" / "Turn on…"
           UNLOCK button is actually clickable. Rendering it inside the inert body
           (where the body component physically runs) made the one control that
           unlocks the feature itself inert. -->
      {#if lockCallout}
        <div class="body-callout">{@render lockCallout()}</div>
      {/if}
      <!-- When the feature is OFF but expanded, its body is visually dimmed
           (`.disabled-feature` in onboarding-body.css). `pointer-events: none`
           blocks the mouse but NOT the keyboard, so without `inert` the dimmed
           draft controls / model-download buttons stay tab-reachable and
           keyboard-activatable. `inert` removes the whole subtree from the tab
           order and blocks activation, matching the visual dim. Same condition
           that drives `disabled-feature` on the row above. The hoisted callout
           above is deliberately NOT a descendant of this inert wrapper. -->
      <div class="body-inner" inert={open && !enabled && !required}>
        {#if body}{@render body()}{/if}
      </div>
    {/if}
  </div>
</section>
