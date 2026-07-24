<script lang="ts">
  // TodayComposer — the Ask-Mnema composer + suggestion chips on the Today
  // front page (Warm Paper redesign, Slice 3). Purely presentational: chips
  // prefill the textarea, Enter / ↑ submits the draft via `onSubmit` (the
  // parent routes it into Chat over the conversation bus). Chips are
  // mechanical templates (chip-fill.ts) — no LLM call here.
  // ponytail: no model picker in this bar — the chat pane owns model choice,
  // and the bus hand-off carries only text; add one if the hand-off ever
  // carries a model pin.
  import type { SuggestionChip } from "$lib/insights/chip-fill";

  interface Props {
    chips: SuggestionChip[];
    onSubmit: (text: string) => void;
  }

  let { chips, onSubmit }: Props = $props();

  let draft = $state("");
  let inputEl = $state<HTMLTextAreaElement | null>(null);
  const canSend = $derived(draft.trim().length > 0);

  function submit(): void {
    const text = draft.trim();
    if (text.length === 0) return;
    draft = "";
    onSubmit(text);
  }

  function onKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
    }
  }

  function pickChip(chip: SuggestionChip): void {
    draft = chip.text;
    // WKWebView doesn't focus buttons on click; put the caret in the textarea
    // at the end so the user can edit or just hit Enter.
    inputEl?.focus();
    if (inputEl) {
      const end = inputEl.value.length;
      inputEl.setSelectionRange(end, end);
    }
  }
</script>

<div class="composer">
  <textarea
    bind:this={inputEl}
    bind:value={draft}
    placeholder="Ask Mnema about your day…"
    rows="1"
    onkeydown={onKeydown}
  ></textarea>
  <div class="composer-bar">
    <span class="composer-hint">↵ opens in Chat</span>
    <button
      type="button"
      class="composer-send"
      disabled={!canSend}
      aria-label="Ask in Chat"
      onclick={submit}
    >↑</button>
  </div>
</div>
{#if chips.length > 0}
  <div class="front-chips">
    {#each chips as chip (chip.text)}
      <button type="button" class="chip" onclick={() => pickChip(chip)}>
        <span class="cm" aria-hidden="true">{chip.glyph}</span>{chip.text}
      </button>
    {/each}
  </div>
{/if}

<style>
  /* Token-clean (`--app-*` only); the Warm Paper retheme is a token swap. */
  .composer {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 14px;
    margin-top: 16px;
    transition:
      border-color 0.15s ease,
      box-shadow 0.15s ease;
  }
  .composer:focus-within {
    border-color: var(--app-accent-border);
    box-shadow: var(--app-ring);
  }
  .composer textarea {
    display: block;
    width: 100%;
    border: 0;
    outline: 0;
    resize: none;
    background: transparent;
    font: inherit;
    font-size: var(--text-md);
    line-height: 1.5;
    color: var(--app-text-strong);
    padding: 14px 18px 0;
    height: 46px;
  }
  .composer textarea::placeholder {
    color: var(--app-text-faint);
  }
  .composer-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 9px 9px 15px;
  }
  .composer-hint {
    font-family: var(--app-font-mono);
    font-size: var(--text-xs);
    letter-spacing: 0.04em;
    color: var(--app-text-faint);
  }
  .composer-send {
    margin-left: auto;
    width: 29px;
    height: 29px;
    border-radius: 9px;
    border: 0;
    background: var(--app-accent);
    color: var(--app-bg);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    transition: background 0.12s ease;
  }
  .composer-send:hover:not(:disabled) {
    background: var(--app-accent-strong);
  }
  .composer-send:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .composer-send:disabled {
    background: var(--app-border-strong);
    color: var(--app-text-faint);
    cursor: default;
  }
  .front-chips {
    display: flex;
    gap: 9px;
    margin-top: 12px;
    flex-wrap: wrap;
  }
  .chip {
    font: inherit;
    font-size: var(--text-base);
    color: var(--app-text-muted);
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 999px;
    padding: 6px 15px;
    cursor: pointer;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    transition:
      border-color 0.12s ease,
      color 0.12s ease,
      background 0.12s ease;
  }
  .chip:hover {
    border-color: var(--app-accent-border);
    color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .chip:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .chip .cm {
    font-family: var(--app-font-mono);
    font-size: var(--text-sm);
    color: var(--app-accent);
    margin-right: 7px;
  }
</style>
