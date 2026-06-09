<script lang="ts">
  // Chat — placeholder only. Chat belongs to a separate issue (#90) and is NOT
  // built here; this stub just reserves the tab in the Insights workspace.
</script>

<section class="surface" aria-label="Chat">
  <header class="surface-head">
    <h1>Chat</h1>
    <span class="caption">Ask the engine about your activity</span>
  </header>
  <div class="placeholder">
    <p class="muted">Chat is coming soon.</p>
  </div>
</section>

<style>
  .surface {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .surface-head {
    display: flex;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  .surface-head h1 {
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .caption {
    font-size: 10.5px;
    color: var(--app-text-faint);
    letter-spacing: 0.02em;
  }
  .placeholder {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 14px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .muted {
    font-size: 13px;
    color: var(--app-text-muted);
  }
</style>
