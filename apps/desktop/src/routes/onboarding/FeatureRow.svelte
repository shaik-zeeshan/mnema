<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon, { type IconName } from "$lib/settings/Icon.svelte";
  import Switch from "$lib/components/Switch.svelte";

  // One capability row of the onboarding accordion. PRESENTATIONAL only — it
  // owns no `open`/`enabled` state. The PARENT controls `open` (so one-open-at-
  // a-time is the parent's job) and `enabled`; the row just calls `onToggle` /
  // `onExpand`. Styling lives in the global `onboarding-ui.css` (imported once by
  // FeatureStack); the only thing scoped here is the header `id` plumbing.
  interface Props {
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
</script>

<section
  class="row"
  class:open
  class:is-on={armed}
  class:disabled-feature={open && !enabled && !required}
  data-feature-row
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
    <div class="icon-chip"><Icon name={icon} /></div>
    <div class="row-titlewrap">
      <div class="row-eyebrow">{eyebrow}</div>
      <div class="row-name" id={titleId}>{name}</div>
      <div class="row-sub">{sub}</div>
    </div>

    {#if required}
      <span class="row-status row-status--req" title="Required — always on">
        <span class="lock-ico"><Icon name="lock" /></span>Required
      </span>
    {:else}
      <span class="row-status">
        {#if download?.running}
          <!-- A live download takes precedence over the On/Off + Needs-setup /
               lock labels: the row should read "Downloading N%" while fetching.
               Kept shown even when open, to confirm continuity. percent may be
               null (unknown totalBytes) — always render `{percent ?? 0}%`. -->
          <span class="row-dl"
            ><span class="dl-dot"></span>Downloading {download.percent ?? 0}%</span
          >
        {:else}
          <span class="status-dot" class:on={enabled}></span>{enabled
            ? "On"
            : "Off"}
          {#if attention}
            <span class="row-attn"
              ><span class="attn-dot"></span>Needs setup</span
            >
          {:else if !enabled && lockReason}
            <span class="row-lock"
              ><span class="lock-ico"><Icon name="lock" /></span>{lockReason}</span
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
  </div>

  <!-- `hidden` when collapsed so a closed row exposes no empty labelled region. -->
  <div class="row-body" id={bodyId} role="region" aria-labelledby={titleId} hidden={!open}>
    {#if open}
      <div class="body-inner">
        {#if body}{@render body()}{/if}
      </div>
    {/if}
  </div>
</section>
