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
    onToggle,
    onExpand,
    body,
  }: Props = $props();

  // The icon chip tints to accent when the row is open OR the feature is armed
  // (required rows are always armed; optional rows when enabled).
  let armed = $derived(required || enabled);

  // Header click: expand only when collapsed, and never when the click landed on
  // the toggle. The shared Switch renders `.switch-track` (bits-ui), so a click
  // inside it is isolated here — guarantees toggle and expand stay independent
  // even though the Switch sits inside the header.
  function onHeadClick(event: MouseEvent) {
    const target = event.target as Element | null;
    if (target?.closest(".switch-track")) return;
    if (open) return; // an already-open header is not a collapse target
    onExpand();
  }

  function onSwitchChange() {
    if (required) return; // locked — never toggles
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
  <!-- Keyboard handling for the header is centralized in FeatureStack's
       window capture-phase listener (WKWebView does not reliably focus/keydown
       on per-element button handlers). -->
  <button
    type="button"
    class="row-head"
    data-feature-head
    aria-expanded={open}
    onclick={onHeadClick}
  >
    <div class="icon-chip"><Icon name={icon} /></div>
    <div class="row-titlewrap">
      <div class="row-eyebrow">{eyebrow}</div>
      <div class="row-name">{name}</div>
      <div class="row-sub">{sub}</div>
    </div>

    {#if required}
      <span class="row-status row-status--req" title="Required — always on">
        <span class="lock-ico"><Icon name="lock" /></span>Required
      </span>
    {:else}
      <span class="row-status">
        <span class="status-dot" class:on={enabled}></span>{enabled
          ? "On"
          : "Off"}
        {#if attention}
          <span class="row-attn"
            ><span class="attn-dot"></span>Needs setup</span
          >
        {/if}
      </span>
    {/if}

    <div class="switch-wrap">
      {#if required}
        <Switch checked={true} disabled={true} />
      {:else}
        <Switch checked={enabled} onCheckedChange={onSwitchChange} />
      {/if}
    </div>
  </button>

  <div class="row-body">
    {#if open}
      <div class="body-inner">
        {#if body}{@render body()}{/if}
      </div>
    {/if}
  </div>
</section>
