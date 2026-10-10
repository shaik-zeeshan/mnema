<script lang="ts">
  // Context — what the user tells Mnema in their own words (insights.html
  // renderContext). Authored statements never fade and steer the dossier first;
  // they are sent to the engine as written (the Sensitive Category Guardrail
  // filters what Mnema INFERS, never what you write). The side column is
  // ContextSide.
  //
  //   list_user_context_authored   → AuthoredContext[] (newest first, `inPrompt`)
  //   user_context_add_authored     { text, topic } → AuthoredContext
  //   user_context_update_authored  { id, text, topic }
  //   user_context_delete_authored  { id }
  // Every local add / edit / delete re-lists, so `inPrompt` and "N of M used"
  // never go stale. Also re-lists on `user_context_changed`.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import type { AiProviderKind, AiRuntimeStatus, AuthoredContext, Conclusion, DismissedView } from "$lib/types/recording";
  import { aiProviderKindLabel, isCloudAiProviderKind } from "$lib/settings/state/ai-providers";
  import { humanizeError } from "$lib/format-error";
  import ContextSide, { GUARDED, TOPICS, topicColor } from "$lib/insights/ContextSide.svelte";
  import IconPlus from "~icons/lucide/plus";
  import IconEdit from "~icons/lucide/pencil";
  import IconTrash from "~icons/lucide/trash-2";
  import IconInbox from "~icons/lucide/inbox";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconCloud from "~icons/lucide/cloud";
  import IconLaptop from "~icons/lucide/laptop";

  interface Props {
    // Subjects' "View dismissed": land with the Dismissed archive in view.
    openDismissed?: boolean;
    onOpenSubject?: (subject: string) => void;
  }
  let { openDismissed = false, onOpenSubject }: Props = $props();

  // Per-statement limit (ruling #15); the add/update commands enforce it too.
  const MAX_CHARS = 280;
  const WORDS = ["Nothing", "One thing", "Two things", "Three things", "Four things", "Five things", "Six things", "Seven things", "Eight things", "Nine things"];

  let statements = $state<AuthoredContext[] | null>(null);
  let loadError = $state(false);
  let loading = $state(true);
  let inferred = $state<Conclusion[] | null>(null);
  /** The default model's provider kind — where statements go (null: no default). */
  let providerKind = $state<AiProviderKind | null>(null);
  let dismissed = $state<DismissedView[]>([]);
  let dismissedError = $state<string | null>(null);
  let restoringKey = $state<string | null>(null);

  let draftText = $state("");
  let draftTopic = $state<string | null>(null);
  let submitting = $state(false);
  let composerError = $state<string | null>(null);

  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state<string | null>(null);
  let savingEdit = $state(false);

  const failed = $derived(loadError && !statements);
  const n = $derived(statements?.length ?? 0);
  // An optimistic local add has no flag yet: it counts as sent until the re-list.
  const unsent = $derived((statements ?? []).filter((s) => s.inPrompt === false).length);

  const day = (ms: number) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
  const metaTime = (s: AuthoredContext) =>
    s.updatedAtMs > s.createdAtMs + 1000 ? `edited ${day(s.updatedAtMs)}` : `authored ${day(s.createdAtMs)}`;

  async function loadStatements(): Promise<void> {
    loading = true;
    try {
      statements = await invoke<AuthoredContext[]>("list_user_context_authored");
      loadError = false;
    } catch {
      // A first-load failure leaves `statements` null → the error, never a fake 0.
      loadError = true;
    } finally {
      loading = false;
    }
  }

  async function loadInferred(): Promise<void> {
    const list = await invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }).catch(() => null);
    if (list) inferred = list.filter((c) => c.status === "visible");
  }

  async function loadProvider(): Promise<void> {
    const status = await invoke<AiRuntimeStatus>("get_ai_runtime_status").catch(() => null);
    providerKind = status?.defaultProviderKind ?? null;
  }

  async function loadDismissed(): Promise<void> {
    try {
      dismissed = await invoke<DismissedView[]>("user_context_list_dismissed");
      dismissedError = null;
    } catch (error) {
      dismissedError = humanizeError(error);
    }
  }

  async function restoreDismissed(d: DismissedView): Promise<void> {
    const key = `${d.subject}\0${d.statement}`;
    if (restoringKey) return;
    restoringKey = key;
    try {
      await invoke("user_context_restore_dismissed", { subject: d.subject, statement: d.statement });
      dismissed = dismissed.filter((x) => `${x.subject}\0${x.statement}` !== key);
      dismissedError = null;
    } catch (error) {
      dismissedError = humanizeError(error);
    } finally {
      restoringKey = null;
    }
  }

  async function submitDraft(): Promise<void> {
    const text = draftText.trim();
    if (!text || submitting) return;
    submitting = true;
    composerError = null;
    try {
      const created = await invoke<AuthoredContext>("user_context_add_authored", { text, topic: draftTopic });
      statements = [created, ...(statements ?? [])];
      draftText = "";
      void loadStatements();
    } catch (error) {
      composerError = humanizeError(error);
    } finally {
      submitting = false;
    }
  }

  function startEdit(s: AuthoredContext): void {
    editingId = s.id;
    editText = s.text;
    editTopic = s.topic;
  }

  async function saveEdit(id: number): Promise<void> {
    const text = editText.trim();
    if (!text || savingEdit) return;
    savingEdit = true;
    try {
      await invoke("user_context_update_authored", { id, text, topic: editTopic });
      statements = (statements ?? []).map((s) => (s.id === id ? { ...s, text, topic: editTopic, updatedAtMs: Date.now() } : s));
      editingId = null;
      void loadStatements();
    } catch (error) {
      await message(humanizeError(error), { title: "Couldn't save context", kind: "error" });
    } finally {
      savingEdit = false;
    }
  }

  async function deleteStatement(s: AuthoredContext): Promise<void> {
    const ok = await confirm("Delete this statement? Mnema stops using it to steer your dossier.", {
      title: "Delete context",
      kind: "warning",
    });
    if (!ok) return;
    try {
      await invoke("user_context_delete_authored", { id: s.id });
      statements = (statements ?? []).filter((x) => x.id !== s.id);
      if (editingId === s.id) editingId = null;
      void loadStatements();
    } catch (error) {
      await message(humanizeError(error), { title: "Couldn't delete context", kind: "error" });
    }
  }

  const onCmdEnter = (fn: () => void) => (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      fn();
    }
  };

  $effect(() => {
    void untrack(() => loadStatements());
    void untrack(() => loadInferred());
    void untrack(() => loadDismissed());
    void untrack(() => loadProvider());
    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadStatements();
      void loadInferred();
      void loadDismissed();
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  });
</script>

