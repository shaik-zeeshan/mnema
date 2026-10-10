<script lang="ts">
  // Main-window titlebar (SHELL.md › Titlebar): Timeline/Insights icon pill ·
  // the one recall/ask field · Chats · bell · gear. Recording moved to the
  // status bar; there is no theme toggle and no help icon here.
  import { page } from "$app/stores";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { tip } from "$lib/components/tooltip";
  import { normalizeAppPathname } from "$lib/route-path";
  import { getLastMainSurface, openDebugWindow, openSettings } from "$lib/surface-windows";
  import { getEffectiveGlobalShortcut } from "$lib/global-shortcuts";
  import { setKeyboardHelpGroups } from "$lib/keyboard-help.svelte";
  import { formatShortcut, matchShortcut, type KeyboardPlatform, type ShortcutDefinition } from "$lib/keyboard";
  import NotificationsPop from "./NotificationsPop.svelte";

  interface Props {
    platform: KeyboardPlatform;
    devEnabled: boolean;
  }
  let { platform, devEnabled }: Props = $props();

  const path = $derived(normalizeAppPathname($page.url.pathname));
  const onTimeline = $derived(path === "/");
  const onInsights = $derived(path.startsWith("/insights"));
  const onChat = $derived(path.startsWith("/chat"));
  const onSettings = $derived(path.startsWith("/settings"));

  // Shell-only chords: the field and the surfaces. Never ⌥Space / ⌥⌘Space —
  // ⌥⌘Space is the global Quick Recall window (keyboard_bindings.rs).
  const shell = (id: string, label: string, key: string): ShortcutDefinition => ({
    id,
    label,
    bindings: [{ key, primary: true }],
    kind: "command",
    scope: "global",
  });
  const KEYS = {
    recall: shell("shellRecall", "Recall anything", "K"),
    ask: shell("shellAsk", "Ask", "J"),
    timeline: shell("shellTimeline", "Timeline", "1"),
    insights: shell("shellInsights", "Insights", "2"),
  };
  $effect(() =>
    setKeyboardHelpGroups("shell", [{ id: "shell", title: "Here", rows: Object.values(KEYS) }]),
  );
  const keyText = (def: ShortcutDefinition) => formatShortcut(def.bindings[0], platform).join("");

  let mode = $state<"recall" | "ask">("recall");
  let query = $state("");
  let inputEl = $state<HTMLInputElement | null>(null);

  function focusField(next: "recall" | "ask"): void {
    mode = next;
    inputEl?.focus();
  }

  function goSurface(target: "/" | "/insights"): void {
    if (path !== target) void goto(target);
  }

  function askInChat(): void {
    const text = query.trim();
    void goto(text ? `/chat?q=${encodeURIComponent(text)}` : "/chat");
  }

  // ↵ in recall is a launcher (decision 2): the Quick Recall window opens with
  // the text; nothing filters in this window.
  async function recallInQuickRecall(): Promise<void> {
    const text = query.trim();
    if (!text) return;
    try {
      await invoke("summon_quick_recall_window_command", { query: text });
      query = "";
      inputEl?.blur();
    } catch {
      // Best-effort: ⌥⌘Space still summons the window; the text stays here.
    }
  }

  function onFieldKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Tab" && !event.shiftKey && !event.metaKey && !event.ctrlKey && !event.altKey) {
      event.preventDefault();
      mode = mode === "ask" ? "recall" : "ask";
    } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey || mode === "ask")) {
      event.preventDefault();
      askInChat();
    } else if (event.key === "Enter") {
      event.preventDefault();
      void recallInQuickRecall();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      mode = "recall";
      inputEl?.blur();
    }
  }

  function onWindowKeydown(event: KeyboardEvent): void {
    if (event.defaultPrevented || event.repeat) return;
    const hit = (def: ShortcutDefinition) => matchShortcut(event, def, platform);
    if (hit(KEYS.recall)) focusField("recall");
    else if (hit(KEYS.ask)) focusField("ask");
    else if (hit(KEYS.timeline)) goSurface("/");
    else if (hit(KEYS.insights)) goSurface("/insights");
    else return;
    event.preventDefault();
  }

  // The gear toggles: from Settings it returns to the surface Settings was opened from.
  function onGear(): void {
    if (!onSettings) {
      void openSettings();
      return;
    }
    goSurface(normalizeAppPathname(getLastMainSurface()).startsWith("/insights") ? "/insights" : "/");
  }
