<script lang="ts">
  // The one Chat composer (chat.html › composer): large under the new-chat
  // greeting, docked under a thread. Enter sends, Shift+Enter newlines; the send
  // button morphs to Stop while a turn streams. When Ask AI can't run, a kit
  // notice with the one helpful action replaces it.
  import { tick } from "svelte";
  import type { AiRuntimeSettings } from "$lib/types/recording";
  import ModelPicker from "./ModelPicker.svelte";
  import IconSend from "~icons/lucide/arrow-up";
  import IconStop from "~icons/lucide/square";
  import IconOff from "~icons/lucide/circle-off";
  import IconAlert from "~icons/lucide/triangle-alert";

  interface Props {
    value: string;
    docked: boolean;
    streaming: boolean;
    /** null while availability loads; else whether Ask AI can run. */
    available: boolean | null;
    /** What to say (and the one action) when it can't. */
    off: { text: string; action: string; warn: boolean };
    onoffaction: () => void;
    /** The provider the question goes to ("Send to Anthropic"). */
    sendTo: string | null;
    aiRuntime: AiRuntimeSettings | null;
    askAiModelOverride: string | null;
    pinProvider: string | null;
    pinModel: string | null;
    pickerOpen: boolean;
    onselect: (engine: { provider: string; model: string } | null) => void;
    onsend: () => void;
    onstop: () => void;
  }
  let {
    value = $bindable(),
    docked,
    streaming,
    available,
    off,
    onoffaction,
    sendTo,
    aiRuntime,
    askAiModelOverride,
    pinProvider,
    pinModel,
    pickerOpen = $bindable(),
    onselect,
    onsend,
    onstop,
  }: Props = $props();

  let el = $state<HTMLTextAreaElement | null>(null);

  /** Focus the box with the caret at the end (after a prefill). */
  export function focus(): void {
    void tick().then(() => {
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
  }

  // Grow with the text up to a cap.
  $effect(() => {
    value;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(220, el.scrollHeight)}px`;
  });

  function onkeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      onsend();
    }
  }
</script>

{#if available === false}
  <div class="ch-composer ch-composer--off" class:ch-composer--docked={docked}>
    <div class="mx-notice" data-tone={off.warn ? "warn" : "off"} role="status">
      <span class="mx-notice__icon">
        {#if off.warn}<IconAlert width="15" height="15" aria-hidden="true" />{:else}<IconOff width="15" height="15" aria-hidden="true" />{/if}
      </span>
      <p class="mx-notice__text">{off.text}</p>
      <div class="mx-notice__acts">
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={onoffaction}>{off.action}</button>
      </div>
    </div>
  </div>
{:else}
  <form
    class="ch-composer"
    class:ch-composer--docked={docked}
    autocomplete="off"
    onsubmit={(e) => {
      e.preventDefault();
      if (streaming) onstop();
      else onsend();
    }}
  >
    <textarea
      bind:this={el}
      bind:value
      rows="2"
      placeholder={docked ? "Ask a follow-up…" : "Ask about anything you've seen, heard, or worked on…"}
      aria-label="Message"
      disabled={streaming}
      {onkeydown}
    ></textarea>
    <div class="ch-bar">
      <ModelPicker
        {aiRuntime}
        {askAiModelOverride}
        {pinProvider}
        {pinModel}
        up={docked}
        bind:open={pickerOpen}
        {onselect}
      />
      <span class="mx-spacer"></span>
      <button
        type="submit"
        class="mx-btn ch-send"
        class:mx-btn--primary={!streaming}
        class:ch-send--stop={streaming}
        class:mx-btn--icon={docked || streaming}
        aria-label={streaming ? "Stop" : "Send"}
        disabled={available === null}
      >
        {#if streaming}
          <IconStop width="12" height="12" aria-hidden="true" />
        {:else}
          <IconSend width="14" height="14" aria-hidden="true" />
          {#if !docked}<span>{sendTo ? `Send to ${sendTo}` : "Send"}</span><kbd>↵</kbd>{/if}
        {/if}
      </button>
    </div>
  </form>
{/if}

<style>
  .ch-composer {
    position: relative;
    max-width: 760px;
    margin: 0 auto;
    border-radius: var(--r-xl);
    background: var(--app-surface-raised);
    border: 1px solid var(--app-border-strong);
    transition: border-color var(--t-fast) var(--ease-quart), box-shadow var(--t-med) var(--ease-quart);
  }
  .ch-composer:focus-within { border-color: var(--app-border-hover); box-shadow: 0 0 0 4px color-mix(in srgb, var(--app-fg) 3%, transparent); }
  .ch-composer--docked { max-width: 696px; }
  .ch-composer--off { border: 0; background: none; }
  .ch-composer--off:focus-within { box-shadow: none; }
  textarea {
    display: block;
    width: 100%;
    resize: none;
    min-height: 84px;
    max-height: 220px;
    padding: var(--s-4) var(--s-4) var(--s-2);
    border: 0;
    outline: 0;
    background: none;
    color: var(--app-text-strong);
    font: 400 var(--text-lg)/1.5 var(--font-sans);
  }
  textarea::placeholder { color: var(--app-text-subtle); }
  textarea:focus-visible { box-shadow: none; }
  textarea:disabled { opacity: 1; }
  .ch-composer--docked textarea { min-height: 48px; font-size: var(--text-md); padding: var(--s-3) var(--s-4) var(--s-1); }
  .ch-bar { display: flex; align-items: center; gap: var(--s-2); padding: var(--s-2) var(--s-2) var(--s-2) var(--s-3); }
  .ch-send--stop :global(rect) { fill: currentColor; } /* lucide draws it hollow; the stop is solid */
  .ch-send kbd { border-color: color-mix(in srgb, currentColor 30%, transparent); color: inherit; opacity: 0.7; }
</style>
