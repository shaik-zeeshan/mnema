<script lang="ts">
  import { onMount } from "svelte";

  let query = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  onMount(() => {
    inputEl?.focus();
  });
</script>

<div class="quick-recall">
  <div class="quick-recall__field">
    <span class="quick-recall__glyph" aria-hidden="true">⌕</span>
    <input
      bind:this={inputEl}
      bind:value={query}
      class="quick-recall__input"
      type="text"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      placeholder="Search your captures…"
      aria-label="Search your captures"
    />
    <kbd class="quick-recall__hint">esc</kbd>
  </div>
</div>

<style>
  .quick-recall {
    height: 100vh;
    height: 100dvh;
    width: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 12px;
    overflow: hidden;
    color: var(--app-text);
    font-family: inherit;
  }

  .quick-recall__field {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 18px;
    flex: 1;
  }

  .quick-recall__glyph {
    font-size: 20px;
    line-height: 1;
    color: var(--app-text-muted);
    flex-shrink: 0;
    transform: rotate(-45deg);
  }

  .quick-recall__input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--app-text-strong);
    font-family: inherit;
    font-size: 18px;
    line-height: 1.4;
    padding: 0;
    caret-color: var(--app-accent);
  }

  .quick-recall__input::placeholder {
    color: var(--app-text-subtle);
  }

  .quick-recall__hint {
    flex-shrink: 0;
    font-family: inherit;
    font-size: 11px;
    line-height: 1;
    text-transform: lowercase;
    color: var(--app-text-muted);
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    border-radius: 6px;
    padding: 4px 7px;
  }
</style>
