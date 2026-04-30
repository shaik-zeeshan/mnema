<script lang="ts">
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import type { Snippet } from "svelte";
  import { developerOptions, loadDeveloperOptions } from "$lib/developer-options.svelte";

  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  const isSettings = $derived($page.url.pathname.startsWith("/settings"));
  const isDebug = $derived($page.url.pathname.startsWith("/debug"));
  // The root route (`/`) is now the Timeline surface — the default landing
  // route. Debug lives at `/debug` and stays gated behind developer-options.
  const isTimeline = $derived($page.url.pathname === "/");

  const devEnabled = $derived(developerOptions.value);
  const devLoaded = $derived(developerOptions.loaded);

  $effect(() => {
    loadDeveloperOptions();
  });

  // Gate direct visits to `/debug` behind developer-options. We wait until
  // the flag has actually loaded to avoid a flash-redirect when the persisted
  // value is `true` but the IPC hasn't returned yet.
  $effect(() => {
    if (!devLoaded) return;
    if (isDebug && !devEnabled) {
      goto("/", { replaceState: true });
    }
  });

  // Hide the gated Debug surface until we know whether developer options
  // are enabled, and while we're redirecting a disabled user away from it.
  // Non-gated routes always render immediately.
  const showChildren = $derived(!isDebug || (devLoaded && devEnabled));
</script>

<div class="app-shell">
  <nav class="app-nav">
    <div class="nav-brand">
      <span class="nav-brand__dot" class:nav-brand__dot--recording={false}></span>
      <span class="nav-brand__name">capture · z</span>
    </div>
    <div class="nav-links">
      {#if devEnabled}
        <a href="/debug" class="nav-link" class:nav-link--active={isDebug}>
          <span class="nav-link__icon">◉</span>
          <span class="nav-link__label">Debug</span>
        </a>
      {/if}
      <a href="/" class="nav-link" class:nav-link--active={isTimeline}>
        <span class="nav-link__icon">▦</span>
        <span class="nav-link__label">Timeline</span>
      </a>
      <a href="/settings" class="nav-link" class:nav-link--active={isSettings}>
        <span class="nav-link__icon">⊙</span>
        <span class="nav-link__label">Settings</span>
      </a>
    </div>
  </nav>

  <main class="app-content" class:app-content--narrow={isSettings || isDebug}>
    {#if showChildren}
      {@render children()}
    {/if}
  </main>
</div>

<style>
  :global(*, *::before, *::after) {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  :global(html) {
    height: 100%;
  }

  :global(body) {
    min-height: 100%;
    background-color: #0c0c0e;
    color: #e2e2e8;
    font-family: "Berkeley Mono", "TX-02", "Monaspace Neon", ui-monospace,
      "Cascadia Code", "Fira Code", monospace;
    font-size: 13px;
    line-height: 1.6;
    -webkit-font-smoothing: antialiased;
  }

  :global(a) {
    text-decoration: none;
  }

  .app-shell {
    display: flex;
    flex-direction: column;
    min-height: 100vh;
  }

  /* ── Nav ───────────────────────────────────────────────────── */
  .app-nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 24px;
    height: 44px;
    background: #0e0e16;
    border-bottom: 1px solid #1a1a2a;
    position: sticky;
    top: 0;
    z-index: 10;
    flex-shrink: 0;
  }

  .nav-brand {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .nav-brand__dot {
    display: block;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #3dffa0;
    flex-shrink: 0;
    transition: background 0.3s;
  }

  .nav-brand__dot--recording {
    background: #ff4455;
    animation: pulse-dot 1.2s ease-in-out infinite;
  }

  @keyframes pulse-dot {
    0%, 100% { opacity: 1; box-shadow: 0 0 0 0 rgba(255, 68, 85, 0.4); }
    50% { opacity: 0.7; box-shadow: 0 0 0 4px rgba(255, 68, 85, 0); }
  }

  .nav-brand__name {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: #6a6a88;
  }

  .nav-links {
    display: flex;
    align-items: center;
    gap: 2px;
  }

  .nav-link {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    border-radius: 4px;
    color: #44445a;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    transition: color 0.12s, background 0.12s;
    border: 1px solid transparent;
  }

  .nav-link:hover {
    color: #8888aa;
    background: #131320;
  }

  .nav-link--active {
    color: #c0c0d8;
    background: #13131e;
    border-color: #1e1e2e;
  }

  .nav-link--active:hover {
    color: #e0e0f0;
  }

  .nav-link__icon {
    font-size: 9px;
    opacity: 0.7;
  }

  .nav-link--active .nav-link__icon {
    color: #3dffa0;
    opacity: 1;
  }

  .nav-link__label {
    font-size: 10px;
  }

  /* ── Content ──────────────────────────────────────────────── */
  .app-content {
    flex: 1;
    width: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* The narrow column is opt-in — only routes that explicitly want a
     centered, padded reading column (currently `/settings`) request it.
     Surfaces like the timeline and debug consume the full viewport width
     by default so previews and dense controls aren't artificially capped. */
  .app-content--narrow {
    max-width: 640px;
    margin: 0 auto;
    padding: 28px 20px 64px;
    gap: 14px;
  }
</style>
