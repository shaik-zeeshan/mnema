<script lang="ts">
  // The statement + topic field pair, used twice on the Context destination:
  // once by the composer and once by an inline edit. Stock fields in a recessed
  // well — no instrument, because a sentence about yourself has no physical
  // quantity to state.
  import type { Snippet } from "svelte";

  let {
    text = $bindable(""),
    topic = $bindable(""),
    el = $bindable<HTMLTextAreaElement | null>(null),
    placeholder = "",
    topicPlaceholder = "topic (optional)",
    label,
    topicLabel = "Topic (optional)",
    onsubmit,
    actions,
  }: {
    text: string;
    topic: string;
    el?: HTMLTextAreaElement | null;
    placeholder?: string;
    topicPlaceholder?: string;
    label: string;
    topicLabel?: string;
    onsubmit?: () => void;
    actions: Snippet;
  } = $props();
</script>

<textarea
  bind:this={el}
  bind:value={text}
  class="ta"
  {placeholder}
  aria-label={label}
  onkeydown={(e) => {
    if (onsubmit && (e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      onsubmit();
    }
  }}
></textarea>

<div class="row">
  <input
    bind:value={topic}
    class="fld"
    type="text"
    placeholder={topicPlaceholder}
    aria-label={topicLabel}
  />
  {@render actions()}
</div>

<style>
  .ta {
    display: block;
    width: 100%;
    min-height: 52px;
    resize: vertical;
    padding: var(--s-8);
    border: 0;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    letter-spacing: var(--ls-read);
    color: var(--app-text-strong);
  }
  .fld {
    flex: 1 1 auto;
    min-width: 0;
    height: var(--h-md);
    padding: 0 var(--pad-control);
    border: 0;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    font: var(--w-regular) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .ta::placeholder,
  .fld::placeholder {
    color: var(--app-text-subtle);
  }
  .ta:focus,
  .fld:focus {
    outline: none;
    box-shadow:
      inset 0 0 0 var(--hairline) var(--app-accent-border),
      var(--ring);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-top: var(--s-8);
  }
</style>
