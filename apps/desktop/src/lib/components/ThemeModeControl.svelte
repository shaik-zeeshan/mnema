<script lang="ts">
  import type { AppearanceSetting } from "$lib/types";
  import { message } from "@tauri-apps/plugin-dialog";
  import { humanizeError } from "$lib/format-error";
  import Segmented from "$lib/components/Segmented.svelte";
  import IconSystem from "~icons/lucide/monitor";
  import IconLight from "~icons/lucide/sun";
  import IconDark from "~icons/lucide/moon";

  let {
    value = $bindable<AppearanceSetting>("system"),
    disabled = false,
    compact = false,
    onChange,
  }: {
    value: AppearanceSetting;
    disabled?: boolean;
    compact?: boolean;
    onChange?: (value: AppearanceSetting) => void | Promise<void>;
  } = $props();

  let pending = $state(false);

  const options: { value: AppearanceSetting; label: string; shortLabel: string }[] = [
    { value: "system", label: "System theme", shortLabel: "System" },
    { value: "light", label: "Light theme", shortLabel: "Light" },
    { value: "dark", label: "Dark theme", shortLabel: "Dark" },
  ];

  // In the compact (titlebar) variant we show icons only; otherwise icon + label.
  // Either way the full theme name rides along as the accessible name, so the
  // icon-only compact pill still announces "System theme" / "Light theme" / etc.
  const segmentedOptions = $derived(
    options.map((o) => ({ value: o.value, label: compact ? "" : o.shortLabel, ariaLabel: o.label }))
  );

  async function select(next: string) {
    const nextValue = next as AppearanceSetting;
    if (disabled || pending || nextValue === value) return;

    if (!onChange) {
      value = nextValue;
      return;
    }

    const previous = value;
    pending = true;
    try {
      await onChange(nextValue);
      if (value === previous) {
        value = nextValue;
      }
    } catch (err) {
      if (value === nextValue) {
        value = previous;
      }
      // The control silently snapped back before; tell the user why so the
      // revert doesn't read as the click being ignored.
      const detail = humanizeError(err);
      await message(`The theme could not be changed.\n\n${detail}`, {
        title: "Couldn't change theme",
        kind: "error",
      });
    } finally {
      pending = false;
    }
  }
</script>

<div class="theme-mode" class:theme-mode--full={!compact}>
  <Segmented
    options={segmentedOptions}
    {value}
    onValueChange={select}
    disabled={disabled || pending}
    {compact}
    ariaLabel="Theme mode"
  >
    {#snippet icon(optionValue)}
      {#if optionValue === "system"}
        <IconSystem aria-hidden="true" />
      {:else if optionValue === "light"}
        <IconLight aria-hidden="true" />
      {:else}
        <IconDark aria-hidden="true" />
      {/if}
    {/snippet}
  </Segmented>
</div>

<style>
  /* In settings, the theme control spans the row; Segmented fills it. */
  .theme-mode--full {
    display: flex;
    width: 100%;
  }

  .theme-mode--full :global(.segmented) {
    width: 100%;
  }

  .theme-mode--full :global(.seg) {
    flex: 1 1 0;
    min-width: 0;
  }

  /* Compact variant sizes itself to its icons (titlebar). */
  .theme-mode {
    display: inline-flex;
  }
</style>
