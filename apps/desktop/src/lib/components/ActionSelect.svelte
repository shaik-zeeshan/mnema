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
    /** Small (--h-sm) trigger that sits beside dense inline action buttons. */
    compact?: boolean;
    /**
     * Fired with the chosen option's value. May be async — the control holds
     * the chosen value until it settles, then resets to the placeholder, so the
     * same menu can dispatch the same command again. This is the action-trigger
     * behaviour of a native `<select>` that reset `select.value = ""` after
     * dispatching, expressed on the shared Select primitive.
     */
    onpick: (value: string) => void | Promise<void>;
  }

  let {
    options,
    placeholder = "Select…",
    disabled = false,
    ariaLabel,
    compact = false,
    onpick,
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
      console.error("ActionSelect onpick failed", error);
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
    {disabled}
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

  /* Compact = the kit's sm control height beside dense inline action buttons.
     Scoped to compact instances; the dropdown keeps its default sizing. */
  .action-select--compact :global(.select-trigger) {
    height: var(--h-sm);
    padding: 0 8px 0 10px;
    font-size: var(--text-base);
  }
</style>
