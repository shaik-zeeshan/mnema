<script lang="ts">
  // The standing-context list: ONE opaque plate whose rows are separated by the
  // material's own hairline (page 10). Four states — loading, failed, empty,
  // populated — plus the inline edit row.
  //
  // An AuthoredContext has text, topic, createdAtMs, updatedAtMs and NOTHING
  // else: no score, no status, no decay. So this list never draws a bar, a
  // percentage or a pin, and there is no Dismiss here — dismissing belongs to a
  // conclusion, and conclusions are corrected on Subjects.
  import type { AuthoredContext } from "$lib/types/recording";
  import { contextAgo } from "$lib/insights/context-time";

  interface Props {
    statements: AuthoredContext[] | null;
    loading: boolean;
    loadError: string | null;
    editingId: number | null;
    editText: string;
    editTopic: string;
    savingEdit: boolean;
    onretry: () => void;
    onedit: (s: AuthoredContext) => void;
    ondelete: (s: AuthoredContext) => void;
    onsave: (id: number) => void;
    oncancel: () => void;
  }

  let {
    statements,
    loading,
    loadError,
    editingId,
    editText = $bindable(),
    editTopic = $bindable(),
    savingEdit,
    onretry,
    onedit,
    ondelete,
    onsave,
    oncancel,
  }: Props = $props();

  // Placeholder rows while the list loads — three, like the mockup.
  const SKELETON = [0, 1, 2];

  // "edited" only when the row was updated meaningfully after creation.
  function metaTime(s: AuthoredContext): string {
    return s.updatedAtMs > s.createdAtMs + 1000
      ? `edited ${contextAgo(s.updatedAtMs)}`
      : `added ${contextAgo(s.createdAtMs)}`;
  }
</script>

{#if loadError && !statements}
  <div class="plate box">
    <span class="ttl">Couldn't load your context.</span>
    <p>{loadError}</p>
    <button type="button" class="btn btn--sm retry" onclick={onretry} disabled={loading}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20.5 4v5.5H15" /><path d="M20 9.5A8.5 8.5 0 1 0 20.2 15" />
      </svg>
      Try again
    </button>
  </div>
{:else if loading && !statements}
  <div class="plate slist" aria-label="Loading context" aria-busy="true">
    {#each SKELETON as i (i)}
      <div class="stmt">
        <span class="stmt__t">
          <span class="skel" style="width:{[86, 72, 80][i]}%; height:11px"></span>
          <span class="skel" style="width:{[38, 30, 34][i]}%; height:8px; margin-top:7px"></span>
        </span>
      </div>
    {/each}
  </div>
{:else if (statements?.length ?? 0) === 0}
  <div class="plate box">
    <span class="ttl">No standing context yet.</span>
    <p>
      Add a short statement above — your role, what you're working on, how you work, what
      you care about. Mnema uses it to steer your dossier, and it never fades.
    </p>
  </div>
{:else}
  <div class="plate slist">
    {#each statements ?? [] as s (s.id)}
      {#if editingId === s.id}
        <div class="stmt stmt--editing">
          <textarea bind:value={editText} class="ta" aria-label="Edit context statement"
          ></textarea>
          <input
            bind:value={editTopic}
            class="input ti"
            type="text"
            placeholder="topic (optional)"
            aria-label="Edit topic (optional)"
          />
          <div class="erow">
            <span class="auth"><i aria-hidden="true">✎</i>editing</span>
            <button type="button" class="btn btn--ghost btn--sm cancel" onclick={oncancel}>
              Cancel
            </button>
            <button
              type="button"
              class="btn btn--primary btn--sm"
              disabled={editText.trim().length === 0 || savingEdit}
              onclick={() => onsave(s.id)}
            >
              {savingEdit ? "Saving…" : "Save"}
            </button>
          </div>
        </div>
      {:else}
        <div class="stmt">
          <span class="stmt__t">
            <span class="x">{s.text}</span>
            <span class="stmt__m">
              {#if s.topic}<span class="topic">{s.topic}</span>{/if}
              <span class="auth"><i aria-hidden="true">✎</i>Authored</span>
              <span class="t-meta subtle">{metaTime(s)}</span>
            </span>
          </span>
          <span class="stmt__a">
            <button type="button" class="btn btn--ghost btn--sm" onclick={() => onedit(s)}>
              Edit
            </button>
            <button type="button" class="btn btn--ghost btn--sm del" onclick={() => ondelete(s)}>
              Delete
            </button>
          </span>
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  /* One plate, rows inside it — never a plate per row (box-in-box). */
  .slist {
    border-radius: var(--r-panel);
    padding: 0 12px;
  }
  .stmt {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 8px 0;
    min-height: 40px;
  }
  .stmt + .stmt {
    box-shadow: inset 0 1px 0 var(--glass-line);
  }
  .stmt__t {
    flex: 1;
    min-width: 0;
  }
  .stmt__t .x {
    display: block;
    max-width: 62ch;
    font: var(--w-medium) var(--t-read) / 1.45 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .stmt__m {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 7px;
    margin-top: 5px;
  }
  .subtle {
    color: var(--app-text-subtle);
  }

  .topic {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--r-sm);
    background: var(--glass-tint);
    color: var(--app-text-muted);
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .topic::before {
    content: "[";
  }
  .topic::after {
    content: "]";
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

  /* Row actions appear on hover — and on keyboard focus, or they'd be
     unreachable without a pointer. */
  .stmt__a {
    display: flex;
    gap: 4px;
    flex: 0 0 auto;
    opacity: 0;
    transition: opacity var(--dur-quick) var(--ease);
  }
  .stmt:hover .stmt__a,
  .stmt:focus-within .stmt__a {
    opacity: 1;
  }
  .del:hover {
    color: var(--app-danger);
    background: var(--app-danger-bg);
  }

  /* inline edit */
  .stmt--editing {
    flex-direction: column;
    align-items: stretch;
    gap: 7px;
  }
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
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border), var(--ring);
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
  }
  .ta:focus {
    outline: none;
  }
  .ti {
    width: 220px;
  }
  .erow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .erow .cancel {
    margin-left: auto;
  }

  /* states */
  .box {
    border-radius: var(--r-panel);
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: flex-start;
  }
  .box .ttl {
    font: var(--w-semi) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .box p {
    margin: 0;
    max-width: 62ch;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .retry svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
  }

  .skel {
    display: block;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    animation: pulse 1.6s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.5;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .skel {
      animation: none;
    }
  }
</style>
