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
  const isMenu = $derived($page.url.pathname.startsWith("/menu"));

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

  // Routes that want a centered, padded reading column.
  const isNarrow = $derived(isSettings || isDebug || isMenu);
</script>

<div class="app-shell">
  <main class="app-content" class:app-content--narrow={isNarrow}>
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

  /* ── Content ──────────────────────────────────────────────── */
  .app-content {
    flex: 1;
    width: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }

  /* The narrow column is opt-in — only routes that explicitly want a
     centered, padded reading column (currently `/settings`, `/debug`, and
     `/menu`) request it. Surfaces like the timeline consume the full
     viewport width by default so previews and dense controls aren't
     artificially capped. */
  .app-content--narrow {
    max-width: 640px;
    margin: 0 auto;
    padding: 28px 20px 64px;
    gap: 14px;
  }
</style>
