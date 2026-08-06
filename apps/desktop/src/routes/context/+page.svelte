<script lang="ts">
  // Context (page 10) — a destination opened from Overview's Context tile (⌃K);
  // `esc` returns. Two columns, because there are two kinds of knowing: the
  // standing statements you write (yours, never fade) and what the engine
  // worked out (counted, never narrated).
  //
  // Every row is a keyboard row: ↑↓ moves with a full-row accent, ⏎ edits,
  // ⌘⏎ saves, ⌘⌫ deletes, ⌘D opens the dismissal archive, esc goes back. The
  // deck carries the save state exactly as it does in settings (G7 — no save
  // bar, ever), which is the whole reason the deck exists.
  //
  // Backend is the shipped #107/#99 command set; this route adds no Rust.
  import { untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";

  import { resetDeck, setDeck, type DeckStatus } from "$lib/deck.svelte";
  import { resetCrumbs, setCrumbs } from "$lib/crumb.svelte";
  import { humanizeError } from "$lib/format-error";
  import { toast } from "$lib/toast.svelte";
  import type { AuthoredContext, Conclusion, DismissedView } from "$lib/types/recording";

  // The surface's shared row / card / callout vocabulary, used by this route
  // and by both of its children — every selector anchored on `.ctx`.
  import "$lib/context/context.css";
  import Archive from "$lib/context/Archive.svelte";
  import EngineRail from "$lib/context/EngineRail.svelte";
  import {
    EMPTY_SNAPSHOT,
    PREFILLS,
    loadContext,
    statementStamp,
    wipeMessage,
    type ContextSnapshot,
  } from "$lib/context/data";

  let snap = $state<ContextSnapshot>({ ...EMPTY_SNAPSHOT });
  let loaded = $state(false);
  let nowMs = $state(Date.now());

  // ── keyboard rows ──────────────────────────────────────────────────────────
  let selected = $state(0);
  let archiveOpen = $state(false);
  let archiveSelected = $state(0);

  // ── composer + inline edit ─────────────────────────────────────────────────
  let draft = $state("");
  let draftTopic = $state("");
  let adding = $state(false);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let editingId = $state<number | null>(null);
  let editText = $state("");
  let editTopic = $state("");
  let editEl = $state<HTMLTextAreaElement | null>(null);

  // ── the deck's save state (settings' pattern, same slot) ───────────────────
  let saveStatus = $state<DeckStatus | null>(null);
  let busy = $state(false);
  let wiping = $state(false);
  let restoringKey = $state<string | null>(null);

  const statements = $derived(snap.statements);
  const dismissed = $derived(snap.dismissed);
  const conclusionCount = $derived(snap.status?.conclusionCount ?? snap.conclusions.length);

  function dismissedKey(d: DismissedView): string {
    return `${d.subject}\0${d.statement}`;
  }

  function markSaving(): void {
    busy = true;
    saveStatus = { tone: "quiet", text: "Saving…" };
  }

  function markSaved(): void {
    busy = false;
    const at = new Date().toLocaleTimeString(undefined, { hour12: false });
    saveStatus = { tone: "ok", text: `All changes saved · ${at}` };
  }

  // A failed write is the one persistent state: the deck names the cause and a
  // toast carries the message, so the two can never disagree (G7).
  function markFailed(what: string, error: unknown): void {
    busy = false;
    const detail = humanizeError(error);
    saveStatus = { tone: "danger", text: `Not saved — ${what}` };
    toast({ tone: "error", title: `Couldn't ${what}`, message: detail });
  }

  async function reload(): Promise<void> {
    snap = await loadContext();
    loaded = true;
    // Same convention as settings: the deck states the resting truth ("nothing
    // unsaved") from the moment the surface has read, and gains a timestamp the
    // first time you change something.
    saveStatus ??= { tone: "ok", text: "All changes saved" };
    if (selected > statements.length - 1) selected = Math.max(0, statements.length - 1);
    if (archiveSelected > dismissed.length - 1) archiveSelected = Math.max(0, dismissed.length - 1);
  }

  // Load once on mount, then follow `user_context_changed`. `untrack` so the
  // loader's own state writes can never re-trigger this effect.
  $effect(() => {
    untrack(() => void reload());

    let unlisten: UnlistenFn | undefined;
    let disposed = false;
    void listen("user_context_changed", () => void reload()).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });

    // One clock for every "N ago" on the surface, ticking only while shown.
    let tick: ReturnType<typeof setInterval> | null = null;
    const stop = (): void => {
      if (tick !== null) clearInterval(tick);
      tick = null;
    };
    const sync = (): void => {
      stop();
      if (document.visibilityState !== "visible") return;
      nowMs = Date.now();
      tick = setInterval(() => (nowMs = Date.now()), 30_000);
    };
    sync();
    document.addEventListener("visibilitychange", sync);

    return () => {
      disposed = true;
      unlisten?.();
      stop();
      document.removeEventListener("visibilitychange", sync);
    };
  });

  // ── writes ─────────────────────────────────────────────────────────────────
  async function addStatement(): Promise<void> {
    const text = draft.trim();
    if (text.length === 0 || adding) return;
    adding = true;
    markSaving();
    const topic = draftTopic.trim();
    try {
      const created = await invoke<AuthoredContext>("user_context_add_authored", {
        text,
        topic: topic.length > 0 ? topic : null,
      });
      snap.statements = [created, ...snap.statements];
      draft = "";
      draftTopic = "";
      selected = 0;
      markSaved();
    } catch (error) {
      markFailed("add this statement", error);
    } finally {
      adding = false;
    }
  }

  function startEdit(index: number): void {
    const s = statements[index];
    if (!s) return;
    selected = index;
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
    if (text.length === 0 || busy) return;
    const topic = editTopic.trim();
    const nextTopic = topic.length > 0 ? topic : null;
    markSaving();
    try {
      await invoke("user_context_update_authored", { id, text, topic: nextTopic });
      snap.statements = snap.statements.map((s) =>
        s.id === id ? { ...s, text, topic: nextTopic, updatedAtMs: Date.now() } : s,
      );
      cancelEdit();
      markSaved();
    } catch (error) {
      markFailed("save this statement", error);
    }
  }

  async function deleteStatement(index: number): Promise<void> {
    const s = statements[index];
    if (!s || busy) return;
    const ok = await confirm(
      "Delete this standing statement? Mnema stops using it to steer what it works out. Nothing it has already concluded changes.",
      { title: "Delete standing statement", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    markSaving();
    try {
      await invoke("user_context_delete_authored", { id: s.id });
      snap.statements = snap.statements.filter((x) => x.id !== s.id);
      if (editingId === s.id) cancelEdit();
      selected = Math.min(index, Math.max(0, snap.statements.length - 1));
      markSaved();
    } catch (error) {
      markFailed("delete this statement", error);
    }
  }

  async function restoreDismissed(d: DismissedView): Promise<void> {
    const key = dismissedKey(d);
    if (restoringKey === key) return;
    restoringKey = key;
    markSaving();
    try {
      await invoke("user_context_restore_dismissed", { subject: d.subject, statement: d.statement });
      snap.dismissed = snap.dismissed.filter((x) => dismissedKey(x) !== key);
      archiveSelected = Math.min(archiveSelected, Math.max(0, snap.dismissed.length - 1));
      markSaved();
    } catch (error) {
      markFailed("restore this belief", error);
    } finally {
      restoringKey = null;
    }
  }

  async function pinConclusion(c: Conclusion): Promise<void> {
    if (busy) return;
    markSaving();
    try {
      await invoke("user_context_set_pinned", { id: c.id, pinned: !c.pinned });
      snap.conclusions = snap.conclusions.map((x) =>
        x.id === c.id ? { ...x, pinned: !c.pinned } : x,
      );
      markSaved();
    } catch (error) {
      markFailed("pin this conclusion", error);
    }
  }

  async function dismissConclusion(c: Conclusion): Promise<void> {
    if (busy) return;
    // Dismiss deletes the belief and writes a veto — say so before it happens.
    const ok = await confirm(
      "Dismissing deletes this belief and records a veto against it. It can only form again from substantially fresher evidence.",
      { title: "Dismiss this conclusion?", kind: "warning", okLabel: "Dismiss", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    markSaving();
    try {
      await invoke("user_context_dismiss_conclusion", { id: c.id });
      snap.conclusions = snap.conclusions.filter((x) => x.id !== c.id);
      markSaved();
    } catch (error) {
      markFailed("dismiss this conclusion", error);
    }
  }

  // The one destructive action. The dialog names every category the call really
  // clears, with this machine's counts — the shipped settings confirmation
  // mentions neither standing statements nor ask history, and both go.
  async function wipeContext(): Promise<void> {
    if (wiping) return;
    const ok = await confirm(wipeMessage(snap.status, statements.length), {
      title: "Wipe everything Mnema has concluded?",
      kind: "warning",
      okLabel: "Wipe",
      cancelLabel: "Cancel",
    });
    if (!ok) return;
    wiping = true;
    markSaving();
    try {
      await invoke("wipe_user_context");
      await reload();
      markSaved();
    } catch (error) {
      markFailed("wipe user context", error);
    } finally {
      wiping = false;
    }
  }

  // ── keyboard ───────────────────────────────────────────────────────────────
  function isTyping(target: EventTarget | null): boolean {
    return target instanceof HTMLElement && /^(INPUT|TEXTAREA)$/.test(target.tagName);
  }

  function onKeydown(event: KeyboardEvent): void {
    const mod = event.metaKey || event.ctrlKey;
    const typing = isTyping(event.target);

    if (event.key === "Escape") {
      if (editingId !== null) {
        event.preventDefault();
        cancelEdit();
        return;
      }
      if (typing) {
        (event.target as HTMLElement).blur();
        return;
      }
      event.preventDefault();
      if (archiveOpen) archiveOpen = false;
      else void goto("/overview");
      return;
    }

    if (mod && event.key === "Enter") {
      event.preventDefault();
      if (editingId !== null) void saveEdit(editingId);
      else void addStatement();
      return;
    }

    if (typing || event.altKey) return;

    if (mod && (event.key === "d" || event.key === "D")) {
      event.preventDefault();
      archiveOpen = !archiveOpen;
      return;
    }

    if (mod && (event.key === "Backspace" || event.key === "Delete")) {
      if (archiveOpen) return;
      event.preventDefault();
      void deleteStatement(selected);
      return;
    }

    if (mod) return;

    const list = archiveOpen ? dismissed.length : statements.length;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (list === 0) return;
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      if (archiveOpen) archiveSelected = Math.min(list - 1, Math.max(0, archiveSelected + step));
      else selected = Math.min(list - 1, Math.max(0, selected + step));
      return;
    }

    if (event.key === "Enter") {
      if (list === 0) return;
      event.preventDefault();
      if (archiveOpen) {
        const d = dismissed[archiveSelected];
        if (d) void restoreDismissed(d);
      } else {
        startEdit(selected);
      }
    }
  }

  // Focus the edit field as soon as a row enters edit mode — ⏎ has to land you
  // in the text, or the key is a lie.
  $effect(() => {
    if (editingId !== null) editEl?.focus();
  });

  function applyPrefill(prompt: string): void {
    draft = draft.trim().length === 0 ? prompt : `${draft} ${prompt}`;
    composerEl?.focus();
  }

  // ── chrome ─────────────────────────────────────────────────────────────────
  $effect(() => {
    setCrumbs([{ label: "Context" }]);
    return resetCrumbs;
  });

  $effect(() => {
    setDeck({
      context: `Context · ${statements.length} standing ${
        statements.length === 1 ? "statement" : "statements"
      } · ${conclusionCount} ${conclusionCount === 1 ? "conclusion" : "conclusions"}`,
      // A keycap only appears for a key this surface really binds RIGHT NOW —
      // with an empty list there is no row to move to, edit or restore.
      hints: archiveOpen
        ? [
            ...(dismissed.length > 0
              ? [
                  { keys: "↑↓", label: "Move" },
                  { keys: "⏎", label: "Restore" },
                ]
              : []),
            { keys: "esc", label: "Back", separator: dismissed.length > 0 },
          ]
        : [
            ...(statements.length > 0
              ? [
                  { keys: "↑↓", label: "Move" },
                  { keys: "⏎", label: "Edit" },
                  { keys: "⌘⌫", label: "Delete" },
                ]
              : [{ keys: "⌘⏎", label: "Add" }]),
            { keys: "esc", label: "Back to Overview", separator: true },
          ],
      status: saveStatus,
    });
    return resetDeck;
  });
</script>

<svelte:window onkeydown={onKeydown} />

<div class="ctx">
  <div class="ctx__hd">
    <p class="t-title ctx__ttl">Context</p>
    {#if archiveOpen}
      <span class="t-meta">Dismissed beliefs and what deletion actually reaches.</span>
    {:else}
      <span class="t-meta ctx__sub">
        What Mnema works from — the standing statements you write, and the conclusions it forms from
        your activity.
      </span>
      <button
        type="button"
        class="btn btn--sm ctx__arch"
        onclick={() => (archiveOpen = true)}
        aria-label="Show dismissed beliefs"
      >
        Dismissed · {snap.status?.dismissedCount ?? dismissed.length}
        <span class="kbd">⌘D</span>
      </button>
    {/if}
  </div>

  {#if snap.error && loaded}
    <p class="ctx__err t-meta">{snap.error}</p>
  {/if}

  <div class="ctx__body">
    {#if archiveOpen}
      <Archive
        {dismissed}
        selected={archiveSelected}
        {restoringKey}
        {nowMs}
        {wiping}
        keyOf={dismissedKey}
        onSelect={(i) => (archiveSelected = i)}
        onRestore={(d) => void restoreDismissed(d)}
        onHide={() => (archiveOpen = false)}
        onWipe={() => void wipeContext()}
      />
    {:else}
      <div class="ctx__main">
        <div class="comp">
          <div class="comp__h">
            <span class="t-label">Standing context</span>
            <span class="t-meta comp__own">yours · never fades</span>
            <span class="hint comp__k"><span class="kbd">⌘⏎</span><span>add</span></span>
          </div>
          <textarea
            bind:this={composerEl}
            bind:value={draft}
            class="comp__ta"
            rows="2"
            placeholder="I'm a… I care about… I work best with…"
            aria-label="Add a standing context statement"
          ></textarea>
          <div class="comp__ft">
            <span class="t-meta comp__pl">prefill:</span>
            {#each PREFILLS as p (p.label)}
              <button type="button" class="prefill" onclick={() => applyPrefill(p.prompt)}>
                {p.label}
              </button>
            {/each}
            <input
              bind:value={draftTopic}
              class="input comp__topic"
              type="text"
              placeholder="topic (optional)"
              aria-label="Topic for this statement (optional)"
            />
            <button
              type="button"
              class="btn btn--accent btn--sm comp__add"
              disabled={draft.trim().length === 0 || adding}
              onclick={() => void addStatement()}
            >
              {adding ? "Adding…" : "Add"}
            </button>
          </div>
        </div>

        {#if statements.length === 0}
          <div class="stlist">
            <div class="strow strow--empty">
              <span class="strow__t">
                <span class="strow__x">Nothing standing yet.</span>
                <span class="t-meta">
                  Write one short statement above — your role, what you're working on, how you work,
                  what you care about. It steers what Mnema works out, and unlike a conclusion it
                  never fades.
                </span>
              </span>
            </div>
          </div>
        {:else}
          <div class="stlist" role="listbox" aria-label="Standing context statements" tabindex="-1">
            {#each statements as s, i (s.id)}
              {#if editingId === s.id}
                <div class="strow strow--edit">
                  <span class="strow__q" aria-hidden="true">✎</span>
                  <span class="strow__t">
                    <textarea
                      bind:this={editEl}
                      bind:value={editText}
                      class="comp__ta"
                      rows="2"
                      aria-label="Edit statement"
                    ></textarea>
                    <span class="strow__edit-row">
                      <input
                        bind:value={editTopic}
                        class="input comp__topic"
                        type="text"
                        placeholder="topic (optional)"
                        aria-label="Edit topic (optional)"
                      />
                      <button
                        type="button"
                        class="btn btn--accent btn--sm"
                        disabled={editText.trim().length === 0 || busy}
                        onclick={() => void saveEdit(s.id)}
                      >
                        Save <span class="kbd">⌘⏎</span>
                      </button>
                      <button type="button" class="btn btn--ghost btn--sm" onclick={cancelEdit}>
                        Cancel <span class="kbd">esc</span>
                      </button>
                    </span>
                  </span>
                </div>
              {:else}
                <div
                  class="strow"
                  class:is-key={i === selected}
                  role="option"
                  aria-selected={i === selected}
                  tabindex="-1"
                  onclick={() => (selected = i)}
                  ondblclick={() => startEdit(i)}
                  onkeydown={() => {}}
                >
                  <span class="strow__q" aria-hidden="true">✎</span>
                  <span class="strow__t">
                    <span class="strow__x">{s.text}</span>
                    <span class="strow__m">
                      {#if s.topic}
                        <span class="topic">{s.topic}</span>
                      {:else}
                        <span class="topic topic--none">— no topic</span>
                      {/if}
                      {statementStamp(s, nowMs)}
                    </span>
                  </span>
                  {#if i === selected}
                    <span class="strow__a">
                      <button
                        type="button"
                        class="btn btn--ghost btn--sm"
                        onclick={(e) => {
                          e.stopPropagation();
                          startEdit(i);
                        }}
                      >
                        Edit <span class="kbd">⏎</span>
                      </button>
                      <button
                        type="button"
                        class="btn btn--ghost btn--sm"
                        onclick={(e) => {
                          e.stopPropagation();
                          void deleteStatement(i);
                        }}
                      >
                        Delete <span class="kbd">⌘⌫</span>
                      </button>
                    </span>
                  {/if}
                </div>
              {/if}
            {/each}
          </div>
        {/if}
      </div>

      <div class="ctx__rail">
        <EngineRail
          status={snap.status}
          conclusions={snap.conclusions}
          {nowMs}
          {busy}
          onPin={(c) => void pinConclusion(c)}
          onDismiss={(c) => void dismissConclusion(c)}
        />
      </div>
    {/if}
  </div>
</div>
