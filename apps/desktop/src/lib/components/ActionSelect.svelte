<script lang="ts">
  import Select from "./Select.svelte";

  interface Option {
    value: string;
    label: string;
  }

  interface Props {
    /** Choosable actions. Picking one fires `onpick`, then the control resets. */
    options: Option[];
    placeholder?: string;
    disabled?: boolean;
    /** Accessible name for the trigger (this control has no visible label). */
    ariaLabel?: string;
    /** Pill-sized trigger that sits beside dense inline action buttons. */
    compact?: boolean;
    /**
     * Fired with the chosen option's value. May be async — the control holds
     * the chosen value until it settles, then resets to the placeholder, so the
     * same menu can dispatch the same command again. This is the action-trigger
     * behaviour of a native `<select>` that reset `select.value = ""` after
     * dispatching, expressed on the shared Select primitive.
     */
    onpick: (value: string) => void | Promise<void>;
    /**
     * Notified when an async `onpick` rejects. Without it the control would
     * silently reset to the placeholder on failure, leaving no trace the action
     * failed; pass a handler to surface it (toast, debug strip, etc.).
     */
    onError?: (error: unknown) => void;
  }

  let {
    options,
    placeholder = "Select…",
    disabled = false,
    ariaLabel,
    compact = false,
    onpick,
    onError,
  }: Props = $props();

  // Never holds a persistent selection — it returns to the placeholder once the
  // dispatched action settles.
  let value = $state<string | null>(null);
  // True while an async `onpick` is in flight: locks the trigger so the same
  // action can't be double-dispatched and gives the control its own busy state.
  let busy = $state(false);

  async function handleChange(next: string): Promise<void> {
    if (!next) return;
    value = next;
    busy = true;
    try {
      await onpick(next);
    } catch (error) {
      // Surface the rejection instead of swallowing it on the silent reset.
      if (onError) onError(error);
      else console.error("ActionSelect onpick failed", error);
    } finally {
      busy = false;
      value = null;
    }
  }
</script>

<div class="action-select" class:action-select--compact={compact}>
  <Select
    bind:value
    {options}
    {placeholder}
    disabled={disabled || busy}
    loading={busy}
    {ariaLabel}
    onValueChange={handleChange}
  />
</div>

<style>
  .action-select {
    width: 100%;
  }

  .action-select--compact {
    width: 11rem;
    max-width: 100%;
  }

  /* Shrink the shared Select trigger to a pill that matches the popover's
     compact action buttons. Scoped to compact instances only; the dropdown
     panel and its items keep their default readable sizing. */
  .action-select--compact :global(.select-trigger) {
    min-height: 24px;
    padding: 3px 8px;
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }
</style>
