<script lang="ts">
  // Windows caption controls (minimize / maximize-restore / close) for the
  // frameless main window. macOS keeps its native overlay traffic lights, so
  // this renders only on Windows, where the app draws its own title bar and the
  // OS provides no window buttons (see `windows.rs` — overlay windows are made
  // `decorations: false` on Windows).
  import { getCurrentWindow } from "@tauri-apps/api/window";

  const appWindow = getCurrentWindow();

  let maximized = $state(false);

  // Track the maximized state so the middle button can flip between the
  // "maximize" and "restore" glyphs. Seed once, then follow resize events
  // (maximize/restore/snap all resize the window).
  $effect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;

    void appWindow
      .isMaximized()
      .then((value) => {
        if (!cancelled) maximized = value;
      })
      .catch(() => {});

    void appWindow
      .onResized(() => {
        void appWindow
          .isMaximized()
          .then((value) => {
            maximized = value;
          })
          .catch(() => {});
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      unlisten?.();
    };
  });

  function minimize(): void {
    void appWindow.minimize().catch(() => {});
  }

  function toggleMaximize(): void {
    void appWindow.toggleMaximize().catch(() => {});
  }

  function close(): void {
    void appWindow.close().catch(() => {});
  }
</script>

<div class="caption" aria-label="Window controls">
  <button
    type="button"
    class="caption__button"
    aria-label="Minimize"
    title="Minimize"
    onclick={minimize}
  >
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path d="M0 5h10" stroke="currentColor" stroke-width="1" fill="none" />
    </svg>
  </button>
  <button
    type="button"
    class="caption__button"
    aria-label={maximized ? "Restore" : "Maximize"}
    title={maximized ? "Restore" : "Maximize"}
    onclick={toggleMaximize}
  >
    {#if maximized}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <path
          d="M2.5 2.5V0.5h7v7h-2M0.5 2.5h7v7h-7z"
          stroke="currentColor"
          stroke-width="1"
          fill="none"
        />
      </svg>
    {:else}
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
        <rect x="0.5" y="0.5" width="9" height="9" stroke="currentColor" stroke-width="1" fill="none" />
      </svg>
    {/if}
  </button>
  <button
    type="button"
    class="caption__button caption__button--close"
    aria-label="Close"
    title="Close"
    onclick={close}
  >
    <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
      <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" stroke-width="1" fill="none" />
    </svg>
  </button>
</div>

<style>
  /* Flush to the top-right corner and full title-bar height, mirroring the
     native Windows caption strip. The controls sit outside the padded right
     group so they hug the window edge. */
  .caption {
    display: inline-flex;
    align-items: stretch;
    height: 100%;
    flex: 0 0 auto;
  }

  .caption__button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 46px;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--app-text-muted);
    cursor: pointer;
    transition: background 0.12s, color 0.12s;
  }

  .caption__button:hover {
    background: var(--app-icon-bg-hover);
    color: var(--app-text-strong);
  }

  .caption__button:active {
    filter: brightness(0.92);
  }

  .caption__button:focus-visible {
    outline: none;
    box-shadow: inset 0 0 0 2px var(--app-accent);
  }

  /* Close follows the Windows convention: red fill with a white glyph on hover. */
  .caption__button--close:hover {
    background: #c42b1c;
    color: #ffffff;
  }

  .caption__button--close:active {
    background: #b0271a;
  }
</style>
