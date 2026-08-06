<script lang="ts">
  // Context — the user-AUTHORED context destination (page 10, issue #107).
  //
  // The user tells Mnema about themselves directly ("I'm a designer", "I care
  // about X"). Authored context COMPLEMENTS the inferred Conclusion dossier,
  // steering it up front rather than only correcting after the fact. Unlike an
  // inferred Conclusion it is NOT subject to Confidence / decay — the user
  // asserted it, so it never fades. It IS still subject to the Sensitive
  // Category Guardrail for what the engine surfaces.
  //
  // Layout (direction 03): composer at the top because it is the page's verb,
  // the standing list under it as ONE opaque plate, the dismissed archive
  // collapsed below, and a side column that looks like a rail but is plates —
  // it carries content, and content never lands on material in this direction.
  //
  // This file owns state and the backend; the four `context-*` children render.
  //
  // Backend (#107 commands already exist):
  //   list_user_context_authored     → AuthoredContext[] (newest-first)
  //   user_context_add_authored      { text, topic } → AuthoredContext
  //   user_context_update_authored   { id, text, topic } → void
  //   user_context_delete_authored   { id } → void
  //   user_context_list_dismissed / user_context_restore_dismissed
  // Refresh on the `user_context_changed` event.

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { toast } from "$lib/toast.svelte";
  import type {
    Conclusion,
    UserContextStatus,
    DerivationBudgetTier,
    AuthoredContext,
    DismissedView,
  } from "$lib/types/recording";
  import { humanizeError } from "$lib/format-error";
  import ContextComposer from "$lib/insights/context-composer.svelte";
  import ContextList from "$lib/insights/context-list.svelte";
  import ContextDismissed from "$lib/insights/context-dismissed.svelte";
  import ContextSide from "$lib/insights/context-side.svelte";

  // ── Authored statement list ──────────────────────────────────────────
  let statements = $state<AuthoredContext[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);

  // ── Composer ─────────────────────────────────────────────────────────
  let submitting = $state(false);
  let composerError = $state<string | null>(null);

  // ── Inline edit ──────────────────────────────────────────────────────
  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let savingEdit = $state(false);

  // ── Dismissed archive (the negative space: "what you're NOT") ─────────
  // Beliefs the user rejected from the inferred dossier. Restoring lifts the
  // suppression veto — it does NOT resurrect the old conclusion; the belief can
  // only re-form on the next derivation pass IF the user's activity still
  // supports it (the honest copy the archive carries).
  let dismissed = $state<DismissedView[] | null>(null);
  let dismissedError = $state<string | null>(null);
  let showDismissed = $state(false);
  const dismissedCount = $derived(dismissed?.length ?? 0);

  // ── Engine tier badge + the conclusions the context steers ───────────
  let budgetTier = $state<DerivationBudgetTier | null>(null);
  let conclusions = $state<Conclusion[] | null>(null);

  // The top three VISIBLE conclusions. Display-only: nothing in the store
  // records "this authored statement produced that conclusion", so the card
  // states what is true (these are your dossier's strongest beliefs, and
  // authored context steers derivation) and claims no per-row causal link.
  const steering = $derived.by(() =>
    [...(conclusions ?? [])]
      .filter((c) => c.status === "visible")
      .sort((a, b) => b.confidence - a.confidence)
      .slice(0, 3),
  );

  const tierLabel = $derived(
    budgetTier ? budgetTier.charAt(0).toUpperCase() + budgetTier.slice(1) : "Engine",
  );

  async function loadStatements(): Promise<void> {
    loading = true;
    try {
      statements = await invoke<AuthoredContext[]>("list_user_context_authored");
      loadError = null;
    } catch (error) {
      loadError = humanizeError(error);
    } finally {
      loading = false;
    }
  }

  // Best-effort side-column context: the engine tier badge + the conclusions.
  // A failure just leaves the column quiet.
  async function loadSideContext(): Promise<void> {
    const [status, list] = await Promise.all([
      invoke<UserContextStatus>("get_user_context_status").catch(() => null),
      invoke<Conclusion[]>("list_user_context_conclusions", { includeFaded: false }).catch(
        () => null,
      ),
    ]);
    if (status) budgetTier = status.budgetTier;
    if (list) conclusions = list;
  }

  async function loadDismissed(): Promise<void> {
    try {
      dismissed = await invoke<DismissedView[]>("user_context_list_dismissed");
      dismissedError = null;
    } catch (error) {
      dismissedError = humanizeError(error);
      dismissed = dismissed ?? [];
    }
  }

  async function restoreDismissed(d: DismissedView): Promise<void> {
    try {
      await invoke<void>("user_context_restore_dismissed", {
        subject: d.subject,
        statement: d.statement,
      });
      // Optimistically drop it from the archive; the `user_context_changed`
      // event the command emits also re-lists, keeping every surface in sync.
      dismissed = (dismissed ?? []).filter(
        (x) => x.subject !== d.subject || x.statement !== d.statement,
      );
      dismissedError = null;
    } catch (error) {
      dismissedError = humanizeError(error);
    }
  }

  async function addStatement(text: string, topic: string | null): Promise<boolean> {
    submitting = true;
    composerError = null;
    try {
      const created = await invoke<AuthoredContext>("user_context_add_authored", {
        text,
        topic,
      });
      // Append optimistically (newest-first) — no re-list needed.
      statements = [created, ...(statements ?? [])];
      return true;
    } catch (error) {
      composerError = humanizeError(error);
      return false;
    } finally {
      submitting = false;
    }
  }

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
    if (text.length === 0 || savingEdit) return;
    savingEdit = true;
    const topic = editTopic.trim();
    const nextTopic = topic.length > 0 ? topic : null;
    try {
      await invoke("user_context_update_authored", { id, text, topic: nextTopic });
      // Reflect locally; createdAt stays, updatedAt advances.
      statements = (statements ?? []).map((s) =>
        s.id === id ? { ...s, text, topic: nextTopic, updatedAtMs: Date.now() } : s,
      );
      cancelEdit();
    } catch (error) {
      // The list-load error surface only renders when there are no statements,
      // so it's unreachable here (we're editing an existing one). Raise a toast
      // instead of silently swallowing the failure.
      toast({
        tone: "error",
        title: "Couldn't save context",
        message: humanizeError(error),
      });
    } finally {
      savingEdit = false;
    }
  }

  async function deleteStatement(s: AuthoredContext): Promise<void> {
    const ok = await confirm(
      "Delete this context statement? Mnema will no longer use it to steer your dossier.",
      { title: "Delete context", kind: "warning" },
    );
    if (!ok) return;
    try {
      await invoke("user_context_delete_authored", { id: s.id });
      statements = (statements ?? []).filter((x) => x.id !== s.id);
      if (editingId === s.id) cancelEdit();
    } catch (error) {
      // Same unreachable-surface problem as saveEdit — raise a toast.
      toast({
        tone: "error",
        title: "Couldn't delete context",
        message: humanizeError(error),
      });
    }
  }

  $effect(() => {
    void untrack(() => loadStatements());
    void untrack(() => loadSideContext());
    void untrack(() => loadDismissed());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadStatements();
      void loadSideContext();
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

<section class="ctx" aria-label="Context">
  <header class="chead">
    <h1>Context</h1>
    <p>
      What you tell Mnema about yourself. It steers your dossier and
      <b>never fades</b> like an inferred conclusion. The Sensitive Category Guardrail keeps
      off-limits categories from being surfaced.
    </p>
  </header>

  <div class="cgrid">
    <div class="cmain">
      <ContextComposer
        tier={tierLabel}
        {submitting}
        error={composerError}
        onadd={addStatement}
      />

      <div class="head">
        <span class="t-label">Standing context</span>
        <span class="count is-num">{statements?.length ?? 0}</span>
      </div>

      <ContextList
        {statements}
        {loading}
        {loadError}
        {editingId}
        bind:editText
        bind:editTopic
        {savingEdit}
        onretry={() => void loadStatements()}
        onedit={startEdit}
        ondelete={(s) => void deleteStatement(s)}
        onsave={(id) => void saveEdit(id)}
        oncancel={cancelEdit}
      />

      {#if dismissedCount > 0 || dismissedError}
        <div class="head head--dismissed">
          <span class="t-label">Dismissed</span>
          <span class="count is-num">{dismissedCount}</span>
          <button
            type="button"
            class="btn btn--ghost btn--sm toggle"
            aria-expanded={showDismissed}
            onclick={() => (showDismissed = !showDismissed)}
          >
            {showDismissed ? "Hide" : "Show"}
          </button>
        </div>
        {#if showDismissed}
          <ContextDismissed
            rows={dismissed}
            error={dismissedError}
            onrestore={restoreDismissed}
          />
        {/if}
      {/if}

      <!-- The deletion sentence — exactly as true as the backend is (ADR 0029).
           Retention Cleanup keeps the dossier; Delete Recent Capture cascades
           into it; Wipe User Context (`wipe_all` + the conversation store)
           clears everything INCLUDING authored statements, and disables the
           engine first. Nothing here promises more than the store implements. -->
      <div class="plate truth">
        <span class="s" aria-hidden="true">
          <svg viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="8.7" /><path d="M12 16.5v-5M12 8h.01" />
          </svg>
        </span>
        <div class="tbody">
          <span class="ttl">Your context outlives your captures</span>
          <p>
            When screen and audio age out under your retention window, what Mnema learned
            from them stays — the summaries become the evidence. <b>Delete Recent Capture</b>
            is different: it erases what was derived from that window too, and drops any
            conclusion left without enough evidence to stand. To erase the understanding
            itself, use <b>Wipe User Context</b> in Settings — it clears every activity,
            conclusion, dismissal and saved chat, and turns the engine off first.
          </p>
          <div class="tchips">
            <span class="chip">Retention Cleanup — captures only</span>
            <span class="chip">
              Delete Recent Capture — captures <i>and</i> what was derived from them
            </span>
            <span class="chip">Wipe User Context — the whole dossier, engine off</span>
          </div>
          <p class="fine">
            Authored statements on this page survive both capture-side operations; only
            Wipe removes them.
          </p>
        </div>
      </div>
    </div>

    <ContextSide {steering} />
  </div>
</section>

<style>
  .ctx {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .chead h1 {
    margin: 0 0 4px;
    font: var(--w-semi) var(--t-display) / var(--lh-display) var(--app-font-sans);
    letter-spacing: var(--ls-display);
    color: var(--app-text-strong);
  }
  .chead p {
    margin: 0;
    max-width: 80ch;
    font: var(--w-regular) var(--t-meta) / 1.5 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .chead p b {
    color: var(--app-accent);
    font-weight: var(--w-medium);
  }

  .cgrid {
    display: grid;
    grid-template-columns: 2fr 1fr;
    gap: 16px;
    align-items: start;
  }
  @media (max-width: 940px) {
    .cgrid {
      grid-template-columns: 1fr;
    }
  }
  .cmain {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }

  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 4px;
    margin-bottom: -4px;
  }
  .head--dismissed {
    margin-top: 4px;
  }
  .head .toggle {
    margin-left: auto;
  }
  .count {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 8px;
    border-radius: var(--r-pill);
    background: var(--glass-tint);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }

  /* the retention truth — the one callout that must not overclaim */
  .truth {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 13px 15px;
    border-radius: var(--r-panel);
    margin-top: 4px;
  }
  .truth .s {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    border-radius: 7px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--app-info-bg);
    color: var(--app-info);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-info-border);
  }
  .truth .s svg {
    width: 13px;
    height: 13px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.7;
    stroke-linecap: round;
  }
  .tbody {
    flex: 1;
    min-width: 0;
  }
  .truth .ttl {
    font: var(--w-semi) var(--t-ui) / var(--lh-ui) var(--app-font-sans);
    color: var(--app-text-strong);
  }
  .truth p {
    margin: 4px 0 0;
    max-width: 78ch;
    font: var(--w-regular) var(--t-read) / 1.55 var(--app-font-sans);
    color: var(--app-text-muted);
  }
  .truth p b {
    color: var(--app-text-strong);
    font-weight: var(--w-medium);
  }
  .truth .fine {
    margin-top: 9px;
    font-size: var(--t-meta);
    color: var(--app-text-subtle);
  }
  .tchips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 9px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--r-pill);
    background: var(--glass-tint);
    color: var(--app-text-muted);
    font: var(--w-medium) var(--t-meta) / 1 var(--app-font-sans);
    box-shadow: inset 0 0 0 var(--hairline) var(--glass-line);
  }
  .chip i {
    font-style: italic;
    margin: 0 3px;
  }
</style>
