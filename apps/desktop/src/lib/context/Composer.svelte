<script lang="ts">
  // One composer, two placements. Adding sits at the TOP of the scroll (adding
  // context is the reason you came here, so it is not a modal); editing turns a
  // ledger row into this exact control in place. Same textarea, same free-text
  // topic field, same ⌘⏎ — so an edit is visibly the same act as an add.
  //
  // The topic field is FREE TEXT, deliberately: `user_context_authored.topic` is
  // a nullable TEXT column, so a category picker here would be a fiction.
  import IconCheck from "~icons/lucide/check";
  import IconPlus from "~icons/lucide/plus";
  import { SUGGESTIONS } from "./context-data.svelte";

  interface Props {
    text: string;
    topic: string;
    /** "add" carries the suggestion chips and the never-fades footnote. */
    variant: "add" | "edit";
    busy?: boolean;
    error?: string | null;
    autofocus?: boolean;
    onsubmit: () => void;
    oncancel?: () => void;
  }

  let {
    text = $bindable(),
    topic = $bindable(),
    variant,
    busy = false,
    error = null,
    autofocus = false,
    onsubmit,
    oncancel,
  }: Props = $props();

  let area = $state<HTMLTextAreaElement | null>(null);

  const canSubmit = $derived(text.trim().length > 0 && !busy);

  $effect(() => {
    if (autofocus) area?.focus();
  });

  function applySuggestion(prompt: string): void {
    text = text.trim().length === 0 ? prompt : `${text} ${prompt}`;
    area?.focus();
  }

  function onkeydown(event: KeyboardEvent): void {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      if (canSubmit) onsubmit();
      return;
    }
    if (event.key === "Escape" && oncancel) {
      event.preventDefault();
      oncancel();
    }
  }
</script>

<div class="composer" class:composer--edit={variant === "edit"}>
  {#if variant === "edit"}
    <div class="editing">
      <span class="t-label lbl">Editing</span>
      <span class="ss-chip"><IconCheck /> editing</span>
    </div>
  {/if}

  <textarea
    bind:this={area}
    bind:value={text}
    class="ta"
    rows={variant === "edit" ? 3 : 2}
    placeholder="I'm a… I care about… I work best with…"
    aria-label={variant === "edit" ? "Edit this statement" : "What you tell Mnema about yourself"}
    {onkeydown}
  ></textarea>

  <div class="line">
    <input
      class="input topic"
      type="text"
      bind:value={topic}
      placeholder="topic (optional, e.g. role, focus, goal)"
      aria-label="Topic"
      {onkeydown}
    />
    {#if variant === "add"}
      <span class="authored"><IconCheck /> authored</span>
    {/if}
    <span class="spacer"></span>
    {#if variant === "edit"}
      <button type="button" class="btn btn--sm btn--ghost" onclick={() => oncancel?.()}>Cancel</button>
      <button
        type="button"
        class="btn btn--sm btn--primary"
        disabled={!canSubmit}
        onclick={onsubmit}>Save</button
      >
    {:else}
      <span class="t-meta hint"><span class="kbd">⌘</span><span class="kbd">⏎</span> to add</span>
      <button
        type="button"
        class="btn btn--sm btn--primary"
        disabled={!canSubmit}
        onclick={onsubmit}><IconPlus /> Add</button
      >
    {/if}
  </div>

  {#if variant === "add"}
    <div class="sugg">
      {#each SUGGESTIONS as s (s.label)}
        <button type="button" class="ss-chip chipbtn" onclick={() => applySuggestion(s.prompt)}
          >{s.label}</button
        >
      {/each}
    </div>
    <p class="t-meta foot">› Authored statements never fade from your dossier.</p>
  {/if}

  {#if error}
    <p class="t-meta err" role="alert">{error}</p>
  {/if}
</div>

<style>
  .composer {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    padding: var(--s-10);
    background: var(--app-surface);
    border-radius: var(--r-lg);
  }

  .composer--edit {
    padding: 0;
    background: transparent;
  }

  .editing {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }

  .lbl {
    color: var(--app-text-strong);
  }

  .ta {
    min-height: 52px;
    padding: 7px 9px;
    border-radius: var(--r-md);
    border: var(--hairline) solid var(--app-border-strong);
    background: var(--app-bg);
    color: var(--app-text-strong);
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    resize: vertical;
  }

  .ta::placeholder {
    color: var(--app-text-subtle);
  }

  .ta:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: var(--ring);
  }

  .line {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }

  .topic {
    width: 280px;
    height: 24px;
    font-size: 12px;
  }

  .spacer {
    flex: 1 1 auto;
  }

  .hint {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }

  /* The one badge that states this row's kind — authored, never inferred. */
  .authored {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 17px;
    padding: 0 6px;
    border-radius: var(--r-pill);
    background: var(--app-accent-bg);
    border: var(--hairline) solid var(--app-accent-border);
    color: var(--app-accent-strong);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .authored :global(svg) {
    width: 9px;
    height: 9px;
  }

  .sugg {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }

  .chipbtn {
    cursor: default;
  }

  .chipbtn:hover {
    background: var(--app-surface-hover);
    color: var(--app-text-strong);
  }

  .foot {
    margin: 0;
    color: var(--app-text-subtle);
  }

  .err {
    margin: 0;
    color: var(--app-danger-strong);
  }

  .composer :global(.ss-chip svg) {
    width: 9px;
    height: 9px;
  }
</style>
