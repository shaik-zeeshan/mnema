<script lang="ts">
  import { onDestroy } from "svelte";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import { tip } from "$lib/components/tooltip";

  interface Props {
    onclick: () => void;
    /** Reload in flight — swaps the icon for the kit spinner and disables the button. */
    busy?: boolean;
    /** Disabled for a reason other than an in-flight reload (e.g. saving). */
    disabled?: boolean;
    /** Tooltip text + default accessible label (e.g. "Reload", "Refresh"). */
    title?: string;
    /** Accessible label override; falls back to `title`. */
    label?: string;
  }

  let { onclick, busy = false, disabled = false, title = "Reload", label }: Props = $props();

  // Guarantee a visible busy state on activation. Several callers either pass no `busy`
  // or a flag that flips true→false within a few ms (a local status read), so a
  // `busy`-only spin is an imperceptible one-frame twitch. On click we latch a
  // short minimum spin; the icon keeps spinning while EITHER the latch or a real
  // `busy` is active, so fast reloads still read as a spin and slow ones spin for
  // their full duration.
  const MIN_SPIN_MS = 500;
  let latched = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  const spinning = $derived(latched || busy);

  function handleClick() {
    latched = true;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      latched = false;
      timer = null;
    }, MIN_SPIN_MS);
    onclick();
  }

  onDestroy(() => {
    if (timer) clearTimeout(timer);
  });
</script>

<!-- Kit ghost icon button (settings mockup's mic "Reload"); in flight the icon
     becomes the kit's `.mx-spin` ring, like every other busy control. -->
<button
  class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
  type="button"
  use:tip={title}
  aria-label={label ?? title}
  aria-busy={spinning}
  disabled={disabled || busy}
  onclick={handleClick}
>
  {#if spinning}
    <span class="mx-spin mx-spin--sm" aria-hidden="true"></span>
  {:else}
    <IconRetry width="14" height="14" aria-hidden="true" />
  {/if}
</button>
