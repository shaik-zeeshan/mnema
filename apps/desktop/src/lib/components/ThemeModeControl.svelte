<script lang="ts">
  import type { AppearanceSetting } from "$lib/types";

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

  async function select(next: AppearanceSetting) {
    if (disabled || pending || next === value) return;

    if (!onChange) {
      value = next;
      return;
    }

    const previous = value;
    pending = true;
    try {
      await onChange(next);
      if (value === previous) {
        value = next;
      }
    } catch (err) {
      if (value === next) {
        value = previous;
      }
      console.error("Failed to update theme mode", err);
    } finally {
      pending = false;
    }
  }
</script>

<div class="theme-mode" class:theme-mode--compact={compact} role="group" aria-label="Theme mode">
  {#each options as option}
    <button
      type="button"
      class="theme-mode__button"
      class:theme-mode__button--active={value === option.value}
      aria-label={option.label}
      aria-pressed={value === option.value}
      title={option.label}
      disabled={disabled || pending}
      onclick={() => select(option.value)}
    >
      <span class="theme-mode__icon" aria-hidden="true">
        {#if option.value === "system"}
          <svg viewBox="0 0 24 24">
            <rect x="4" y="5" width="16" height="11" rx="2" />
            <path d="M9 19h6" />
            <path d="M12 16v3" />
          </svg>
        {:else if option.value === "light"}
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
      </span>
      {#if !compact}
        <span class="theme-mode__label">{option.shortLabel}</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .theme-mode {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    width: 100%;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
  }

  .theme-mode__button {
    min-width: 0;
    flex: 1 1 0;
    height: 34px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    border: 1px solid transparent;
    border-radius: 3px;
    color: var(--app-text-muted);
    background: transparent;
    font: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.04em;
    cursor: pointer;
    padding: 0 10px;
    transition: background 0.12s, border-color 0.12s, color 0.12s;
  }

  .theme-mode__button:hover:not(:disabled) {
    background: var(--app-surface-hover);
    border-color: var(--app-border-hover);
    color: var(--app-text);
  }

  .theme-mode__button:focus-visible {
    outline: 1px solid var(--app-accent);
    outline-offset: 1px;
  }

  .theme-mode__button--active {
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  .theme-mode__button--active:hover:not(:disabled) {
    color: var(--app-accent);
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
  }

  .theme-mode__button--active .theme-mode__icon,
  .theme-mode__button--active .theme-mode__label {
    color: inherit;
  }

  .theme-mode__button:disabled {
    opacity: 0.42;
    cursor: default;
  }

  .theme-mode__icon,
  .theme-mode__icon svg {
    width: 15px;
    height: 15px;
    display: block;
    flex: 0 0 auto;
  }

  .theme-mode__icon svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  .theme-mode__label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .theme-mode--compact {
    width: auto;
    height: 30px;
    padding: 2px;
    gap: 2px;
    border-radius: 999px;
    border-color: var(--app-icon-border-hover);
    background: var(--app-surface-raised);
  }

  .theme-mode--compact .theme-mode__button {
    flex: 0 0 auto;
    width: 28px;
    min-width: 28px;
    height: 24px;
    padding: 0;
    border-radius: 999px;
    color: var(--app-icon-fg);
  }

  .theme-mode--compact .theme-mode__button:hover:not(:disabled) {
    background: var(--app-icon-bg-hover);
    border-color: var(--app-icon-border-hover);
    color: var(--app-icon-fg-hover);
  }

  .theme-mode--compact .theme-mode__button--active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent);
  }

  .theme-mode--compact .theme-mode__button--active:hover:not(:disabled) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent);
    color: var(--app-accent);
  }

  .theme-mode--compact .theme-mode__icon,
  .theme-mode--compact .theme-mode__icon svg {
    width: 16px;
    height: 16px;
  }

  :global([data-theme="light"]) .theme-mode__button--active,
  :global([data-theme="light"]) .theme-mode__button--active:hover:not(:disabled) {
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }

  :global([data-theme="light"]) .theme-mode__button--active:hover:not(:disabled) {
    border-color: var(--app-accent);
  }

  :global([data-theme="light"]) .theme-mode--compact .theme-mode__button--active,
  :global([data-theme="light"]) .theme-mode--compact .theme-mode__button--active:hover:not(:disabled) {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }

  :global([data-theme="light"]) .theme-mode--compact .theme-mode__button--active:hover:not(:disabled) {
    border-color: var(--app-accent);
  }
</style>
