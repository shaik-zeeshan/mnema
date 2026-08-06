<script lang="ts">
  // The tab's own head (the census's leading `.ghd`): the tab name at title
  // weight, plus one meta line saying what the tab is about. The tab strip
  // above names the tab too — this is the reading column's own entry point, and
  // it is where a tab-wide quantity belongs when there is a real one.
  //
  // `meta` is nullable BY DESIGN: the Data head's figures come from measured
  // machine facts, and G8 says an unmeasurable denominator renders nothing at
  // all rather than a placeholder.

  interface Props {
    title: string;
    meta?: string | null;
  }

  let { title, meta = null }: Props = $props();
</script>

<div class="settings-panel__head">
  <h2 class="settings-panel__title">{title}</h2>
  {#if meta}
    <span class="settings-panel__meta">{meta}</span>
  {/if}
</div>

<style>
  .settings-panel__head {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 8px;
    padding: 0 4px 4px;
  }

  .settings-panel__title {
    margin: 0;
    font-family: var(--app-font-sans);
    font-size: var(--t-title);
    font-weight: 600;
    letter-spacing: -0.016em;
    color: var(--app-text-strong);
  }

  .settings-panel__meta {
    font-family: var(--app-font-sans);
    font-size: var(--t-meta);
    color: var(--app-text-muted);
    letter-spacing: 0.01em;
  }

  /* While ⌘F is filtering, all five panels are mounted and the pane is one
     ranked list of rows — five tab heads would be five headings over a list
     that no longer has tabs. Same rule the group headers follow. */
  :global(.settings-shell.is-finding) .settings-panel__head {
    display: none;
  }
</style>
