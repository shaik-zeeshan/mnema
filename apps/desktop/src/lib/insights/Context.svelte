<script lang="ts">
  // Context — the user-AUTHORED Context sub-surface (issue #107).
  //
  // The user tells Mnema about themselves directly ("I'm a designer", "I care
  // about X"). Authored context COMPLEMENTS the inferred Conclusion dossier,
  // steering it up front rather than only correcting after the fact. Unlike an
  // inferred Conclusion, authored context is NOT subject to Confidence / decay —
  // the user asserted it, so it never fades. It IS still subject to the
  // Sensitive Category Guardrail for what the engine surfaces.
  //
  // Mirrors `docs/user-context/mockups/context.html`: a two-pane grid with a
  // MAIN column (composer + authored-statement list) beside a sticky SIDE rail
  // (authored-vs-inferred, steering links into a few inferred Conclusions, and
  // the guardrail print). No props — the parent route renders <Context /> with
  // no navigation hook exposed, so steering chips are display-only.
  //
  // Backend (#107 commands already exist):
  //   list_user_context_authored    → AuthoredContext[] (newest-first)
  //   user_context_add_authored      { text, topic } → AuthoredContext
  //   user_context_update_authored   { id, text, topic } → void
  //   user_context_delete_authored   { id } → void
  // Refresh on the `user_context_changed` event.

  import { untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm, message } from "@tauri-apps/plugin-dialog";
  import type {
    Conclusion,
    UserContextStatus,
    DerivationBudgetTier,
    AuthoredContext,
  } from "$lib/types/recording";
  import Skeleton from "$lib/insights/Skeleton.svelte";

  // Placeholder statement rows shown while authored context loads.
  const SKELETON_COUNT = 3;

  // Static suggestion chips that prefill the composer. Mirrors the mockup's
  // cosmetic chip→prompt map.
  const SUGGESTIONS: { label: string; prompt: string }[] = [
    { label: "Your role", prompt: "I'm a … " },
    { label: "What you're working on", prompt: "I'm currently working on … " },
    { label: "How you work", prompt: "I prefer to work by … " },
    { label: "What you care about", prompt: "I care deeply about … " },
    { label: "Goals this quarter", prompt: "Goal: " },
  ];

  // ── Authored statement list ──────────────────────────────────────────
  let statements = $state<AuthoredContext[] | null>(null);
  let loadError = $state<string | null>(null);
  let loading = $state(true);

  // ── Composer ─────────────────────────────────────────────────────────
  let draftText = $state("");
  let draftTopic = $state("");
  let submitting = $state(false);
  let composerError = $state<string | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);

  const canSubmit = $derived(draftText.trim().length > 0 && !submitting);

  // ── Inline edit ──────────────────────────────────────────────────────
  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let savingEdit = $state(false);

  // ── Engine tier badge + steering links ───────────────────────────────
  let budgetTier = $state<DerivationBudgetTier | null>(null);
  let conclusions = $state<Conclusion[] | null>(null);

  // A few inferred Conclusions the authored context "steers". Display-only:
  // there is no cross-surface navigation hook exposed to <Context />.
  const steerLinks = $derived.by(() => {
    if (!conclusions) return [];
    return [...conclusions]
      .filter((c) => c.status === "visible")
      .sort((a, b) => b.confidence - a.confidence)
      .slice(0, 3);
  });

  const countLabel = $derived(statements?.length ?? 0);

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
    const yr = Math.floor(day / 365);
    return `${yr}y ago`;
  }

  // edited if updated meaningfully after creation; show "edited"/"added".
  function metaTime(s: AuthoredContext): string {
    const edited = s.updatedAtMs > s.createdAtMs + 1000;
    return edited
      ? `edited ${relativeTime(s.updatedAtMs)}`
      : `added ${relativeTime(s.createdAtMs)}`;
  }

  async function loadStatements(): Promise<void> {
    loading = true;
    try {
      const list = await invoke<AuthoredContext[]>("list_user_context_authored");
      statements = list;
      loadError = null;
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error);
      statements = statements ?? [];
    } finally {
      loading = false;
    }
  }

  // Best-effort side-rail context: the engine tier badge + a few inferred
  // Conclusions for the steering links. A failure just leaves the rail quiet.
  async function loadSideContext(): Promise<void> {
    try {
      const [status, list] = await Promise.all([
        invoke<UserContextStatus>("get_user_context_status").catch(() => null),
        invoke<Conclusion[]>("list_user_context_conclusions", {
          includeFaded: false,
        }).catch(() => null),
      ]);
      if (status) budgetTier = status.budgetTier;
      if (list) conclusions = list;
    } catch {
      // Best-effort; the rail degrades gracefully.
    }
  }

  function applySuggestion(prompt: string): void {
    // Prefill an empty composer, otherwise append onto the existing draft.
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
      // Append optimistically (newest-first) — no re-list needed.
      statements = [created, ...(statements ?? [])];
      draftText = "";
      draftTopic = "";
    } catch (error) {
      composerError = error instanceof Error ? error.message : String(error);
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
        s.id === id
          ? { ...s, text, topic: nextTopic, updatedAtMs: Date.now() }
          : s,
      );
      cancelEdit();
    } catch (error) {
      // The list-load error surface only renders when there are no statements,
      // so it's unreachable here (we're editing an existing one). Show a visible
      // dialog instead of silently swallowing the failure.
      const detail = error instanceof Error ? error.message : String(error);
      await message(detail, { title: "Couldn't save context", kind: "error" });
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
      // Same unreachable-surface problem as saveEdit — surface a visible dialog.
      const detail = error instanceof Error ? error.message : String(error);
      await message(detail, { title: "Couldn't delete context", kind: "error" });
    }
  }

  $effect(() => {
    void untrack(() => loadStatements());
    void untrack(() => loadSideContext());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => {
      void loadStatements();
      void loadSideContext();
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
  <!-- ── Page header ── -->
  <header class="ctx-header">
    <h1>Context</h1>
    <p class="subtitle">
      What you tell Mnema about yourself. It steers your dossier and
      <span class="accent-word">never fades</span> like an inferred conclusion.
      The Sensitive Category Guardrail keeps off-limits categories from being
      surfaced.
    </p>
  </header>

  <div class="ctx-panes">
    <!-- ────────────────────────── MAIN COLUMN ────────────────────────── -->
    <div class="ctx-main">
      <!-- COMPOSER -->
      <div class="card composer">
        <div class="composer-head">
          <span class="section-title">Add context</span>
          <span class="spacer"></span>
          <span class="tier-badge" title="Reasoning Engine derivation tier">
            <span class="dot" aria-hidden="true"></span>{tierLabel(budgetTier)}
          </span>
          <span class="authored-pill"><span class="quill">✎</span>authored</span>
        </div>

        <textarea
          bind:this={composerEl}
          bind:value={draftText}
          class="composer-input"
          placeholder="I'm a… I care about… I work best with…"
          aria-label="Add a context statement"
          onkeydown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
              e.preventDefault();
              void submitDraft();
            }
          }}
        ></textarea>

        <div class="composer-topic">
          <input
            bind:value={draftTopic}
            class="topic-input"
            type="text"
            placeholder="topic (optional, e.g. role, focus, goal)"
            aria-label="Topic for this statement (optional)"
          />
        </div>

        <div class="composer-suggest">
          <span class="suggest-label">Try</span>
          {#each SUGGESTIONS as s (s.label)}
            <button
              type="button"
              class="chip chip--suggest"
              onclick={() => applySuggestion(s.prompt)}
            >
              {s.label}
            </button>
          {/each}
        </div>

        <div class="composer-foot">
          <span class="helper">
            <span class="hint-glyph" aria-hidden="true">›</span>Authored statements
            never fade from your dossier.
          </span>
          <button
            type="button"
            class="btn btn--accent"
            disabled={!canSubmit}
            onclick={() => void submitDraft()}
          >
            {submitting ? "Adding…" : "＋ Add"}
          </button>
        </div>

        {#if composerError}
          <p class="composer-error">{composerError}</p>
        {/if}
      </div>

      <!-- LIST HEADER -->
      <div class="list-head">
        <span class="section-title">Standing context</span>
        <span class="pill count-pill">{countLabel}</span>
      </div>

      <!-- LIST / STATES -->
      {#if loadError && !statements}
        <div class="state state--error">
          <p class="state-title">Couldn't load your context.</p>
          <p class="state-detail">{loadError}</p>
          <button
            type="button"
            class="state-retry"
            onclick={() => void loadStatements()}
            disabled={loading}
          >
            <span class="state-retry-ico" aria-hidden="true">↻</span>
            Try again
          </button>
        </div>
      {:else if loading && !statements}
        <!-- Loading skeleton rows — mirror the authored-statement shape. The
             empty state below only renders once loading has completed with no
             statements, so loading and empty stay visually distinct. -->
        <div class="stmt-list" aria-label="Loading context" aria-busy="true">
          {#each Array.from({ length: SKELETON_COUNT }) as _, i (i)}
            <div class="stmt stmt--skeleton">
              <Skeleton variant="text" width="82%" height="13px" />
              <Skeleton variant="text" width="54%" height="13px" />
              <div class="stmt-meta">
                <Skeleton variant="text" width="60px" height="11px" radius="4px" />
                <Skeleton variant="text" width="72px" height="11px" radius="999px" />
                <Skeleton variant="text" width="64px" height="11px" />
              </div>
            </div>
          {/each}
        </div>
      {:else if (statements?.length ?? 0) === 0}
        <div class="state state--empty">
          <p class="state-title">No standing context yet.</p>
          <p class="state-detail">
            Add a short statement above — your role, what you're working on, how
            you work, what you care about. Mnema uses it to steer your dossier,
            and it never fades.
          </p>
        </div>
      {:else}
        <div class="stmt-list">
          {#each statements ?? [] as s (s.id)}
            {#if editingId === s.id}
              <!-- inline edit -->
              <div class="stmt stmt--editing">
                <div class="stmt-edit">
                  <textarea
                    bind:value={editText}
                    aria-label="Edit context statement"
                  ></textarea>
                  <input
                    bind:value={editTopic}
                    class="topic-input"
                    type="text"
                    placeholder="topic (optional)"
                    aria-label="Edit topic (optional)"
                  />
                  <div class="edit-row">
                    <button
                      type="button"
                      class="btn btn--accent"
                      disabled={editText.trim().length === 0 || savingEdit}
                      onclick={() => void saveEdit(s.id)}
                    >
                      {savingEdit ? "Saving…" : "Save"}
                    </button>
                    <button type="button" class="btn" onclick={cancelEdit}>
                      Cancel
                    </button>
                    <span class="editing-tag">
                      <span class="quill">✎</span>editing
                    </span>
                  </div>
                </div>
              </div>
            {:else}
              <div class="stmt">
                <div class="stmt-text">{s.text}</div>
                <div class="stmt-meta">
                  {#if s.topic}
                    <span class="topic-chip">{s.topic}</span>
                  {/if}
                  <span class="authored-pill">
                    <span class="quill">✎</span>Authored
                  </span>
                  <span class="meta-time">{metaTime(s)}</span>
                  <span class="meta-actions">
                    <button
                      type="button"
                      class="btn btn--ghost"
                      onclick={() => startEdit(s)}
                    >
                      Edit
                    </button>
                    <button
                      type="button"
                      class="btn btn--ghost btn--danger-hover"
                      onclick={() => void deleteStatement(s)}
                    >
                      Delete
                    </button>
                  </span>
                </div>
              </div>
            {/if}
          {/each}
        </div>
      {/if}
    </div>

    <!-- ────────────────────────── SIDE PANEL ────────────────────────── -->
    <aside class="ctx-side" aria-label="How Mnema uses this">
      <!-- AUTHORED vs INFERRED -->
      <div class="side-card">
        <div class="side-title"><span>How Mnema uses this</span></div>

        <div class="av-row">
          <span class="av-glyph av-glyph--authored" aria-hidden="true">✎</span>
          <div class="av-body">
            <div class="av-head">
              Authored <span class="av-where">· this page</span>
            </div>
            <div class="av-desc">
              You asserted it. It steers your dossier up front and stays as
              written.
            </div>
            <span class="steady-mark"><i></i>never fades</span>
          </div>
        </div>

        <div class="av-row">
          <span class="av-glyph av-glyph--inferred" aria-hidden="true">◆</span>
          <div class="av-body">
            <div class="av-head">
              Inferred <span class="av-where">· your dossier</span>
            </div>
            <div class="av-desc">
              Mnema concluded it from your activity. Confidence rises and fades
              over time.
            </div>
            <span class="fade-mark"
              ><span class="mini-conf"><i></i></span>confidence rises &amp;
              fades</span
            >
          </div>
        </div>
      </div>

      <!-- STEERING LINKS -->
      <div class="side-card">
        <div class="side-title"><span>Steering your dossier</span></div>

        {#if steerLinks.length > 0}
          <div class="steer-list">
            {#each steerLinks as c (c.id)}
              <div class="steer">
                <div class="steer-rail" aria-hidden="true">
                  <span class="node"></span>
                  <span class="line"></span>
                  <span class="arrow">▾</span>
                </div>
                <div class="steer-body">
                  <div class="steer-from">
                    <span class="quill">✎</span>{c.subject}
                  </div>
                  <div class="steer-to">
                    <span class="supports">supports</span>
                    <span class="infer-chip" title="Inferred conclusion">
                      ◆ {c.statement}
                    </span>
                    <span class="infer-conf">
                      {Math.round(c.confidence * 100)}%
                    </span>
                  </div>
                </div>
              </div>
            {/each}
          </div>
        {:else}
          <p class="steer-empty">
            As Mnema forms inferred conclusions, you'll see how your authored
            context steers them here.
          </p>
        {/if}
      </div>

      <!-- GUARDRAIL PRINT -->
      <div class="side-card guardrail-card">
        <span class="lock" aria-hidden="true">⊘</span>
        <div class="gd-body">
          <div class="gd-title">Sensitive Category Guardrail</div>
          <div class="gd-text">
            Health, politics, sexuality, religion, and similar are never inferred
            or surfaced — even if you mention them here. Mnema errs toward
            over-suppression.
          </div>
        </div>
      </div>
    </aside>
  </div>
</section>

<style>
  .ctx {
    display: flex;
    flex-direction: column;
    gap: 18px;
  }

  /* ---- Page header ---- */
  .ctx-header {
    max-width: 760px;
  }
  .ctx-header h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--app-text-strong);
  }
  .ctx-header .subtitle {
    margin: 4px 0 0;
    font-size: 12.5px;
    line-height: 1.6;
    color: var(--app-text-muted);
  }
  .ctx-header .subtitle .accent-word {
    color: var(--app-accent-strong);
  }

  /* ---- Two-pane grid ---- */
  .ctx-panes {
    display: grid;
    grid-template-columns: 2fr 1fr;
    gap: 20px;
    align-items: start;
  }
  @media (max-width: 940px) {
    .ctx-panes {
      grid-template-columns: 1fr;
    }
  }

  .ctx-main {
    display: flex;
    flex-direction: column;
    gap: 16px;
    min-width: 0;
  }

  .ctx-side {
    position: sticky;
    top: 4px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-width: 0;
  }
  @media (max-width: 940px) {
    .ctx-side {
      position: static;
    }
  }

  /* ---- Shared card + control primitives (mirrors app.css) ---- */
  .card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 14px;
  }
  .section-title {
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-muted);
  }
  .pill {
    display: inline-flex;
    align-items: center;
    font-size: 11px;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
  }

  .btn {
    font: inherit;
    font-size: 11.5px;
    line-height: 1;
    letter-spacing: 0.02em;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 0 11px;
    height: 26px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .btn:hover {
    color: var(--app-text-strong);
    border-color: var(--app-border-hover);
  }
  .btn:not(:disabled):active {
    transform: translateY(1px);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .btn--accent {
    border-color: var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .btn--accent:hover:not(:disabled) {
    border-color: var(--app-accent);
    color: var(--app-accent);
  }
  .btn--ghost {
    border-color: transparent;
    background: transparent;
    padding: 0 8px;
    height: 22px;
  }
  .btn--ghost:hover {
    background: var(--app-surface-hover);
    border-color: transparent;
  }
  .btn--danger-hover:hover {
    color: var(--app-danger);
    background: var(--app-danger-bg);
    border-color: transparent;
  }

  .chip {
    font: inherit;
    display: inline-flex;
    align-items: center;
    font-size: 11px;
    letter-spacing: 0.02em;
    padding: 2px 9px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
  }

  /* ---- Composer ---- */
  .composer {
    background: var(--app-surface-raised);
  }
  .composer-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 11px;
  }
  .composer-head .spacer {
    flex: 1 1 auto;
  }

  .tier-badge {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
  }
  .tier-badge .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--app-accent);
  }

  .composer-input {
    width: 100%;
    min-height: 60px;
    resize: vertical;
    font: inherit;
    font-size: 13.5px;
    line-height: 1.6;
    padding: 11px 12px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      box-shadow 0.12s ease;
  }
  .composer-input::placeholder {
    color: var(--app-text-faint);
  }
  .composer-input:focus {
    outline: none;
    border-color: var(--app-accent-border);
    background: var(--app-surface);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .composer-topic {
    margin-top: 9px;
  }
  .topic-input {
    width: 100%;
    font: inherit;
    font-size: 12px;
    padding: 8px 11px;
    border: 1px solid var(--app-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .topic-input::placeholder {
    color: var(--app-text-faint);
  }
  .topic-input:focus {
    outline: none;
    border-color: var(--app-accent-border);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }

  .composer-suggest {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 11px 0 0;
  }
  .composer-suggest .suggest-label {
    font-size: 10px;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
    margin-right: 2px;
  }
  .chip--suggest {
    cursor: pointer;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .chip--suggest:hover {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .chip--suggest:not(:disabled):active {
    transform: translateY(1px);
  }

  .composer-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    margin-top: 13px;
    padding-top: 12px;
    border-top: 1px dashed var(--app-border);
  }
  .composer-foot .helper {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .composer-foot .helper .hint-glyph {
    color: var(--app-text-subtle);
  }
  .composer-error {
    margin: 10px 0 0;
    font-size: 11.5px;
    color: var(--app-danger);
    line-height: 1.5;
  }

  /* ---- List ---- */
  .list-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 2px 2px 0;
  }
  .count-pill {
    font-size: 10.5px;
    min-width: 18px;
    justify-content: center;
    padding: 1px 7px;
    font-variant-numeric: tabular-nums;
  }

  .stmt-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .stmt {
    display: flex;
    flex-direction: column;
    gap: 9px;
    padding: 13px 14px;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface);
    /* steady authored-ledger accent: a quiet left edge so authored statements
       read as durable, not decaying. */
    border-left: 2px solid var(--app-accent-border);
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .stmt:hover {
    border-color: var(--app-border-hover);
    border-left-color: var(--app-accent);
  }
  .stmt--skeleton {
    border-left-color: var(--app-border);
  }
  .stmt--skeleton:hover {
    border-color: var(--app-border);
    border-left-color: var(--app-border);
  }

  .stmt-text {
    font-size: 13.5px;
    line-height: 1.55;
    color: var(--app-text-strong);
    font-weight: 600;
  }

  .stmt-meta {
    display: flex;
    align-items: center;
    gap: 9px;
    flex-wrap: wrap;
  }
  .topic-chip {
    display: inline-flex;
    align-items: center;
    font-size: 10px;
    letter-spacing: 0.02em;
    padding: 1px 7px;
    border-radius: 4px;
    border: 1px solid var(--app-border);
    background: var(--app-surface-hover);
    color: var(--app-text-muted);
  }
  .topic-chip::before {
    content: "[";
    color: var(--app-text-faint);
  }
  .topic-chip::after {
    content: "]";
    color: var(--app-text-faint);
  }

  .authored-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
    letter-spacing: 0.02em;
    padding: 1px 8px;
    border-radius: 999px;
    border: 1px solid var(--app-accent-border);
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
  }
  .authored-pill .quill {
    font-size: 9.5px;
  }

  .stmt-meta .meta-time {
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .stmt-meta .meta-actions {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 2px;
    opacity: 0.55;
    transition: opacity 0.12s ease;
  }
  .stmt:hover .meta-actions {
    opacity: 1;
  }

  /* ---- inline edit state ---- */
  .stmt--editing {
    background: var(--app-surface-raised);
    border-left-color: var(--app-accent);
    border-color: var(--app-accent-border);
  }
  .stmt-edit {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .stmt-edit textarea {
    width: 100%;
    min-height: 52px;
    resize: vertical;
    font: inherit;
    font-size: 13.5px;
    line-height: 1.55;
    padding: 10px 11px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-surface-subtle);
    color: var(--app-text-strong);
  }
  .stmt-edit textarea:focus {
    outline: none;
    border-color: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .stmt-edit .edit-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .stmt-edit .editing-tag {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-accent-strong);
  }

  /* ---- Side panel ---- */
  .side-card {
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
    padding: 13px 14px;
  }
  .side-card .side-title {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    margin-bottom: 11px;
  }

  /* authored vs inferred mini rows */
  .av-row {
    display: grid;
    grid-template-columns: 22px 1fr;
    gap: 9px;
    padding: 10px 0;
    align-items: start;
  }
  .av-row + .av-row {
    border-top: 1px dashed var(--app-border);
  }
  .av-glyph {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    font-size: 12px;
  }
  .av-glyph--authored {
    color: var(--app-accent-strong);
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
  }
  .av-glyph--inferred {
    color: var(--app-info);
    background: var(--app-info-bg);
    border: 1px solid var(--app-info-border);
  }
  .av-body .av-head {
    font-size: 12px;
    font-weight: 600;
    color: var(--app-text-strong);
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .av-body .av-head .av-where {
    font-weight: 400;
    font-size: 10.5px;
    color: var(--app-text-subtle);
  }
  .av-body .av-desc {
    font-size: 11px;
    line-height: 1.5;
    color: var(--app-text-muted);
    margin-top: 2px;
  }
  .steady-mark {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-top: 6px;
    font-size: 9.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-accent-strong);
  }
  .steady-mark i {
    width: 24px;
    height: 4px;
    border-radius: 999px;
    background: var(--app-accent);
    opacity: 0.85;
    display: inline-block;
  }
  .fade-mark {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    margin-top: 6px;
    font-size: 9.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .mini-conf {
    width: 36px;
    height: 4px;
    border-radius: 999px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
    overflow: hidden;
    display: inline-block;
  }
  .mini-conf > i {
    display: block;
    height: 100%;
    width: 60%;
    background: linear-gradient(90deg, var(--app-info), transparent);
    opacity: 0.85;
  }

  /* steering links */
  .steer-list {
    display: flex;
    flex-direction: column;
    gap: 11px;
  }
  .steer {
    display: grid;
    grid-template-columns: 14px 1fr;
    gap: 9px;
  }
  .steer-rail {
    position: relative;
    width: 14px;
  }
  .steer-rail .node {
    position: absolute;
    left: 4px;
    top: 3px;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent-bg);
    border: 1.5px solid var(--app-accent);
  }
  .steer-rail .line {
    position: absolute;
    left: 7px;
    top: 9px;
    bottom: 7px;
    width: 1px;
    background: linear-gradient(
      180deg,
      var(--app-accent-border),
      var(--app-info-border)
    );
  }
  .steer-rail .arrow {
    position: absolute;
    left: 3px;
    bottom: 0;
    color: var(--app-info);
    font-size: 10px;
    line-height: 1;
  }
  .steer-body {
    min-width: 0;
  }
  .steer-from {
    font-size: 12px;
    line-height: 1.5;
    color: var(--app-text-strong);
    font-weight: 600;
  }
  .steer-from .quill {
    color: var(--app-accent-strong);
    margin-right: 3px;
  }
  .steer-to {
    display: flex;
    align-items: center;
    gap: 7px;
    flex-wrap: wrap;
    margin-top: 5px;
    font-size: 11px;
    line-height: 1.45;
    color: var(--app-text-muted);
  }
  .steer-to .supports {
    color: var(--app-text-subtle);
  }
  .infer-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--app-info-bg);
    border: 1px solid var(--app-info-border);
    color: var(--app-info);
    min-width: 0;
  }
  .infer-conf {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 10.5px;
    color: var(--app-text-muted);
    font-variant-numeric: tabular-nums;
  }
  .steer-empty {
    margin: 0;
    font-size: 11px;
    line-height: 1.5;
    color: var(--app-text-muted);
  }

  /* guardrail print card */
  .guardrail-card {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    background: var(--app-surface-subtle);
    border-style: dashed;
  }
  .guardrail-card .lock {
    flex: 0 0 auto;
    width: 24px;
    height: 24px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    border: 1px solid var(--app-border-strong);
    color: var(--app-text-subtle);
    font-size: 12px;
  }
  .guardrail-card .gd-body {
    min-width: 0;
  }
  .guardrail-card .gd-title {
    font-size: 11px;
    font-weight: 600;
    color: var(--app-text);
    letter-spacing: 0.02em;
  }
  .guardrail-card .gd-text {
    font-size: 11px;
    line-height: 1.5;
    color: var(--app-text-muted);
    margin-top: 3px;
  }

  /* ---- States ---- */
  .state {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 18px;
    background: var(--app-surface);
    border: 1px solid var(--app-border);
    border-radius: 9px;
  }
  .state--error {
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
  }
  .state--empty {
    border-style: dashed;
  }
  .state-title {
    margin: 0;
    font-size: 13px;
    color: var(--app-text-strong);
  }
  .state-detail {
    margin: 0;
    font-size: 11.5px;
    color: var(--app-text-muted);
    line-height: 1.6;
  }
  /* Retry affordance — mirrors the Overview lede's "↻ re-read" pill. */
  .state-retry {
    align-self: flex-start;
    margin-top: 4px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 2px 7px;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: transparent;
    color: var(--app-text-subtle);
    font: inherit;
    font-size: 10px;
    letter-spacing: 0.18em;
    text-transform: uppercase;
    cursor: pointer;
    transition:
      color 0.12s ease,
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .state-retry:hover:not(:disabled) {
    color: var(--app-accent);
    border-color: var(--app-accent);
    background: var(--app-accent-bg);
  }
  .state-retry:not(:disabled):active {
    transform: translateY(1px);
  }
  .state-retry:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .state-retry-ico {
    font-size: 12px;
    line-height: 1;
    letter-spacing: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .btn:not(:disabled):active,
    .chip--suggest:not(:disabled):active,
    .state-retry:not(:disabled):active {
      transform: none;
    }
  }
</style>
