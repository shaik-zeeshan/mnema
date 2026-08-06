<script lang="ts">
  // The Context composer — page 10's verb, so it sits at the TOP of the page.
  // Owns the draft (text + optional topic) and the suggestion chips; the parent
  // owns the mutation and reports success, which is when the draft clears.
  import { tip } from "$lib/components/tooltip";

  interface Props {
    /** Derivation-budget tier badge label ("Balanced"), from the engine status. */
    tier: string;
    submitting: boolean;
    error: string | null;
    /** Resolves true when the statement was stored, which clears the draft. */
    onadd: (text: string, topic: string | null) => Promise<boolean>;
  }

  let { tier, submitting, error, onadd }: Props = $props();

  // Static suggestion chips that prefill the composer.
  const SUGGESTIONS: { label: string; prompt: string }[] = [
    { label: "Your role", prompt: "I'm a … " },
    { label: "What you're working on", prompt: "I'm currently working on … " },
    { label: "How you work", prompt: "I prefer to work by … " },
    { label: "What you care about", prompt: "I care deeply about … " },
    { label: "Goals this quarter", prompt: "Goal: " },
  ];

  let text = $state("");
  let topic = $state("");
  let field = $state<HTMLTextAreaElement | null>(null);

  const canSubmit = $derived(text.trim().length > 0 && !submitting);

  function applySuggestion(prompt: string): void {
    text = text.trim().length === 0 ? prompt : `${text} ${prompt}`;
    field?.focus();
  }

  async function submit(): Promise<void> {
    const body = text.trim();
    if (body.length === 0 || submitting) return;
    const t = topic.trim();
    if (await onadd(body, t.length > 0 ? t : null)) {
      text = "";
      topic = "";
    }
  }
</script>

<div class="plate card">
  <div class="card__h">
    <span class="t-label">Add context</span>
    <span class="r">
      <span class="chip" use:tip={"Reasoning Engine derivation tier"}>{tier}</span>
      <span class="auth"><i aria-hidden="true">✎</i>authored</span>
    </span>
  </div>

  <textarea
    bind:this={field}
    bind:value={text}
    class="ta"
    placeholder="I'm a… I care about… I work best with…"
    aria-label="Add a context statement"
    onkeydown={(e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
        e.preventDefault();
        void submit();
      }
    }}
  ></textarea>

  <input
    bind:value={topic}
    class="input ti"
    type="text"
    placeholder="topic (optional, e.g. role, focus, goal)"
    aria-label="Topic for this statement (optional)"
  />

  <div class="trychips">
    <span class="t-meta subtle">Try</span>
    {#each SUGGESTIONS as s (s.label)}
      <button type="button" class="chip chip--try" onclick={() => applySuggestion(s.prompt)}>
        {s.label}
      </button>
    {/each}
  </div>

  <div class="cfoot">
    <span class="t-meta subtle">› Authored statements never fade from your dossier.</span>
    <button
      type="button"
      class="btn btn--primary btn--sm add"
      disabled={!canSubmit}
      onclick={() => void submit()}
    >
      <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 5.5v13M5.5 12h13" /></svg>
      {submitting ? "Adding…" : "Add"}
    </button>
  </div>

  {#if error}
    <p class="err">{error}</p>
  {/if}
</div>

<style>
  /* The plate's radius is the direction's panel radius, not `.plate`'s --r-lg. */
  .card {
    border-radius: var(--r-panel);
    padding: 10px 12px 11px;
  }
  .card__h {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 7px;
  }
  .card__h .r {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 6px;
  }

  /* A text field is a control, so it keeps the app's inset bezel. */
  .ta {
    display: block;
    width: 100%;
    min-height: 50px;
    resize: vertical;
    padding: 8px 10px;
    border: 0;
    border-radius: var(--r-md);
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border-strong);
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
  }
  .ta::placeholder {
    color: var(--app-text-subtle);
  }
  .ta:focus {
    outline: none;
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border), var(--ring);
  }

  .ti {
    width: 100%;
    margin-top: 7px;
  }
  .ti::placeholder {
    color: var(--app-text-subtle);
  }

  /* `.t-meta` is global; the quieter tone is not, so it stays local. */
  .subtle {
    color: var(--app-text-subtle);
  }

  .trychips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 8px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--glass-tint);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .chip--try {
    cursor: pointer;
    transition: background var(--dur-quick) var(--ease), color var(--dur-quick) var(--ease);
  }
  .chip--try:hover {
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }

  .auth {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--r-pill);
    background: var(--app-accent-bg);
    color: var(--app-accent);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-sans);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }

  .cfoot {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 9px;
  }
  .cfoot .add {
    margin-left: auto;
  }
  .add svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
  }

  .err {
    margin: 8px 0 0;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-danger);
  }
</style>
