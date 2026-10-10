// Shared Insights conversation store — the ONE frontend source of truth for the
// conversation-history list AND the selected/open thread (Insights-rail refactor,
// Slice 1). A persistent left rail (later slices) and the Chat workspace both read
// this singleton, so the history list, search, rename, delete, and selection all
// live here instead of duplicated per-surface. Mirrors the repo's `.svelte.ts`
// store idiom (a class with `$state` class fields), like `ModelPoolLoader`.
//
// Selection is a BUS, not a direct call: a surface (the rail, a handoff) asks to
// open a thread via `requestOpen(id)` / `requestNewChat()`, which bumps
// `pendingOpen`; Chat watches that bus and does the actual `get_conversation`
// load (it owns the right-pane turns/streaming/pins). `activeConversationId` is
// written BY Chat and read by the rail for the row highlight — a one-way mirror,
// no loop.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";
import type { ConversationSummary } from "$lib/insights/conversation";
import { groupHistory, type HistoryGroup } from "$lib/chat/chat-format";

const SEARCH_DEBOUNCE_MS = 220;

/** Compact last-activity label ("now" / "5m" / "2h" / "3d" / "2w" / "4mo" / "1y")
 *  for a history row's right-aligned `.when` stamp. Deliberately single-token (no
 *  "ago") to stay narrow in the 200px rail so the chat title keeps the width —
 *  mirrors the mockup's "2h" / "1d" stamps. Pure; the rail imports it directly. */
