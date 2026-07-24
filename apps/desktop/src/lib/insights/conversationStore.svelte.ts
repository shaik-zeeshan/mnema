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

const SEARCH_DEBOUNCE_MS = 220;

// ── Date grouping (left rail) ──────────────────────────────────────────────
// The flat history list renders under quiet section headers computed from each
// conversation's last-activity timestamp (`updatedAtMs`, the same field the
// list is sorted by): Today / Yesterday / This week (the rest of the last 7
// calendar days) / earlier months ("May 2026"). Buckets are keyed by label in
// first-seen order, so the existing sort order is preserved within each group
// (and search results never produce a duplicated header).
export interface HistoryGroup {
  label: string;
  items: ConversationSummary[];
}

/** The rail's origin filter (issue #179): "chats" = anything a human started
 *  (quick_recall + chat), "triggers" = trigger runs. Applied CLIENT-SIDE over
 *  whatever the backend returned (browse list or search results), so text
 *  search keeps working inside a filtered view. */
export type OriginFilter = "all" | "chats" | "triggers";

const DAY_MS = 86_400_000;

function historyGroupLabel(ms: number, todayStartMs: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "Earlier";
  if (ms >= todayStartMs) return "Today";
  if (ms >= todayStartMs - DAY_MS) return "Yesterday";
  if (ms >= todayStartMs - 6 * DAY_MS) return "This week";
  return new Date(ms).toLocaleDateString(undefined, {
    month: "long",
    year: "numeric",
  });
}

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
  /** The debounced search query over the history list. */
  searchQuery = $state("");
  /** The rail's All / Chats / Triggers origin filter. */
  originFilter = $state<OriginFilter>("all");
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
   *  ready to review/edit and send (it does NOT auto-send). */
  pendingOpen = $state<{
    id: string | null;
    nonce: number;
    prefill: string | null;
  }>({
    id: null,
    nonce: 0,
    prefill: null,
  });

  /** `conversations` narrowed by the origin filter (search already applied by
   *  the backend fetch, so filter and search compose). */
  filteredConversations = $derived.by((): ConversationSummary[] => {
    if (this.originFilter === "all") return this.conversations;
    const wantTrigger = this.originFilter === "triggers";
    return this.conversations.filter(
      (c) => (c.origin === "trigger") === wantTrigger,
    );
  });

  /** Date-grouped view of the filtered list for the rail's section headers. */
  historyGroups = $derived.by((): HistoryGroup[] => {
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    const todayStartMs = todayStart.getTime();
    const groups: HistoryGroup[] = [];
    const byLabel = new Map<string, HistoryGroup>();
    for (const c of this.filteredConversations) {
      const label = historyGroupLabel(c.updatedAtMs, todayStartMs);
      let group = byLabel.get(label);
      if (group === undefined) {
        group = { label, items: [] };
        byLabel.set(label, group);
        groups.push(group);
      }
      group.items.push(c);
    }
    return groups;
  });

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
    } catch {
      if (generation !== this.#historyGeneration) return;
      this.conversations = [];
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

  /** Delete a conversation after a Tauri confirm. If the deleted thread is the
   *  open one, arm a fresh empty pane via the bus. The backend's
   *  `conversation_changed` event refreshes the list. */
  async deleteConversation(summary: ConversationSummary): Promise<void> {
    const ok = await confirm(
      `Delete “${summary.title || summary.preview || "this conversation"}”? This can't be undone.`,
      { title: "Delete conversation", kind: "warning" },
    );
    if (!ok) return;
    try {
      await invoke("delete_conversation", {
        conversationId: summary.conversationId,
      });
    } catch {
      // The conversation_changed listener refreshes the list regardless.
    }
    if (summary.conversationId === this.activeConversationId) {
      // The open conversation was deleted — arm a fresh empty pane.
      this.requestNewChat();
    }
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
    };
  }

  /** Ask Chat to start a fresh empty chat (id === null). An optional `prefill`
   *  seeds the composer (a Subject→Chat hand-off prefills "Ask AI about …"); the
   *  user reviews/edits and presses Enter — it is never auto-sent. */
  requestNewChat(prefill?: string): void {
    const seed = prefill?.trim() ?? "";
    this.pendingOpen = {
      id: null,
      nonce: this.pendingOpen.nonce + 1,
      prefill: seed.length > 0 ? seed : null,
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
