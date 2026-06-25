<script lang="ts">
  // Collapses a model's missing-file list behind a compact, clickable summary.
  // The raw filenames are diagnostic noise for most users (a missing model just
  // needs a Download), so the default view shows only a count; the full list is
  // available on demand for diagnosing a partial or corrupt install.
  let { files }: { files: string[] } = $props();
</script>

{#if files.length > 0}
  <details class="missing-files">
    <summary class="group-hint group-hint--warn">
      <span class="missing-files__caret" aria-hidden="true"></span>
      {files.length}
      {files.length === 1 ? "file" : "files"} missing — download to complete this model
    </summary>
    <ul class="missing-files__list">
      {#each files as file}
        <li>{file}</li>
      {/each}
    </ul>
  </details>
{/if}

<style>
  .missing-files {
    width: 100%;
  }

  .missing-files summary {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    list-style: none;
    user-select: none;
  }

  .missing-files summary::-webkit-details-marker {
    display: none;
  }

  .missing-files__caret {
    width: 0;
    height: 0;
    border-top: 4px solid transparent;
    border-bottom: 4px solid transparent;
    border-left: 5px solid currentColor;
    transition: transform 120ms ease;
  }

  .missing-files[open] .missing-files__caret {
    transform: rotate(90deg);
  }

  .missing-files__list {
    margin: 6px 0 0;
    padding: 8px 10px;
    list-style: none;
    max-height: 160px;
    overflow-y: auto;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10px;
    line-height: 1.6;
    color: var(--app-text-faint);
  }

  .missing-files__list li {
    overflow-wrap: anywhere;
  }
</style>