export function relativeTime(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";
  const diff = Date.now() - ms;
  if (diff < 0) return "now";
  const min = Math.floor(diff / 60000);
  if (min < 1) return "now";
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d`;
  const wk = Math.floor(day / 7);
  if (wk < 5) return `${wk}w`;
  const mo = Math.floor(day / 30);
  if (mo < 12) return `${mo}mo`;
  return `${Math.floor(day / 365)}y`;
}

export class ConversationStore {
  /** Newest-first conversation rows for the history list. */
  conversations = $state<ConversationSummary[]>([]);
  /** True once a full history fetch has completed at least once. */
  historyLoaded = $state(false);
  /** True when the latest history fetch failed — "couldn't load", NOT empty. */
  historyError = $state(false);
  /** The debounced search query over the history list. */
  searchQuery = $state("");
  /** The conversation id currently being inline-renamed, or null. */
  renamingId = $state<string | null>(null);
  /** The in-progress rename text. */
  renameDraft = $state("");
  /** The currently-open thread id. Chat WRITES this; the rail reads it for the
   *  row highlight (one-way mirror — never read back into Chat's state). */
  activeConversationId = $state<string | null>(null);
  /** The selection BUS. `id === null` means "start a new empty chat"; a string
   *  is a thread to open. `nonce` bumps on EVERY request so re-requesting the
   *  same id still re-triggers the watcher. `prefill` (new-chat only) carries an
   *  optional question to seed the composer with — a hand-off (e.g. "Ask AI about
   *  {subject}") drops the user into a fresh chat with the prompt already typed,
   *  ready to review/edit and send. `send` (the shell's ask field → `/chat?q=`)
   *  sends the prefill once; Chat clears it via `settleOpen` when consumed. */
  pendingOpen = $state<{
    id: string | null;
    nonce: number;
    prefill: string | null;
    send: boolean;
  }>({
    id: null,
    nonce: 0,
    prefill: null,
    send: false,
  });

  /** `conversations` grouped for the list: Pinned first, then by date. */
  historyGroups = $derived.by((): HistoryGroup[] => groupHistory(this.conversations));

  /** The last pin/unpin that failed (already reverted) — the page shows a
   *  danger toast with Retry; null once dismissed. */
  pinFailure = $state<{ conversationId: string; pinned: boolean } | null>(null);

  // Generation token so a stale (out-of-order) history/search response is dropped.
  #historyGeneration = 0;
  // Search-debounce timer handle.
  #searchDebounce: ReturnType<typeof setTimeout> | null = null;
  // Idempotency guard for ensureStarted() — only the first caller does work.
  #started = false;

  /** Refresh the history list: a search invoke when the query is non-empty, a
   *  plain list otherwise, both capped at 60 rows. A generation token drops a
   *  stale (out-of-order) response so a slow earlier fetch can't clobber a newer
   *  one. */
  async refreshHistory(): Promise<void> {
    this.#historyGeneration += 1;
    const generation = this.#historyGeneration;
    const trimmed = this.searchQuery.trim();
    try {
      const rows =
        trimmed.length > 0
          ? await invoke<ConversationSummary[]>("search_conversations", {
              query: trimmed,
              limit: 60,
            })
          : await invoke<ConversationSummary[]>("list_conversations", {
              limit: 60,
              offset: 0,
            });
      if (generation !== this.#historyGeneration) return;
      this.conversations = rows;
      this.historyError = false;
    } catch {
      // A failure is an error state, never an empty list ("no chats").
      if (generation !== this.#historyGeneration) return;
      this.historyError = true;
    } finally {
      if (generation === this.#historyGeneration) this.historyLoaded = true;
    }
  }

  /** Debounce a search input change into a `refreshHistory()`. */
  onSearchInput(): void {
    if (this.#searchDebounce !== null) clearTimeout(this.#searchDebounce);
    this.#searchDebounce = setTimeout(() => {
      this.#searchDebounce = null;
      void this.refreshHistory();
    }, SEARCH_DEBOUNCE_MS);
  }

  // ── Inline rename (left rail hover action) ─────────────────────────────────
  // Clicking ✎ swaps the row's title for a text input pre-filled with the
  // current title. The commit optimistically rewrites the local row so the rail
  // doesn't flicker while the backend's `conversation_changed` refresh catches
  // up; a failed persist re-fetches to undo the optimism.
  startRename(summary: ConversationSummary): void {
    this.renamingId = summary.conversationId;
    this.renameDraft = summary.title || summary.preview || "";
  }

  cancelRename(): void {
    this.renamingId = null;
    this.renameDraft = "";
  }

  async commitRename(): Promise<void> {
    const id = this.renamingId;
    if (id === null) return;
    const title = this.renameDraft.trim();
    const row = this.conversations.find((c) => c.conversationId === id);
    const current = (row?.title || row?.preview || "").trim();
    this.renamingId = null;
    this.renameDraft = "";
    // Empty or unchanged → cancel (the less surprising blur behavior).
    if (title.length === 0 || title === current) return;
    // Optimistic: rewrite the row text now; `conversation_changed` re-fetches
    // the authoritative list right after the backend persists. (Chat derives the
    // active header title from this list, so a rename of the open thread shows
    // immediately without the store touching `activeConversationId`.)
    this.conversations = this.conversations.map((c) =>
      c.conversationId === id ? { ...c, title } : c,
    );
    try {
      await invoke("set_conversation_title", {
        request: { conversationId: id, title },
      });
    } catch {
      // The rename didn't land (e.g. the row vanished) — no
      // conversation_changed will fire, so re-fetch to undo the optimism.
      void this.refreshHistory();
    }
  }

  /** Pin or unpin a chat. Optimistic: the row moves now and the backend's
   *  `conversation_changed` refresh confirms it; a failed persist fires no
   *  event, so the row is flipped back here and `pinFailure` is set. Pinning
   *  never touches a timestamp (the backend owns that rule). */
  async togglePin(conversationId: string): Promise<void> {
    const row = this.conversations.find((c) => c.conversationId === conversationId);
    if (!row) return;
    const pinned = !row.pinned;
    const patch = (value: boolean) =>
      (this.conversations = this.conversations.map((c) =>
        c.conversationId === conversationId ? { ...c, pinned: value } : c,
      ));
    patch(pinned);
    this.pinFailure = null;
    try {
      await invoke("set_conversation_pinned", { conversationId, pinned });
    } catch {
      patch(!pinned);
      this.pinFailure = { conversationId, pinned };
    }
  }

  /** Delete a conversation after a Tauri confirm; true once it's gone (for the
   *  caller's toast). If it was the open thread, arm a fresh empty pane via the
   *  bus. The backend's `conversation_changed` event refreshes the list. */
  async deleteConversation(summary: ConversationSummary): Promise<boolean> {
    const ok = await confirm(
      `Delete “${summary.title || summary.preview || "this chat"}”? This can’t be undone.`,
      { title: "Delete chat", kind: "warning" },
    );
    if (!ok) return false;
    try {
      await invoke("delete_conversation", {
        conversationId: summary.conversationId,
      });
    } catch {
      // The conversation_changed listener refreshes the list regardless.
      return false;
    }
    if (summary.conversationId === this.activeConversationId) {
      // The open conversation was deleted — arm a fresh empty pane.
      this.requestNewChat();
    }
    return true;
  }

  // ── Selection bus ──────────────────────────────────────────────────────────
  /** Ask a surface (Chat) to open `conversationId`. Bumps the nonce so the same
   *  id requested twice still re-triggers. */
  requestOpen(conversationId: string): void {
    const id = conversationId.trim();
    if (!id) return;
    this.pendingOpen = {
      id,
      nonce: this.pendingOpen.nonce + 1,
      prefill: null,
      send: false,
    };
  }

  /** Ask Chat to start a fresh empty chat (id === null). An optional `prefill`
   *  seeds the composer (a Subject→Chat hand-off prefills "Ask AI about …"); the
   *  user reviews/edits and presses Enter — it is never auto-sent. */
  requestNewChat(prefill?: string, send = false): void {
    const seed = prefill?.trim() ?? "";
    this.pendingOpen = {
      id: null,
      nonce: this.pendingOpen.nonce + 1,
      prefill: seed.length > 0 ? seed : null,
      send: send && seed.length > 0,
    };
  }

  /** Rewrite the CURRENT request without bumping the nonce: Chat calls it once
   *  it has consumed a request (a sent prefill, or a new chat that now has an
   *  id), so a later remount replays "open this thread", never a second send. */
  settleOpen(conversationId: string | null): void {
    this.pendingOpen = {
      id: conversationId,
      nonce: this.pendingOpen.nonce,
      prefill: null,
      send: false,
    };
  }

  /** Idempotent startup: wire the `conversation_changed` refresh listener (kept
   *  for the app session — the singleton outlives any one surface) and run the
   *  first history fetch. Multiple callers may call this; only the first works. */
  async ensureStarted(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    // The singleton lives for the whole app session, so this listener is never
    // torn down.
    void listen("conversation_changed", () => void this.refreshHistory());
    await this.refreshHistory();
  }
}

export const conversationStore = new ConversationStore();
