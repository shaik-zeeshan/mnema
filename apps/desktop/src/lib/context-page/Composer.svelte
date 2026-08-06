<script lang="ts">
  // ADD CONTEXT (4×1) — the composer. The direction's claim is that a tile's
  // payload is free: this one's is a text field, a topic field and five prefill
  // chips instead of a row list, under the SAME 18px header row as every other
  // tile.
  //
  // The copy on this tile is a privacy contract, not decoration: authored
  // context genuinely carries no confidence and no decay (unlike a conclusion,
  // which is recency-weighted), so "never fade" is a fact about the backend.
  import type { ContextData } from "./context-data.svelte";
  import { humanizeError } from "$lib/format-error";

  let { data }: { data: ContextData } = $props();

  // Cosmetic chip → starter phrase. The chip drops the phrase into the
  // composer; it never writes anything on its own.
  const SUGGESTIONS: { label: string; prompt: string }[] = [
    { label: "Your role", prompt: "I'm a … " },
    { label: "What you're working on", prompt: "I'm currently working on … " },
    { label: "How you work", prompt: "I prefer to work by … " },
    { label: "What you care about", prompt: "I care deeply about … " },
    { label: "Goals this quarter", prompt: "Goal: " },
  ];

  let text = $state("");
  let topic = $state("");
  let submitting = $state(false);
  let error = $state<string | null>(null);
  let textEl = $state<HTMLTextAreaElement | null>(null);

  const canSubmit = $derived(text.trim().length > 0 && !submitting);
  // The engine's derivation budget tier — a real backend field, so it is shown;
  // it is absent (not guessed) when the status read failed.
  const tier = $derived.by(() => {
    const t = data.status?.budgetTier;
    return t ? t.charAt(0).toUpperCase() + t.slice(1) : null;
  });

  // ⌘⏎ submits from EITHER field — the keycap sits next to the button, so the
  // shortcut has to work wherever the caret happens to be.
  function onComposerKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      void submit();
    }
  }

  function applySuggestion(prompt: string): void {
    text = text.trim().length === 0 ? prompt : `${text} ${prompt}`;
    textEl?.focus();
  }

  async function submit(): Promise<void> {
    const body = text.trim();
    if (body.length === 0 || submitting) return;
    submitting = true;
    error = null;
    const t = topic.trim();
    try {
      await data.add(body, t.length > 0 ? t : null);
      text = "";
      topic = "";
    } catch (e) {
      error = humanizeError(e);
    } finally {
      submitting = false;
    }
  }
</script>

<div class="tile tile--w4 tile--static">
  <div class="tile__h">
    <span class="t-label">Add context</span>
    <span class="tile__more">
      what you tell Mnema about yourself
      {#if tier}<span class="chip chip--verdict chip--flat">{tier}</span>{/if}
      <span class="chip chip--verdict chip--ok">✎ authored</span>
    </span>
  </div>

  <div class="cx-body">
    <div class="cx-main">
      <textarea
        bind:this={textEl}
        bind:value={text}
        class="ta"
        placeholder="I'm a… I care about… I work best with…"
        aria-label="Add a context statement"
        onkeydown={onComposerKey}
      ></textarea>

      <div class="cx-foot">
        <input
          bind:value={topic}
          class="input topic"
          type="text"
          placeholder="topic (optional, e.g. role, focus, goal)"
          aria-label="Topic for this statement (optional)"
          onkeydown={onComposerKey}
        />
        <span class="cx-help t-meta">
          <span aria-hidden="true">›</span> Authored statements never fade from your dossier.
        </span>
        <button
          type="button"
          class="btn btn--primary btn--sm add"
          disabled={!canSubmit}
          onclick={() => void submit()}
        >
          <svg viewBox="0 0 12 12" aria-hidden="true">
            <path d="M6 2v8M2 6h8" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" />
          </svg>
          {submitting ? "Adding…" : "Add"}
        </button>
        <span class="kbd kbd--mod">⌘ ⏎</span>
      </div>

      {#if error}<p class="cx-err t-meta">{error}</p>{/if}
    </div>

    <div class="cx-try">
      <span class="t-label faint">Try</span>
      <div class="cx-chips">
        {#each SUGGESTIONS as s (s.label)}
          <button type="button" class="chip chip--flat" onclick={() => applySuggestion(s.prompt)}>
            {s.label}
          </button>
        {/each}
      </div>
    </div>
  </div>
</div>

<style>
  .tile__more .chip {
    margin-left: var(--s-8);
  }
  .cx-body {
    display: flex;
    gap: var(--cell-gutter);
    flex: 1 1 auto;
    min-height: 0;
  }
  .cx-main {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
  }
  /* The composer field: a recessed well, not a bordered card. */
  .ta {
    flex: 0 1 auto;
    min-height: 62px;
    padding: var(--s-8) var(--pad-control);
    border: var(--hairline) solid var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    font: var(--w-regular) var(--t-read) / var(--lh-read) var(--app-font-sans);
    color: var(--app-text-strong);
    resize: none;
  }
  .ta::placeholder {
    color: var(--app-text-subtle);
  }
  .ta:focus-visible,
  .ta:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: var(--ring);
  }
  .cx-foot {
    display: flex;
    align-items: center;
    gap: var(--s-8);
  }
  .topic {
    width: 280px;
    flex: 0 1 280px;
    min-width: 0;
  }
  .cx-help {
    color: var(--app-text-subtle);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .add {
    margin-left: auto;
  }
  .add svg {
    width: 10px;
    height: 10px;
    fill: none;
  }
  .cx-err {
    margin: 0;
    color: var(--app-danger);
  }
  .cx-try {
    flex: 0 0 300px;
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    min-width: 0;
  }
  .faint {
    color: var(--app-text-faint);
  }
  .cx-chips {
    display: flex;
    align-items: flex-start;
    align-content: flex-start;
    flex-wrap: wrap;
    gap: var(--s-6);
  }
  .cx-chips .chip {
    border: 0;
    cursor: pointer;
  }
  .cx-chips .chip:hover {
    background: var(--app-surface-active);
    color: var(--app-text-strong);
  }
  .cx-chips .chip:focus-visible {
    outline: none;
    box-shadow: var(--ring);
  }

  /* 800×600: the Try column is the first thing to go — the composer keeps its
     full width and the chips are a shortcut, never the only way in. */
  @media (max-width: 900px) {
    .cx-try {
      display: none;
    }
  }
</style>
