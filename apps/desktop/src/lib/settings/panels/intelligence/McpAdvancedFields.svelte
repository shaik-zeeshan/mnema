<script lang="ts">
  // McpAdvancedFields — the collapsible "Advanced" overrides on the preset
  // step-2 connect body (name + URL for hosted, name + command + args for
  // local), split out of McpConnectorPicker (800-line cap split). Purely the
  // disclosure + inputs; the parent owns the bound values (they feed
  // presetOverrides) and the `onInput` write-through (edit mode rides autosave).
  interface Props {
    /** Disclosure open/closed (two-way — the parent seeds it closed on select). */
    open: boolean;
    /** Preset kind: hosted shows a URL field, local shows command + args. */
    kind: "hosted" | "local";
    name: string;
    url: string;
    command: string;
    args: string;
    /** Fires on every edit so edit mode writes through to the live draft. */
    onInput: () => void;
  }

  let {
    open = $bindable(),
    kind,
    name = $bindable(),
    url = $bindable(),
    command = $bindable(),
    args = $bindable(),
    onInput,
  }: Props = $props();
</script>

<div class="adv">
  <button type="button" class="adv__toggle" aria-expanded={open} onclick={() => (open = !open)}>
    <span>Advanced</span>
    <span class="adv__chev" class:adv__chev--open={open} aria-hidden="true">›</span>
  </button>
  {#if open}
    <div class="adv__body">
      <div class="field">
        <label class="field-label" for="mcp-picker-adv-name">Name</label>
        <input id="mcp-picker-adv-name" class="text-input" autocomplete="off" bind:value={name} oninput={onInput} />
      </div>
      {#if kind === "hosted"}
        <div class="field">
          <label class="field-label" for="mcp-picker-adv-url">URL</label>
          <input id="mcp-picker-adv-url" class="text-input" autocomplete="off" bind:value={url} oninput={onInput} />
        </div>
      {:else}
        <div class="field">
          <label class="field-label" for="mcp-picker-adv-command">Command</label>
          <input id="mcp-picker-adv-command" class="text-input" autocomplete="off" bind:value={command} oninput={onInput} />
        </div>
        <div class="field">
          <label class="field-label" for="mcp-picker-adv-args">Arguments</label>
          <input id="mcp-picker-adv-args" class="text-input" autocomplete="off" placeholder="space-separated" bind:value={args} oninput={onInput} />
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .adv {
    border: 1px solid var(--app-border);
    border-radius: 10px;
    background: var(--app-surface);
    overflow: hidden;
  }
  .adv__toggle {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 10px 12px;
    background: transparent;
    border: 0;
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.1em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    cursor: pointer;
  }
  .adv__toggle:hover {
    color: var(--app-text-strong);
  }
  .adv__chev {
    font-size: 11px;
    transition: transform 0.15s;
  }
  .adv__chev--open {
    transform: rotate(90deg);
  }
  .adv__body {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 2px 12px 14px;
    border-top: 1px solid var(--app-border);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field .text-input {
    width: 100%;
  }

  @media (prefers-reduced-motion: reduce) {
    .adv__chev {
      transition: none;
    }
  }
</style>
