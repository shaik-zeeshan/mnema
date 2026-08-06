<script lang="ts">
  // STANDING CONTEXT (2×1) — the statements you wrote. This is the only tile on
  // the page whose rows you can change: edit rewrites in place, delete removes.
  // Everything on the right half of the page is a read-out.
  //
  // An authored row deliberately carries NO confidence — it is an assertion,
  // not a conclusion — so nothing here shows a percentage or a decay mark.
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { toast } from "$lib/toast.svelte";
  import { humanizeError } from "$lib/format-error";
  import type { AuthoredContext } from "$lib/types/recording";
  import { metaTime, type ContextData } from "./context-data.svelte";

  let { data }: { data: ContextData } = $props();

  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let saving = $state(false);

  const count = $derived(data.statements.length);

  function startEdit(s: AuthoredContext): void {
    editingId = s.id;
    editText = s.text;
    editTopic = s.topic ?? "";
  }

  function cancelEdit(): void {
    editingId = null;
    editText = "";
    editTopic = "";
  }

  async function saveEdit(id: number): Promise<void> {
    const text = editText.trim();
    if (text.length === 0 || saving) return;
    saving = true;
    const topic = editTopic.trim();
    try {
      await data.update(id, text, topic.length > 0 ? topic : null);
      cancelEdit();
    } catch (e) {
      toast({ tone: "error", title: "Couldn't save context", message: humanizeError(e) });
    } finally {
      saving = false;
    }
  }

  // The confirm is the native dialog, and its copy is verbatim: it promises
  // exactly one thing — that the statement stops steering the dossier. It does
  // NOT promise that what was already derived from it is unlearned.
  async function remove(s: AuthoredContext): Promise<void> {
    const ok = await confirm(
      "Delete this context statement? Mnema will no longer use it to steer your dossier.",
      { title: "Delete context", kind: "warning" },
    );
    if (!ok) return;
    try {
      await data.remove(s.id);
      if (editingId === s.id) cancelEdit();
    } catch (e) {
      toast({ tone: "error", title: "Couldn't delete context", message: humanizeError(e) });
    }
  }
</script>

<div class="tile tile--w2 tile--static">
  <div class="tile__h">
    <span class="t-label">Standing context</span>
    {#if count > 0}
      <span class="tile__more is-num">{count}</span>
    {:else if data.loaded}
      <span class="tile__more">nothing yet</span>
    {/if}
  </div>

  {#if data.loadError}
    <div class="pay quiet">
      <span class="t-meta">Couldn't load your context. {data.loadError}</span>
    </div>
  {:else if !data.loaded}
    <div class="pay quiet"><span class="t-meta subtle">Reading…</span></div>
  {:else if count === 0}
    <!-- On this surface the empty state IS the onboarding: the composer's
         instructions, restated. -->
    <div class="pay empty">
      <span class="t-ui strong">No standing context yet.</span>
      <span class="t-meta">
        Add a short statement above — your role, what you're working on, how you work, what you
        care about. Mnema uses it to steer your dossier, and it never fades.
      </span>
    </div>
  {:else}
    <div class="pay pay--rows scroll">
      {#each data.statements as s (s.id)}
        {#if editingId === s.id}
          <div class="row row--static arow is-editing">
            <textarea class="ta" bind:value={editText} aria-label="Edit context statement"
            ></textarea>
            <span class="ameta">
              <input
                class="input topic-edit"
                type="text"
                bind:value={editTopic}
                placeholder="topic (optional)"
                aria-label="Edit topic (optional)"
              />
              <span class="tag tag--edit">✎ editing</span>
              <span class="aact">
                <button
                  type="button"
                  class="btn btn--primary btn--sm"
                  disabled={editText.trim().length === 0 || saving}
                  onclick={() => void saveEdit(s.id)}
                >
                  {saving ? "Saving…" : "Save"}
                </button>
                <button type="button" class="btn btn--ghost btn--sm" onclick={cancelEdit}>
                  Cancel
                </button>
              </span>
            </span>
          </div>
        {:else}
          <div class="row row--static arow">
            <span class="atext">{s.text}</span>
            <span class="ameta">
              {#if s.topic}<span class="tag">{s.topic}</span>{/if}
              <span class="tag tag--auth">✎ Authored</span>
              <span class="awhen">{metaTime(s)}</span>
              <span class="aact">
                <button type="button" class="btn btn--ghost btn--sm" onclick={() => startEdit(s)}>
                  Edit
                </button>
                <button
                  type="button"
                  class="btn btn--ghost btn--sm"
                  onclick={() => void remove(s)}
                >
                  Delete
                </button>
              </span>
            </span>
          </div>
        {/if}
      {/each}
    </div>
  {/if}
</div>

<style>
  .pay--rows {
    overflow-y: auto;
  }
  /* An authored row is block, not a two-column row: the statement is the point,
     and its meta line hangs under it. */
  .arow {
    display: block;
    padding: var(--s-10, 10px) var(--tile-pad);
  }
  .is-editing {
    background: var(--app-surface-subtle);
  }
  .atext {
    display: block;
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .ameta {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    margin-top: var(--s-6);
  }
  .tag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 18px;
    padding: 0 var(--s-6);
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    letter-spacing: var(--ls-label);
    color: var(--app-text-muted);
    white-space: nowrap;
  }
  .tag--auth {
    background: var(--app-accent-bg);
    color: var(--app-accent);
  }
  .tag--edit {
    background: var(--app-warn-bg);
    color: var(--app-warn);
  }
  .awhen {
    font: var(--w-regular) var(--t-label) / 1 var(--app-font-mono);
    font-variant-numeric: tabular-nums;
    color: var(--app-text-faint);
    white-space: nowrap;
  }
  .aact {
    margin-left: auto;
    display: inline-flex;
    gap: var(--s-4);
  }
  .ta {
    display: block;
    width: 100%;
    min-height: 44px;
    padding: var(--s-6) var(--s-8);
    border: var(--hairline) solid var(--app-border-strong);
    border-radius: var(--r-md);
    background: var(--app-surface-raised);
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    color: var(--app-text-strong);
    resize: none;
  }
  .ta:focus-visible,
  .ta:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: var(--ring);
  }
  .topic-edit {
    height: var(--h-sm);
    width: 180px;
    flex: 0 1 180px;
    min-width: 0;
  }
  .quiet {
    display: flex;
    align-items: center;
  }
  .empty {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--s-6);
  }
  .strong {
    font-weight: var(--w-semi);
    color: var(--app-text-strong);
  }
  .subtle {
    color: var(--app-text-subtle);
  }
</style>
