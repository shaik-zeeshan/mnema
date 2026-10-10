<script lang="ts">
  // New-chat view (chat.html › default): a time-of-day greeting, the privacy
  // line, the composer (passed in), and "Try asking" suggestions that fill the
  // composer — never auto-send.
  import type { Component, Snippet } from "svelte";
  import { greeting } from "./chat-format";
  import IconMonitor from "~icons/lucide/monitor";
  import IconMic from "~icons/lucide/mic";
  import IconSearch from "~icons/lucide/search";
  import IconSpeaker from "~icons/lucide/volume-2";
  import IconNote from "~icons/lucide/user";

  interface Props {
    /** Show the suggestions (only when there's a composer to fill). */
    suggest: boolean;
    onpick: (text: string) => void;
    children: Snippet;
  }
  let { suggest, onpick, children }: Props = $props();

  const hello = greeting(new Date().getHours());
  const SUGGESTIONS: [string, Component, string][] = [
    ["What did I work on yesterday?", IconMonitor, "var(--app-source-screen)"],
    ["Summarize my calls this week", IconMic, "var(--app-source-mic)"],
    ["Find the PR link I saw Tuesday", IconSearch, "var(--app-source-screen)"],
    ["What was I reading before lunch?", IconMonitor, "var(--app-source-screen)"],
    ["Who did I talk to most this week?", IconSpeaker, "var(--app-source-sysaudio)"],
    ["Draft my standup from today", IconNote, "var(--app-info)"],
  ];
</script>

<div class="ch-new">
  <div class="ch-hello mx-reveal" style="--i:0">
    <span class="mx-kicker">New chat</span>
    <h2 class="mx-display">{hello} <span>What do you want to remember?</span></h2>
    <p class="mx-prose">Answers come from what Mnema captured on this Mac. The model only reads inside the scope below, and you choose where it runs.</p>
  </div>
  <div class="ch-slot mx-reveal" style="--i:1">{@render children()}</div>
  {#if suggest}
    <div class="ch-try mx-reveal" style="--i:2">
      <span class="mx-label">Try asking</span>
      <div class="ch-sugs">
        {#each SUGGESTIONS as [text, Icon, tint] (text)}
          <button type="button" class="mx-btn mx-btn--sm ch-sug" style="--c:{tint}" onclick={() => onpick(text)}>
            <Icon width="13" height="13" aria-hidden="true" />{text}
          </button>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .ch-new { max-width: 760px; margin: 0 auto; padding: clamp(32px, 9vh, 96px) var(--s-6) var(--s-6); }
  .ch-hello { margin-bottom: var(--s-5); }
  .ch-hello .mx-display { margin: var(--s-3) 0; }
  .ch-hello .mx-display span { color: var(--app-text-muted); font-weight: 500; }
  .ch-slot { position: relative; z-index: 5; }
  .ch-try { margin-top: var(--s-5); }
  .ch-try .mx-label { display: block; margin-bottom: var(--s-2); }
  .ch-sugs { display: flex; flex-wrap: wrap; gap: var(--s-2); }
  .ch-sug :global(svg) { color: var(--c); }
</style>
