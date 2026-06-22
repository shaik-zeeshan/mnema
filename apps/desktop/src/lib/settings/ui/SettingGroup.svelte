<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    hint?: string;
    /** Optional anchor id for deeplink scroll-to. */
    id?: string;
    /** Optional snippet rendered at the right of the header (e.g. a reset button). */
    actions?: Snippet;
    /** The stack of <SettingRow>s. */
    children: Snippet;
  }

  let { title, hint, id, actions, children }: Props = $props();
</script>

<section class="setting-group" {id}>
  <header class="setting-group__header">
    <div class="setting-group__heading">
      <span class="setting-group__title">{title}</span>
      {#if hint}
        <span class="setting-group__hint">{hint}</span>
      {/if}
    </div>
    {#if actions}
      <div class="setting-group__actions">
        {@render actions()}
      </div>
    {/if}
  </header>
  <div class="setting-group__rows">
    {@render children()}
  </div>
</section>

<style>
  .setting-group {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .setting-group__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }

  .setting-group__heading {
    display: flex;
    flex-direction: column;
    gap: 3px;
    min-width: 0;
  }

  .setting-group__title {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.12em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }

  .setting-group__hint {
    font-size: 11px;
    color: var(--app-text-muted);
    letter-spacing: 0.01em;
    line-height: 1.5;
  }

  .setting-group__actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    flex-wrap: wrap;
    flex-shrink: 0;
  }

  /* Flat list of rows — not a heavy card. A single understated top hairline
     echoes the existing `.card::before` accent without boxing the rows in. */
  .setting-group__rows {
    position: relative;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--app-border);
  }

  .setting-group__rows::before {
    content: "";
    position: absolute;
    inset: 0 0 auto 0;
    height: 1px;
    background: linear-gradient(
      90deg,
      transparent,
      var(--app-accent-strong) 20%,
      var(--app-accent) 50%,
      var(--app-accent-strong) 80%,
      transparent
    );
    opacity: 0.14;
    pointer-events: none;
  }
</style>
