<script lang="ts">
  import type { AppearanceSetting } from "$lib/types";
  import Segmented from "$lib/components/Segmented.svelte";

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
      console.error("Failed to update theme mode", err);
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
        <svg viewBox="0 0 24 24">
          <rect x="4" y="5" width="16" height="11" rx="2" />
          <path d="M9 19h6" />
          <path d="M12 16v3" />
        </svg>
      {:else if optionValue === "light"}
        <svg viewBox="0 0 24 24">
          <circle cx="12" cy="12" r="4" />
          <path d="M12 3v2" />
          <path d="M12 19v2" />
          <path d="M3 12h2" />
          <path d="M19 12h2" />
          <path d="m5.6 5.6 1.4 1.4" />
          <path d="m17 17 1.4 1.4" />
          <path d="m18.4 5.6-1.4 1.4" />
          <path d="m7 17-1.4 1.4" />
        </svg>
      {:else}
        <svg viewBox="0 0 24 24">
          <path d="M20 14.5A7.5 7.5 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5Z" />
        </svg>
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
