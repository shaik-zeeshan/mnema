<!-- The pressed pill's popover (recall.html `.rc-pop`): the operators it stands
     for, its alternatives (with result counts once `search.facetCounts` has
     them), Edit as text, and Remove ⌫. The page mounts it at the top of the
     body, in the same grid cell as the results so it overlays them and the
     window still grows to fit it; it hangs under its pill. Keys:
     search-keys.ts `handleLauncherCaptureKeydown`. -->
<script lang="ts">
  import IconCalendar from "~icons/lucide/calendar";
  import IconCheck from "~icons/lucide/check";
  import IconMic from "~icons/lucide/mic";
  import IconMonitor from "~icons/lucide/monitor";
  import IconSpeaker from "~icons/lucide/speaker";
  import IconX from "~icons/lucide/x";
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { pillOperators } from "./filter-chips";
  import { appIcons } from "./app-icons.svelte";

  const filters = search.filters;
  const pill = $derived(filters.pillIndex === null ? null : search.pills[filters.pillIndex]);
  const current = $derived(pill ? pillOperators(pill) : "");

  let el = $state<HTMLElement | null>(null);
  let left = $state(0);

  // Hang under the pressed pill.
  $effect(() => {
    void filters.pillIndex;
    const host = el?.parentElement;
    const tok = document.querySelector('.rc-tok[aria-pressed="true"]');
    if (host && tok) {
      left = Math.max(0, tok.getBoundingClientRect().left - host.getBoundingClientRect().left);
    }
  });

  // A press outside the popover and the pills closes it.
  $effect(() => {
    const onDown = (event: PointerEvent) => {
      if (!(event.target as Element | null)?.closest(".rc-pop, .rc-tok")) filters.closePopover();
    };
    document.addEventListener("pointerdown", onDown, true);
    return () => document.removeEventListener("pointerdown", onDown, true);
  });

  // Keep the highlighted option in view (the app list scrolls).
  $effect(() => {
    el?.querySelectorAll("li")[filters.popoverIndex]?.scrollIntoView({ block: "nearest" });
  });
</script>

{#if pill}
  <div bind:this={el} class="rc-pop" role="dialog" aria-label="Edit filter" style:margin-left="{left}px">
    <div class="rc-pop__op">{current}</div>
    <ul class="mx-menu" role="menu">
      {#each filters.popoverOptions as option, i (option.op + option.label)}
        {@const count = search.facetCounts?.[option.op]}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          role="menuitem"
          data-active={filters.popoverIndex === i || undefined}
          data-on={option.op === current || undefined}
          onmousemove={() => (filters.popoverIndex = i)}
          onclick={() => filters.applyOption(option)}
        >
          {#if option.pill === null}
            <IconX width="13" height="13" aria-hidden="true" />
          {:else if option.pill.kind === "app"}
            {@const icon = appIcons.src(option.pill.app)}
            {#if icon}<img class="rc-pop__app" src={icon} alt="" />{:else}<i class="rc-pop__app"
                >{option.pill.app.charAt(0).toUpperCase()}</i
              >{/if}
          {:else if option.pill.kind === "source"}
            {#if option.pill.source === "mic"}<IconMic width="13" height="13" aria-hidden="true" />
            {:else if option.pill.source === "system"}<IconSpeaker width="13" height="13" aria-hidden="true" />
            {:else}<IconMonitor width="13" height="13" aria-hidden="true" />{/if}
          {:else}
            <IconCalendar width="13" height="13" aria-hidden="true" />
          {/if}
          <span class="rc-pop__label">{option.label}</span>
          {#if count !== undefined}<span class="mx-menu__meta">{count}</span>{/if}
          <IconCheck class="rc-pop__on" width="13" height="13" aria-hidden="true" />
        </li>
      {/each}
    </ul>
    <div class="rc-pop__foot">
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => filters.editAsText()}
        >Edit as text</button
      >
      <span class="mx-spacer"></span>
      <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => filters.removePill()}
        >Remove<kbd>⌫</kbd></button
      >
    </div>
  </div>
{/if}

<style>
  .rc-pop {
    grid-area: 1 / 1;
    align-self: start;
    justify-self: start;
    position: relative;
    z-index: 2;
    min-width: 240px;
    max-width: 320px;
    padding: 4px 0 0;
    background: color-mix(in srgb, var(--app-surface-raised) 96%, transparent);
    border: 1px solid var(--app-overlay-border);
    border-radius: var(--r-lg);
    -webkit-backdrop-filter: blur(22px) saturate(1.4);
    backdrop-filter: blur(22px) saturate(1.4);
    box-shadow: var(--app-shadow-popover);
  }
  .rc-pop__op {
    padding: 8px 14px 4px;
    font: 400 var(--text-xs) / 1 var(--font-mono);
    color: var(--app-text-subtle);
  }
  .rc-pop .mx-menu {
    max-height: 216px;
    overflow-y: auto;
  }
  .rc-pop .mx-menu li {
    cursor: pointer;
  }
  .rc-pop .mx-menu li[data-active] {
    background: var(--mx-selected);
    color: var(--app-text-strong);
  }
  .rc-pop__label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rc-pop .mx-menu li :global(.rc-pop__on) {
    flex: none;
    margin-left: auto;
    color: var(--app-accent);
    visibility: hidden;
  }
  .rc-pop .mx-menu__meta + :global(.rc-pop__on) {
    margin-left: 6px;
  }
  .rc-pop .mx-menu li[data-on] :global(.rc-pop__on) {
    visibility: visible;
  }
  .rc-pop__app {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 4px;
    font: 700 8.5px / 1 var(--font-sans);
    font-style: normal;
    background: var(--mx-wash-strong);
    color: var(--app-text-strong);
    object-fit: contain;
  }
  img.rc-pop__app {
    background: none;
  }
  .rc-pop__foot {
    display: flex;
    gap: 4px;
    padding: 6px;
    border-top: 1px solid var(--mx-hairline);
  }
</style>
