<!--
  The small-window strip (access.html `.ac-strip`, update.html `.up-strip`):
  three drawn lights, the window's name centred, and the whole strip is the
  drag region. These windows are undecorated, so the first light IS the close
  button (the onboarding strip does the same); the other two are drawn.
-->
<script lang="ts">
  import { closeCurrentWindow } from "$lib/surface-windows";

  let { title, onclose }: { title: string; onclose?: () => void } = $props();
</script>

<header class="mx-strip" data-tauri-drag-region>
  <div class="mx-strip__lights" data-tauri-drag-region>
    <button type="button" aria-label="Close window" onclick={() => (onclose ? onclose() : void closeCurrentWindow())}></button>
    <i aria-hidden="true"></i><i aria-hidden="true"></i>
  </div>
  <span class="mx-strip__title">{title}</span>
</header>

<style>
  .mx-strip {
    position: relative;
    flex: none;
    display: flex;
    align-items: center;
    height: var(--mx-titlebar-h);
    border-bottom: 1px solid var(--mx-hairline);
    user-select: none;
    -webkit-user-select: none;
  }
  .mx-strip__lights {
    position: relative;
    z-index: 1;
    display: flex;
    gap: 8px;
    width: 78px;
    padding-left: 13px;
  }
  .mx-strip__lights > * {
    width: 12px;
    height: 12px;
    padding: 0;
    border: 0;
    border-radius: 50%;
    background: var(--app-border-strong);
  }
  .mx-strip:hover .mx-strip__lights > :nth-child(1),
  .mx-strip__lights button:focus-visible {
    background: var(--app-status-running-dot);
  }
  .mx-strip:hover .mx-strip__lights > :nth-child(2) { background: var(--app-status-paused-dot); }
  .mx-strip:hover .mx-strip__lights > :nth-child(3) { background: var(--app-source-mic); }
  .mx-strip__lights button:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .mx-strip__title {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    pointer-events: none;
    font: 500 var(--text-base) / 1 var(--font-sans);
    color: var(--app-text-subtle);
  }
</style>