</script>

<svelte:window onkeydown={onWindowKeydown} />

<header class="mx-titlebar" data-tauri-drag-region>
  <div class="mx-titlebar__lead" data-tauri-drag-region>
    <nav class="mx-nav" aria-label="Main surface">
      <span class="mx-nav__ink" style:transform={onInsights ? "translateX(32px)" : undefined}></span>
      <a href="/" aria-label="Timeline" aria-current={onTimeline ? "page" : undefined} use:tip={`Timeline  ${keyText(KEYS.timeline)}`}>
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="M3 12h18" /><path d="M7 8v8" /><path d="M12 5v14" /><path d="M17 9v6" /></svg>
      </a>
      <a href="/insights" aria-label="Insights" aria-current={onInsights ? "page" : undefined} use:tip={`Insights  ${keyText(KEYS.insights)}`}>
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><path d="M4 20V10" /><path d="M10 20V4" /><path d="M16 20v-7" /><path d="M22 20H2" /></svg>
      </a>
    </nav>
  </div>

  <label class="mx-cmd" data-mode={mode}>
    <button
      type="button"
      class="mx-cmd__mode"
      tabindex="-1"
      aria-label="Switch between recall and ask (Tab)"
      onclick={(e) => {
        e.preventDefault();
        focusField(mode === "ask" ? "recall" : "ask");
      }}
    >
      {#if mode === "ask"}
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linejoin="round" aria-hidden="true"><path d="M12 3l1.8 5.2L19 10l-5.2 1.8L12 17l-1.8-5.2L5 10l5.2-1.8z" /></svg><b>Ask</b>
      {:else}
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true"><circle cx="11" cy="11" r="7" /><path d="m20 20-3.5-3.5" /></svg>
      {/if}
    </button>
    <input
      bind:this={inputEl}
      bind:value={query}
      placeholder={mode === "ask" ? "Ask about anything you’ve seen or heard…" : "Recall anything…"}
      aria-label={mode === "ask" ? "Ask — Tab to recall instead" : "Recall anything — Tab to ask instead"}
      autocomplete="off"
      spellcheck="false"
      onkeydown={onFieldKeydown}
    />
    <span class="mx-cmd__keys" aria-hidden="true">
      <kbd class="mx-cmd__k-idle">{keyText(KEYS.recall)}</kbd>
      <span class="mx-cmd__k-recall"><kbd>⇥</kbd>to ask</span>
      <span class="mx-cmd__k-ask"><kbd>↵</kbd>ask in chat</span>
    </span>
  </label>

  <div class="mx-titlebar__trail" data-tauri-drag-region>
    <!-- ponytail: /chat lands with CH1; until then this 404s (DC1 seam). -->
    <a class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" href="/chat" aria-label="Chats" aria-current={onChat ? "page" : undefined} use:tip={"Chats"}>
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12a8 8 0 0 1-11.6 7.1L4 20.5l1.4-4.9A8 8 0 1 1 21 12z" /></svg>
    </a>
    <NotificationsPop />
    <button
      type="button"
      class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
      aria-label={onSettings ? "Close settings" : "Settings"}
      aria-current={onSettings ? "page" : undefined}
      use:tip={onSettings ? "Close settings" : `Settings  ${formatShortcut(getEffectiveGlobalShortcut("openSettings").bindings[0], platform).join("")}`}
      onclick={onGear}
    >
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" /><circle cx="12" cy="12" r="3" /></svg>
    </button>
    {#if devEnabled}
      <button
        type="button"
        class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm"
        aria-label="Open debug"
        use:tip={`Debug  ${formatShortcut(getEffectiveGlobalShortcut("openDebug").bindings[0], platform).join("")}`}
        onclick={() => void openDebugWindow()}
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 3h6" /><path d="M10 9V7a2 2 0 1 1 4 0v2" /><rect x="5" y="9" width="14" height="10" rx="2" /><path d="M8 13h.01" /><path d="M16 13h.01" /><path d="M9 19v2" /><path d="M15 19v2" /><path d="M2 12h3" /><path d="M19 12h3" /></svg>
      </button>
    {/if}
  </div>
</header>
