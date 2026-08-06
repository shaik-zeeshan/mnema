<script lang="ts">
  // The standing ledger: one row per sentence the user wrote.
  //
  // The entire correction vocabulary this data has is Edit and Delete — no
  // confirm/reject (those belong to inferred beliefs on Subjects), no confidence
  // bar (the row has no confidence field), no evidence list (you asserted it).
  // Edit turns the row into the SAME composer it was written with, in place.
  import { confirm } from "@tauri-apps/plugin-dialog";
  import IconCheck from "~icons/lucide/check";
  import IconPen from "~icons/lucide/pencil-line";
  import type { AuthoredContext } from "$lib/types/recording";
  import Composer from "./Composer.svelte";
  import { metaTime, type ContextData } from "./context-data.svelte";

  interface Props {
    data: ContextData;
  }

  let { data }: Props = $props();

  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let savingEdit = $state(false);
  let editError = $state<string | null>(null);

  const selectedId = $derived(
    data.focus.kind === "authored" || data.focus.kind === "editing" ? data.focus.item.id : null,
  );

  function startEdit(s: AuthoredContext): void {
    editingId = s.id;
    editText = s.text;
    editTopic = s.topic ?? "";
    editError = null;
    data.focus = { kind: "editing", item: s };
  }

  function cancelEdit(): void {
    editingId = null;
    editError = null;
    if (data.focus.kind === "editing") data.focus = { kind: "authored", item: data.focus.item };
  }

  async function saveEdit(id: number): Promise<void> {
    const text = editText.trim();
    if (text.length === 0 || savingEdit) return;
    savingEdit = true;
    const failure = await data.update(id, text, editTopic.trim());
    savingEdit = false;
    if (failure) {
      editError = failure;
      return;
    }
    cancelEdit();
  }

  async function remove(s: AuthoredContext): Promise<void> {
    // Native sheet, per the repo's dialog rule — never window.confirm.
    const ok = await confirm(
      "Delete this context statement? Mnema will no longer use it to steer your dossier.",
      { title: "Delete context", kind: "warning" },
    );
    if (!ok) return;
    if (editingId === s.id) cancelEdit();
    await data.remove(s.id);
  }
</script>

<div class="chd">
  <span class="chd__n">Standing context</span>
  {#if data.standingCount !== null}
    <span class="chd__c is-mono">{data.standingCount}</span>
  {/if}
</div>

{#if data.loading && data.statements === null}
  <p class="quiet">Reading what you told Mnema…</p>
{:else if data.loadError && (data.statements?.length ?? 0) === 0}
  <p class="quiet">{data.loadError}</p>
{:else if (data.statements?.length ?? 0) === 0}
  <p class="quiet">Nothing yet. Write a sentence above — it steers your dossier from the moment you add it.</p>
{:else}
  <div class="ss-grp grp">
    {#each data.statements ?? [] as s (s.id)}
      {#if editingId === s.id}
        <div class="arow arow--edit">
          <Composer
            variant="edit"
            bind:text={editText}
            bind:topic={editTopic}
            busy={savingEdit}
            error={editError}
            autofocus
            onsubmit={() => void saveEdit(s.id)}
            oncancel={cancelEdit}
          />
        </div>
      {:else}
        <div
          class="arow"
          class:is-sel={selectedId === s.id}
          role="button"
          tabindex="0"
          onclick={() => (data.focus = { kind: "authored", item: s })}
          onfocusin={() => (data.focus = { kind: "authored", item: s })}
          onkeydown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.preventDefault();
              data.focus = { kind: "authored", item: s };
            }
          }}
        >
          <div class="arow__t">
            <p class="arow__x">{s.text}</p>
            <div class="arow__m">
              {#if s.topic}<span class="topicchip">{s.topic}</span>{/if}
              <span class="authored"><IconPen /> authored</span>
              <span class="t-meta age is-mono">{metaTime(s)}</span>
              {#if data.echoId === s.id}
                <span class="ss-echo"><IconCheck /> Saved</span>
              {/if}
            </div>
          </div>
          <!-- Both actions stop the click: the row's own handler would otherwise
               bubble afterwards and drag the inspector back off the edit record. -->
          <div class="arow__a">
            <button
              type="button"
              class="btn btn--sm btn--ghost"
              onclick={(e) => {
                e.stopPropagation();
                startEdit(s);
              }}>Edit</button
            >
            <button
              type="button"
              class="btn btn--sm btn--ghost"
              onclick={(e) => {
                e.stopPropagation();
                void remove(s);
              }}>Delete</button
            >
          </div>
        </div>
      {/if}
    {/each}
  </div>
{/if}

<style>
  /* A section header is a hairline and a name, not a card edge. */
  .chd {
    display: flex;
    align-items: baseline;
    gap: var(--s-8);
    height: 26px;
    padding: 0 var(--s-16);
    margin-top: 14px;
    background: var(--app-bg);
    border-bottom: var(--hairline) solid var(--app-border);
  }

  .chd__n {
    font: var(--w-semi) var(--t-ui) / 1 var(--app-font-sans);
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }

  .chd__c {
    margin-left: auto;
    font: var(--w-regular) var(--t-meta) / 1 var(--app-font-mono);
    color: var(--app-text-faint);
    font-variant-numeric: tabular-nums;
  }

  .grp {
    margin: var(--s-8) var(--s-16) 0;
  }

  .quiet {
    margin: var(--s-10) var(--s-16) 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-text-muted);
  }

  .arow {
    display: flex;
    align-items: flex-start;
    gap: var(--s-10);
    padding: var(--s-8) var(--s-10);
    border-top: var(--hairline) solid var(--app-border);
  }

  .arow:first-child {
    border-top: 0;
  }

  .arow:hover {
    background: var(--app-surface-hover);
  }

  .arow:focus-visible {
    outline: none;
  }

  /* Selection is a tint and an accent edge — the row's text stays readable,
     unlike the kit's full-accent list rows: this row IS the content. */
  .arow.is-sel {
    background: var(--app-surface-active);
    box-shadow: inset 2px 0 0 var(--app-accent);
  }

  .arow--edit {
    display: block;
    background: var(--app-surface-active);
    box-shadow: inset 2px 0 0 var(--app-accent);
  }

  .arow__t {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .arow__x {
    margin: 0;
    max-width: 72ch;
    font: var(--w-regular) var(--t-read) / 1.5 var(--app-font-sans);
    color: var(--app-text-strong);
  }

  .arow__m {
    display: flex;
    align-items: center;
    gap: var(--s-6);
    flex-wrap: wrap;
  }

  .arow__a {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 0 0 auto;
  }

  .age {
    color: var(--app-text-subtle);
  }

  .topicchip {
    display: inline-flex;
    align-items: center;
    height: 17px;
    padding: 0 6px;
    border-radius: var(--r-sm);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
  }

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

  .authored :global(svg),
  .arow :global(.ss-echo svg) {
    width: 9px;
    height: 9px;
  }
</style>
