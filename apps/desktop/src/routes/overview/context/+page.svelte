<script lang="ts">
  // ══ CONTEXT — a DESTINATION inside Overview (`/overview/context`) ══════════
  //
  // Direction 05 "Tactile Instruments". The one page in this direction with NO
  // instrument on it, deliberately: nothing here has a physical quantity. A
  // sentence you wrote about yourself costs no disk, does not decay and cannot
  // be turned up — so Context is System-Settings bones the whole way down (a
  // composer, a list, an archive, a rail and one plain ledger). A gauge here
  // would be claiming a cost that does not exist.
  //
  // Four verbs are deliberately absent, because no backend command backs them:
  // edit an inferred conclusion, forget a whole subject, pause derivation from
  // here, and tell a user-rejected dismissal apart from an engine-retired one.
  //
  // Reads (all shipping commands, reused from `lib/insights/Context.svelte`):
  //   list_user_context_authored   → AuthoredContext[] (newest-first)
  //   user_context_add_authored     { text, topic } → AuthoredContext
  //   user_context_update_authored  { id, text, topic }
  //   user_context_delete_authored  { id }
  //   user_context_list_dismissed   → DismissedView[]
  //   user_context_restore_dismissed { subject, statement }
  //   get_user_context_status       → the engine tier chip
  //   list_user_context_conclusions → the steering rail
  // Everything re-reads on the `user_context_changed` event.
  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { toast } from "$lib/toast.svelte";
  import type {
    AuthoredContext,
    Conclusion,
    DismissedView,
  } from "$lib/types/recording";
  import type { DerivationBudgetTier, UserContextStatus } from "$lib/types";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import ContextFields from "$lib/overview/context/ContextFields.svelte";
  import ContextRail from "$lib/overview/context/ContextRail.svelte";
  import EraseLedger from "$lib/overview/context/EraseLedger.svelte";
  import { humanizeError } from "$lib/format-error";

  const SKELETON_COUNT = 3;
  const STEER_ROWS = 3;

  // The five Try chips are PROMPTS, not categories: the topic field is free
  // text with no enumeration behind it, so each chip only prefills the
  // composer's opening words.
  const SUGGESTIONS: { label: string; prompt: string }[] = [
    { label: "Your role", prompt: "I'm a … " },
    { label: "What you're working on", prompt: "I'm currently working on … " },
    { label: "How you work", prompt: "I prefer to work by … " },
    { label: "What you care about", prompt: "I care deeply about … " },
    { label: "Goals this quarter", prompt: "Goal: " },
  ];

  // ── Authored statements ──────────────────────────────────────────────────
  let statements = $state<AuthoredContext[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);

  // ── Composer ─────────────────────────────────────────────────────────────
  let draftText = $state("");
  let draftTopic = $state("");
  let submitting = $state(false);
  let composerError = $state<string | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  const canSubmit = $derived(draftText.trim().length > 0 && !submitting);

  // ── Inline edit ──────────────────────────────────────────────────────────
  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let savingEdit = $state(false);

  // ── Dismissed archive ────────────────────────────────────────────────────
  // Beliefs the engine formed and you removed. Restoring lifts the veto; it
  // does NOT put the belief back — it can only re-form if your activity still
  // supports it, and the copy says exactly that.
  let dismissed = $state<DismissedView[] | null>(null);
  let dismissedError = $state<string | null>(null);
  let showDismissed = $state(true);
  let restoringKey = $state<string | null>(null);

  const dismissedCount = $derived(dismissed?.length ?? 0);

  function dismissedKey(d: DismissedView): string {
    return `${d.subject}\0${d.statement}`;
  }

  // ── The rail ─────────────────────────────────────────────────────────────
  let budgetTier = $state<DerivationBudgetTier | null>(null);
  let conclusions = $state<Conclusion[] | null>(null);

  // The strongest visible conclusion per subject.
  const subjects = $derived.by(() => {
    const best = new Map<string, Conclusion>();
    for (const c of conclusions ?? []) {
      if (c.status !== "visible") continue;
      const held = best.get(c.subject);
      if (!held || c.confidence > held.confidence) best.set(c.subject, c);
    }
    return [...best.values()].sort(
      (a, b) => b.confidence - a.confidence || a.subject.localeCompare(b.subject),
    );
  });

  // Steering rows are DERIVED, never invented. A row claims a link only when
  // one is defensible from the data we actually have: the authored statement
  // names the subject (its topic is the subject, or the subject's name appears
  // in what you wrote). There is no stored authored→conclusion edge in the
  // backend, so anything looser would be a fabricated mapping. When nothing
  // matches, the rail falls back to the subjects your dossier holds and the
  // copy stops claiming your statements steered them.
  const steering = $derived.by(() => {
    const written = statements ?? [];
    const rows: { subject: string; confidence: number; via: string }[] = [];
    for (const s of subjects) {
      const needle = s.subject.trim().toLowerCase();
      if (needle.length < 4) continue;
      const match = written.find((w) => {
        const topic = (w.topic ?? "").trim().toLowerCase();
        return (
          topic === needle ||
          topic.includes(needle) ||
          needle.includes(topic.length >= 4 ? topic : "\0") ||
          w.text.toLowerCase().includes(needle)
        );
      });
      if (match) rows.push({ subject: s.subject, confidence: s.confidence, via: match.text });
      if (rows.length === STEER_ROWS) break;
    }
    return rows;
  });

  const fallbackSubjects = $derived(subjects.slice(0, STEER_ROWS));

  function tierLabel(tier: DerivationBudgetTier | null): string {
    if (!tier) return "engine";
    return tier.charAt(0).toUpperCase() + tier.slice(1);
  }

  function relativeTime(ms: number): string {
    if (!Number.isFinite(ms) || ms <= 0) return "—";
    const diff = Date.now() - ms;
    if (diff < 0) return "just now";
    const min = Math.floor(diff / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min}m ago`;
    const hr = Math.floor(min / 60);
    if (hr < 24) return `${hr}h ago`;
    const day = Math.floor(hr / 24);
    if (day < 7) return `${day}d ago`;
    const wk = Math.floor(day / 7);
    if (wk < 5) return `${wk}w ago`;
    const mo = Math.floor(day / 30);
    if (mo < 12) return `${mo}mo ago`;
    return `${Math.floor(day / 365)}y ago`;
  }

  function metaTime(s: AuthoredContext): string {
    const edited = s.updatedAtMs > s.createdAtMs + 1000;
    return edited
      ? `edited ${relativeTime(s.updatedAtMs)}`
      : `added ${relativeTime(s.createdAtMs)}`;
  }

  async function loadStatements(): Promise<void> {
    loading = true;
    try {
      statements = await invoke<AuthoredContext[]>("list_user_context_authored");
      loadError = null;
    } catch (error) {
      loadError = humanizeError(error);
      statements = statements ?? null;
    } finally {
      loading = false;
    }
  }

  // Best-effort: a failed rail read leaves the rail quiet rather than raising a
  // banner over a page whose main column loaded fine.
  async function loadRail(): Promise<void> {
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
    const key = dismissedKey(d);
    if (restoringKey === key) return;
    restoringKey = key;
    try {
      await invoke<void>("user_context_restore_dismissed", {
        subject: d.subject,
        statement: d.statement,
      });
      // Optimistic; the command also emits `user_context_changed`, which
      // re-lists every surface.
      dismissed = (dismissed ?? []).filter((x) => dismissedKey(x) !== key);
      dismissedError = null;
    } catch (error) {
      dismissedError = humanizeError(error);
    } finally {
      restoringKey = null;
    }
  }

  function applySuggestion(prompt: string): void {
    draftText = draftText.trim().length === 0 ? prompt : `${draftText} ${prompt}`;
    composerEl?.focus();
  }

  async function submitDraft(): Promise<void> {
    const text = draftText.trim();
    if (text.length === 0 || submitting) return;
    submitting = true;
    composerError = null;
    const topic = draftTopic.trim();
    try {
      const created = await invoke<AuthoredContext>("user_context_add_authored", {
        text,
        topic: topic.length > 0 ? topic : null,
      });
      statements = [created, ...(statements ?? [])];
      draftText = "";
      draftTopic = "";
    } catch (error) {
      composerError = humanizeError(error);
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
      statements = (statements ?? []).map((s) =>
        s.id === id ? { ...s, text, topic: nextTopic, updatedAtMs: Date.now() } : s,
      );
      cancelEdit();
    } catch (error) {
      // The list-level error card only renders on an empty list, so it is
      // unreachable here — raise a toast instead of swallowing the failure.
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
    // The NATIVE confirm (`@tauri-apps/plugin-dialog`), never drawn chrome.
    // This is a hard row delete with no tombstone and no undo, so the dialog is
    // the whole safety net.
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
      toast({
        tone: "error",
        title: "Couldn't delete context",
        message: humanizeError(error),
      });
    }
  }

  $effect(() => {
    void untrack(() => loadStatements());
    void untrack(() => loadRail());
    void untrack(() => loadDismissed());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadStatements();
      void loadRail();
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

<div class="dest">
  <header class="dest__bar">
    <p class="t-title">Context</p>
    <p class="t-meta dest__lede">
      What you tell Mnema about yourself. It steers your dossier and
      <span class="dest__acc">never fades</span> like an inferred conclusion. The
      Sensitive Category Guardrail keeps off-limits categories from being surfaced.
    </p>
  </header>

  <div class="dest__body">
    <!-- ── MAIN COLUMN — the only thing that scrolls ─────────────────────── -->
    <div class="dest__main">
      <!-- COMPOSER: two stock fields and a button. No instrument. -->
      <div class="comp">
        <div class="comp__h">
          <span class="t-label">Add context</span>
          {#if budgetTier}
            <span class="ti-chip"><i class="comp__dot"></i>{tierLabel(budgetTier)}</span>
          {/if}
          <span class="ti-chip ti-chip--acc comp__pen">✎ authored</span>
        </div>

        <ContextFields
          bind:text={draftText}
          bind:topic={draftTopic}
          bind:el={composerEl}
          placeholder="I'm a… I care about… I work best with…"
          topicPlaceholder="topic (optional, e.g. role, focus, goal)"
          label="Add a context statement"
          topicLabel="Topic for this statement (optional)"
          onsubmit={() => void submitDraft()}
        >
          {#snippet actions()}
            <button
              type="button"
              class="btn btn--accent btn--sm"
              disabled={!canSubmit}
              onclick={() => void submitDraft()}
            >
              {submitting ? "Adding…" : "＋ Add"}
            </button>
          {/snippet}
        </ContextFields>

        <div class="try">
          <span class="t-label">Try</span>
          {#each SUGGESTIONS as s (s.label)}
            <button type="button" class="ti-chip try__c" onclick={() => applySuggestion(s.prompt)}>
              {s.label}
            </button>
          {/each}
        </div>

        <p class="t-meta comp__note">
          <span class="comp__glyph" aria-hidden="true">›</span>Authored statements
          never fade from your dossier.
        </p>

        {#if composerError}<p class="err">{composerError}</p>{/if}
      </div>

      <!-- STANDING CONTEXT -->
      <div class="sechd">
        <span class="t-label">Standing context</span>
        {#if statements}<span class="cnt is-num">{statements.length}</span>{/if}
      </div>

      {#if loadError && !statements}
        <div class="estate estate--err">
          <span class="t-ui strong">Couldn't load your context.</span>
          <span class="t-meta">{loadError}</span>
          <button
            type="button"
            class="btn btn--sm estate__retry"
            disabled={loading}
            onclick={() => void loadStatements()}>↻ Try again</button
          >
        </div>
      {:else if loading && !statements}
        <div class="ti-grp" aria-label="Loading context" aria-busy="true">
          {#each Array.from({ length: SKELETON_COUNT }) as _, i (i)}
            <div class="st st--skel">
              <span class="st__txt">
                <Skeleton variant="text" width="82%" height="12px" />
                <span class="st__m">
                  <Skeleton variant="text" width="60px" height="11px" radius="4px" />
                  <Skeleton variant="text" width="72px" height="11px" radius="999px" />
                  <Skeleton variant="text" width="64px" height="11px" />
                </span>
              </span>
            </div>
          {/each}
        </div>
      {:else if (statements?.length ?? 0) === 0}
        <div class="estate">
          <span class="t-ui strong">No standing context yet.</span>
          <span class="t-meta">
            Add a short statement above — your role, what you're working on, how you
            work, what you care about. Mnema uses it to steer your dossier, and it
            never fades.
          </span>
        </div>
      {:else}
        <div class="ti-grp">
          {#each statements ?? [] as s (s.id)}
            {#if editingId === s.id}
              <div class="st st--edit">
                <span class="st__txt">
                  <ContextFields
                    bind:text={editText}
                    bind:topic={editTopic}
                    label="Edit context statement"
                    topicLabel="Edit topic (optional)"
                    onsubmit={() => void saveEdit(s.id)}
                  >
                    {#snippet actions()}
                      <button
                        type="button"
                        class="btn btn--accent btn--sm"
                        disabled={editText.trim().length === 0 || savingEdit}
                        onclick={() => void saveEdit(s.id)}
                      >
                        {savingEdit ? "Saving…" : "Save"}
                      </button>
                      <button type="button" class="btn btn--ghost btn--sm" onclick={cancelEdit}>
                        Cancel
                      </button>
                      <span class="ti-chip ti-chip--acc">✎ editing</span>
                    {/snippet}
                  </ContextFields>
                </span>
              </div>
            {:else}
              <div class="st">
                <span class="st__txt">
                  <span class="st__t">{s.text}</span>
                  <span class="st__m">
                    {#if s.topic}<span class="topic">{s.topic}</span>{/if}
                    <span class="ti-chip ti-chip--acc">✎ Authored</span>
                    <span class="t-meta">{metaTime(s)}</span>
                  </span>
                </span>
                <!-- Edit and Delete are the only two verbs this store has, so
                     they live on hover rather than taking permanent room. -->
                <span class="st__acts">
                  <button type="button" class="btn btn--ghost btn--sm" onclick={() => startEdit(s)}>
                    Edit
                  </button>
                  <button
                    type="button"
                    class="btn btn--ghost btn--sm st__del"
                    onclick={() => void deleteStatement(s)}
                  >
                    Delete
                  </button>
                </span>
              </div>
            {/if}
          {/each}
        </div>
      {/if}

      <!-- DISMISSED ARCHIVE — at zero it is not drawn at all. -->
      {#if dismissedCount > 0 || dismissedError}
        <div class="sechd">
          <span class="t-label">Dismissed</span>
          <span class="cnt is-num">{dismissedCount}</span>
          <button
            type="button"
            class="btn btn--ghost btn--sm sechd__act"
            aria-expanded={showDismissed}
            onclick={() => (showDismissed = !showDismissed)}
          >
            {showDismissed ? "Hide" : "Show"}
          </button>
        </div>
        {#if showDismissed}
          <p class="t-meta sechd__note">
            Beliefs you removed from your dossier. Restoring lets one form again —
            only if your activity still supports it.
          </p>
          {#if dismissedError}<p class="err">{dismissedError}</p>{/if}
          <div class="ti-grp">
            {#each dismissed ?? [] as d (dismissedKey(d))}
              <div class="st st--dis">
                <span class="st__txt">
                  <span class="st__t">{d.statement}</span>
                  <span class="st__m">
                    <span class="topic">{d.subject}</span>
                    <span class="t-meta">dismissed {relativeTime(d.dismissedAtMs)}</span>
                  </span>
                </span>
                <button
                  type="button"
                  class="btn btn--ghost btn--sm"
                  disabled={restoringKey === dismissedKey(d)}
                  onclick={() => void restoreDismissed(d)}
                >
                  {restoringKey === dismissedKey(d) ? "Restoring…" : "Restore"}
                </button>
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      <!-- THE ERASE LEDGER — a table, deliberately not a gauge. -->
      <div class="sechd">
        <span class="t-label">What can erase this</span>
      </div>
      <p class="t-meta sechd__note">
        Every delete control does something different to this page, and none of the
        obvious ones clear it.
      </p>
      <div class="ti-grp ti-grp--flush">
        <EraseLedger />
      </div>
    </div>

    <!-- The rail is a print, not a panel of controls, and it does not scroll
         with the list. Steering rows are derived honestly or not drawn. -->
    <ContextRail {steering} fallback={fallbackSubjects} />
  </div>
</div>

<style>
  .dest {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* Taller than the Journal/Subjects bar: this destination leads with a
     paragraph rather than a one-line subtitle. */
  .dest__bar {
    flex: 0 0 auto;
    padding: var(--s-12) var(--s-16);
    box-shadow: inset 0 -1px 0 var(--app-border);
  }
  .dest__bar p {
    margin: 0;
  }
  .dest__lede {
    margin-top: 3px;
    max-width: 78ch;
  }
  .dest__acc {
    color: var(--app-accent);
    font-weight: 510;
  }
  /* The rail is a print, not a panel of controls: only the main column
     scrolls, so the rail stays put as the list runs past the fold. */
  .dest__body {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    gap: var(--s-20);
    padding: var(--s-16) var(--s-20) 0;
    overflow: hidden;
  }
  .dest__main {
    flex: 1 1 auto;
    min-width: 0;
    overflow-y: auto;
    padding-bottom: var(--s-24);
  }

  /* ── section headers ─────────────────────────────────────────────────── */
  .sechd {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    padding: var(--s-16) var(--s-2) var(--s-6);
  }
  .sechd__act {
    margin-left: auto;
  }
  .sechd__note {
    margin: 0 var(--s-2) var(--s-8);
    max-width: 84ch;
  }
  .cnt {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 18px;
    height: 17px;
    padding: 0 5px;
    border-radius: var(--r-pill);
    background: var(--app-surface-hover);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-muted);
  }

  /* ── composer ────────────────────────────────────────────────────────── */
  .comp {
    background: var(--ti-grp-fill);
    border-radius: var(--r-lg);
    padding: var(--s-12);
    /* The one accent edge on the page: this is where you write, and writing is
       the only thing here that changes the dossier. */
    border-left: 3px solid var(--app-accent);
  }
  .comp__h {
    display: flex;
    align-items: center;
    gap: var(--s-8);
    margin-bottom: var(--s-8);
  }
  .comp__dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
  }
  .comp__pen {
    margin-left: auto;
  }
  .comp__note {
    margin: var(--s-8) 0 0;
    color: var(--app-text-subtle);
  }
  .comp__glyph {
    color: var(--app-text-faint);
    margin-right: 4px;
  }
  .try {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-6);
    margin-top: var(--s-8);
  }
  .try__c {
    cursor: default;
  }
  .try__c:hover {
    background: var(--app-accent-bg);
    color: var(--app-accent);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-accent-border);
  }
  .err {
    margin: var(--s-8) 0 0;
    font: var(--w-regular) var(--t-meta) / 1.45 var(--app-font-sans);
    color: var(--app-danger);
  }

  /* ── one statement row ───────────────────────────────────────────────── */
  .st {
    display: flex;
    align-items: flex-start;
    gap: var(--s-12);
    padding: var(--s-8) var(--s-12);
    position: relative;
  }
  .st + .st::before {
    content: "";
    position: absolute;
    left: var(--s-12);
    right: 0;
    top: 0;
    height: var(--hairline);
    background: var(--app-border);
  }
  .st:hover {
    background: var(--app-surface-hover);
  }
  .st--dis {
    opacity: 0.82;
  }
  .st--edit {
    background: color-mix(in srgb, var(--app-accent) 6%, transparent);
  }
  .st__txt {
    flex: 1 1 auto;
    min-width: 0;
  }
  .st__t {
    display: block;
    font: var(--w-medium) var(--t-ui) / 1.35 var(--app-font-sans);
    letter-spacing: var(--ls-ui);
    color: var(--app-text-strong);
  }
  .st__m {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-6);
    margin-top: 4px;
  }
  .st__acts {
    flex: 0 0 auto;
    display: flex;
    gap: var(--s-4);
    opacity: 0;
  }
  .st:hover .st__acts,
  .st:focus-within .st__acts {
    opacity: 1;
  }
  .st__del:hover {
    color: var(--app-danger);
    background: var(--app-danger-bg);
  }
  .topic {
    display: inline-flex;
    align-items: center;
    height: 18px;
    padding: 0 7px;
    border-radius: var(--r-sm);
    background: var(--app-surface-subtle);
    box-shadow: inset 0 0 0 var(--hairline) var(--app-border);
    font: var(--w-medium) var(--t-label) / 1 var(--app-font-mono);
    color: var(--app-text-muted);
  }
  .topic::before {
    content: "[";
    opacity: 0.5;
    margin-right: 1px;
  }
  .topic::after {
    content: "]";
    opacity: 0.5;
    margin-left: 1px;
  }

  /* ── states ──────────────────────────────────────────────────────────── */
  .estate {
    padding: var(--s-16);
    border-radius: var(--r-lg);
    background: var(--ti-grp-fill);
    display: flex;
    flex-direction: column;
    gap: var(--s-6);
    align-items: flex-start;
  }
  .estate--err {
    background: var(--app-danger-bg);
  }
  .estate__retry {
    margin-top: var(--s-4);
  }

  /* ── the rail ────────────────────────────────────────────────────────── */

  /* the ledger sits flush inside its group fill */
  .ti-grp--flush {
    padding: 0;
    overflow: hidden;
  }
</style>