{#snippet topicChips(value: string | null, set: (t: string | null) => void)}
  <div class="c-topics" role="group" aria-label="Topic">
    {#each TOPICS as t (t.name)}
      <button
        type="button"
        class="mx-btn mx-btn--sm"
        aria-pressed={value === t.name}
        onclick={() => set(value === t.name ? null : t.name)}
      >
        <span class="in-sw" style="--c:{t.color}"></span>{t.name}
      </button>
    {/each}
  </div>
{/snippet}

<section class="ctx" aria-label="Context">
  <div class="in-read">
    <div class="mx-reveal" style="--i:1">
      <span class="mx-kicker">Context · in your own words</span>
      <h1 class="mx-display">
        {#if failed}
          What you've told Mnema. <em>It steers the dossier, and never fades.</em>
        {:else if n}
          {WORDS[n] ?? `${n} things`} you've told Mnema.
          <em>{unsent ? "They steer the dossier first — the newest ones are sent." : "They steer the dossier first, and they never fade."}</em>
        {:else}
          Nothing you've told Mnema yet. <em>What you write here steers the dossier, and never fades.</em>
        {/if}
      </h1>
      <p class="mx-prose">
        Everything else Mnema knows is inferred from activity and scored on confidence. What you write here
        is taken as given — and outranks an inference that disagrees.
      </p>
    </div>
    <div class="in-lede-stats mx-reveal" style="--i:2">
      {#each [["Authored", failed ? "—" : n, failed ? "couldn't load" : "never fades"], ["Inferred", inferred?.length ?? "—", "scored · can fade"], ["Dismissed", dismissed.length, "won't be re-inferred"], ["Guarded", GUARDED, "categories never inferred"]] as [label, value, note] (label)}
        <div class="mx-stat">
          <span class="mx-stat__label">{label}</span><span class="mx-stat__value">{value}</span><span class="mx-stat__delta">{note}</span>
        </div>
      {/each}
    </div>
  </div>

  <div class="in-ex">
    <div>
      <div class="mx-panel mx-panel--raised in-compose mx-reveal" style="--i:3">
        <span class="mx-kicker">Add context</span>
        <div class="in-compose__ta">
          <textarea
            bind:value={draftText}
            maxlength={MAX_CHARS}
            placeholder="e.g. “I'm a designer — Figma time is real work even when it looks like browsing.”"
            aria-label="Add context"
            disabled={submitting}
            onkeydown={onCmdEnter(() => void submitDraft())}
          ></textarea>
          <span class="num in-compose__count">{draftText.length} / {MAX_CHARS}</span>
        </div>
        <div class="in-compose__row">
          <span class="mx-label">Topic</span>
          {@render topicChips(draftTopic, (t) => (draftTopic = t))}
          <span class="mx-spacer"></span>
          <button
            type="button"
            class="mx-btn mx-btn--primary mx-btn--sm"
            disabled={submitting || !draftText.trim()}
            aria-busy={submitting}
            onclick={() => void submitDraft()}
          >
            {#if submitting}<span class="mx-spin mx-spin--sm"></span>Adding…{:else}<IconPlus width="13" height="13" aria-hidden="true" />Add context<kbd>⌘↵</kbd>{/if}
          </button>
        </div>
        {#if providerKind}
          <div class="in-compose__priv">
            {#if isCloudAiProviderKind(providerKind)}
              <IconCloud width="13" height="13" aria-hidden="true" />Goes to {aiProviderKindLabel(providerKind)} each time Mnema
              forms conclusions · stays on this Mac with a local model.
            {:else}
              <IconLaptop width="13" height="13" aria-hidden="true" />Stays on this Mac — {aiProviderKindLabel(providerKind)} forms
              conclusions locally.
            {/if}
          </div>
        {/if}
        {#if composerError}
          <div>
            <span class="mx-inline" data-tone="danger">
              <IconAlert width="13" height="13" aria-hidden="true" />Couldn't save that statement — {composerError}. Your text is still here.
              <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => void submitDraft()}>Retry</button>
            </span>
          </div>
        {/if}
      </div>

      <div class="mx-reveal" style="--i:4">
        <div class="in-sec">
          <h2 class="mx-h2">Standing context</h2>
          <span class="mx-label num">
            {#if failed}{:else if unsent}{n - unsent} of {n} statements used — older ones aren't sent{:else}{n} statement{n === 1 ? "" : "s"}{n ? " · all sent" : ""}{/if}
          </span>
        </div>
        {#if failed}
          <div class="mx-empty" data-tone="danger" role="alert">
            <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
            <h4 class="mx-empty__title">Couldn't load your context.</h4>
            <p class="mx-empty__text">Reading your statements failed. Nothing is lost.</p>
            <div class="mx-empty__acts">
              <button type="button" class="mx-btn mx-btn--sm" disabled={loading} onclick={() => void loadStatements()}>
                <IconRetry width="13" height="13" aria-hidden="true" />Try again
              </button>
            </div>
          </div>
        {:else if loading && !statements}
          <div aria-busy="true">
            <span class="mx-sr">Loading context</span>
            {#each Array.from({ length: 4 }) as _, i (i)}
              <div class="mx-skel-row"><div class="mx-skel"></div><div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div></div><div class="mx-skel"></div></div>
            {/each}
          </div>
        {:else if !n}
          <div class="mx-empty">
            <span class="mx-empty__glyph"><IconInbox width="18" height="18" aria-hidden="true" /></span>
            <h4 class="mx-empty__title">No standing context yet.</h4>
            <p class="mx-empty__text">
              Add a short statement above — your role, what you're working on, how you work, what you care about.
              It steers the dossier, and it never fades.
            </p>
          </div>
        {:else}
          {#each statements ?? [] as s (s.id)}
            {@const notSent = s.inPrompt === false}
            <div class="in-stmt" class:in-stmt--unsent={notSent && editingId !== s.id}>
              <span class="in-stmt__topic">
                {#if s.topic}<span class="in-sw" style="--c:{topicColor(s.topic)}"></span>{s.topic}{/if}
              </span>
              {#if editingId === s.id}
                <div class="in-stmt__edit">
                  <textarea
                    class="mx-input"
                    bind:value={editText}
                    maxlength={MAX_CHARS}
                    aria-label="Edit statement"
                    onkeydown={onCmdEnter(() => void saveEdit(s.id))}
                  ></textarea>
                  <div class="in-compose__row">
                    {@render topicChips(editTopic, (t) => (editTopic = t))}
                    <span class="mx-spacer"></span>
                    <span class="num in-stmt__count">{editText.length} / {MAX_CHARS}</span>
                    <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => (editingId = null)}>Cancel</button>
                    <button
                      type="button"
                      class="mx-btn mx-btn--primary mx-btn--sm"
                      disabled={!editText.trim() || savingEdit}
                      aria-busy={savingEdit}
                      onclick={() => void saveEdit(s.id)}
                    >
                      {#if savingEdit}<span class="mx-spin mx-spin--sm"></span>Saving…{:else}Save<kbd>⌘↵</kbd>{/if}
                    </button>
                  </div>
                </div>
                <span></span>
              {:else}
                <p>{s.text}<small>{metaTime(s)} · {#if notSent}<span class="num">not sent</span>{:else}never fades{/if}</small></p>
                <span class="in-stmt__acts">
                  <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Edit" onclick={() => startEdit(s)}>
                    <IconEdit width="13" height="13" aria-hidden="true" />
                  </button>
                  <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Delete" onclick={() => void deleteStatement(s)}>
                    <IconTrash width="13" height="13" aria-hidden="true" />
                  </button>
                </span>
              {/if}
            </div>
          {/each}
        {/if}
      </div>
    </div>

    <ContextSide
      authored={statements ?? []}
      {inferred}
      {dismissed}
      {dismissedError}
      {restoringKey}
      {openDismissed}
      onRestore={(d) => void restoreDismissed(d)}
      {onOpenSubject}
    />
  </div>
</section>

<style>
  /* insights.html `.in-read` lede + Context main column (`.in-compose`, `.in-stmt`). */
  .in-read {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: var(--s-7);
    align-items: end;
    padding: var(--s-4) 0 var(--s-6);
    margin-bottom: var(--s-6);
    border-bottom: 1px solid var(--mx-hairline);
  }
  .in-read .mx-display {
    margin: var(--s-4) 0;
    max-width: 24ch;
  }
  .in-read .mx-display em {
    font-style: normal;
    font-weight: 500;
    color: var(--app-text-subtle);
  }
  .in-lede-stats {
    display: grid;
    grid-template-columns: repeat(2, minmax(132px, auto));
    gap: var(--s-5) var(--s-6);
    padding-bottom: 4px;
  }
  .in-ex {
    display: grid;
    grid-template-columns: minmax(0, 1.45fr) minmax(0, 1fr);
    gap: var(--s-6);
    margin-bottom: var(--s-7);
  }
  .in-sec {
    display: flex;
    align-items: baseline;
    gap: var(--s-3);
    margin: 0 0 var(--s-3);
  }
  .in-sw {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
    background: var(--c);
    flex: none;
  }
  .in-compose {
    padding: var(--s-4) var(--s-4) var(--s-3);
    display: grid;
    gap: var(--s-3);
    margin-bottom: var(--s-6);
  }
  .in-compose__ta {
    position: relative;
    display: grid;
  }
  .in-compose textarea {
    resize: none;
    min-height: 76px;
    padding: 0 72px 0 0;
    border: 0;
    outline: 0;
    background: none;
    color: var(--app-text-strong);
    font: 400 var(--text-xl) / 1.4 var(--font-sans);
    letter-spacing: -0.01em;
  }
  .in-compose textarea::placeholder {
    color: var(--app-text-faint);
  }
  .in-compose__count {
    position: absolute;
    right: 2px;
    bottom: 2px;
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    pointer-events: none;
  }
  .in-compose__row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-top: var(--s-3);
    border-top: 1px solid var(--mx-hairline);
  }
  .in-compose__priv {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: calc(var(--s-3) * -1 + 4px);
    font: 400 var(--text-sm) / 1.4 var(--font-sans);
    color: var(--app-text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .in-compose__priv :global(svg) {
    flex: none;
    color: var(--app-text-subtle);
  }
  .in-compose__row > .mx-label {
    margin-right: 4px;
  }
  .c-topics {
    display: flex;
    gap: 4px;
  }
  .c-topics .in-sw {
    width: 7px;
    height: 7px;
  }
  .in-stmt {
    display: grid;
    grid-template-columns: 120px minmax(0, 1fr) auto;
    gap: var(--s-4);
    padding: 14px var(--s-3);
    margin: 0 calc(var(--s-3) * -1);
    border-top: 1px solid var(--mx-hairline);
    align-items: start;
    border-radius: var(--r-sm);
    transition: background-color var(--t-fast) var(--ease-quart);
  }
  .in-stmt:hover {
    background: color-mix(in srgb, var(--app-fg) 2.5%, transparent);
  }
  .in-stmt__topic {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font: 500 var(--text-base) / 1.7 var(--font-sans);
    color: var(--app-text-subtle);
  }
  .in-stmt p {
    margin: 0;
    color: var(--app-text-strong);
    font-size: var(--text-lg);
    line-height: 1.45;
    letter-spacing: -0.005em;
  }
  .in-stmt p small {
    display: block;
    margin-top: 6px;
    font: 400 var(--text-sm) / 1 var(--font-sans);
    color: var(--app-text-subtle);
    letter-spacing: 0;
  }
  .in-stmt__acts {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--t-fast);
  }
  .in-stmt:hover .in-stmt__acts,
  .in-stmt:focus-within .in-stmt__acts {
    opacity: 1;
  }
  .in-stmt--unsent {
    opacity: 0.38;
  }
  .in-stmt__edit {
    display: grid;
    gap: var(--s-3);
  }
  .in-stmt__edit textarea {
    height: auto;
    min-height: 64px;
    padding: var(--s-2) var(--s-3);
    resize: vertical;
    font: 400 var(--text-lg) / 1.45 var(--font-sans);
  }
  .in-stmt__edit .in-compose__row {
    border-top: 0;
    padding-top: 0;
  }
  .in-stmt__count {
    font-size: var(--text-xs);
    color: var(--app-text-subtle);
    margin-right: var(--s-2);
  }
  @media (max-width: 1180px) {
    .in-ex {
      grid-template-columns: 1fr;
    }
  }
</style>
