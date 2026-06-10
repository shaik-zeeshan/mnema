<script lang="ts">
  // Chat — the persistent, searchable conversation workspace of the Insights
  // surface (issue #110, ADR 0031). Chat is a two-pane workspace whose
  // conversations are persisted to the shared conversation store (#102) and
  // answered by the SAME Ask AI engine, which reaches capture data ONLY through
  // the brokered tools. Quick Recall now persists to that SAME store too (issue
  // #111), so a launcher thread can be opened/continued here (the "Continue in
  // Chat" hand-off) under the same conversationId. Answers can additionally
  // render inline charts / dossier cards (acceptance #2) via the mnema-bars /
  // mnema-dossier fenced-block post-processor below.
  //
  // Layout:
  //   LEFT rail   — "+ New chat", a debounced search over conversations, and the
  //                 newest-first history list (select → load via get_conversation,
  //                 per-item delete with a Tauri confirm).
  //   RIGHT pane  — the active conversation: a vertically-scrolling transcript
  //                 (ONLY the transcript scrolls, not the page) rendered as a
  //                 centered column where the user's question is a right-aligned
  //                 bubble and the engine's answer is left-aligned (with inline
  //                 charts / dossier cards + Answer Sources), and a bottom
  //                 composer (Enter sends, Shift+Enter newlines). When the engine
  //                 is off the composer is replaced by a quiet "enable" card.
  //
  // Persistence is two writes per turn (start: phase "streaming" + question;
  // finish: phase "done"/"error" + answer/sources/toolActivities), never per
  // delta. A conversation loaded from history with no live PI session RESURRECTS
  // on the next question (ask_ai_start carrying priorTranscript), keeping the
  // same conversationId so it persists back into the same stored thread.
  import { onMount, onDestroy, tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { openSettingsWindow } from "$lib/surface-windows";
  import { renderMarkdown } from "$lib/markdown";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import {
    type ConversationSummary,
    type Conversation,
    type ConversationTurn,
    type SaveConversationTurnRequest,
    type AskAiAvailability,
    type AskAiStatusEvent,
    type AskAiDeltaEvent,
    type AskAiDoneEvent,
    type AskAiErrorEvent,
    type AskAiSourceEvent,
    type AskAiSource,
    type AskToolKind,
    type AskToolActivityEntry,
  } from "$lib/insights/conversation";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";

  // Quick Recall → Chat handoff (issue #111, ADR 0031). When the Insights page
  // receives an `insights_open_conversation` signal it sets `openConversationId`
  // (the handed-off thread) and bumps `openConversationNonce` so the SAME id
  // handed off twice still re-triggers the load. We select + load that
  // conversation via the normal `get_conversation` path; because Quick Recall
  // persisted it under the same id, the thread continues seamlessly (Chat's
  // existing resurrect-from-priorTranscript path answers the next question).
  let {
    openConversationId = null,
    openConversationNonce = 0,
  }: {
    openConversationId?: string | null;
    openConversationNonce?: number;
  } = $props();

  const SEARCH_DEBOUNCE_MS = 220;
  const TITLE_MAX = 80;

  // ── Ask AI availability (engine-off gate) ────────────────────────────────
  let askAvailability = $state<AskAiAvailability | null>(null);
  let askAvailable = $derived(askAvailability?.available === true);

  async function loadAskAvailability(): Promise<void> {
    try {
      askAvailability = await invoke<AskAiAvailability>("ask_ai_availability");
    } catch (error) {
      askAvailability = {
        available: false,
        reason: error instanceof Error ? error.message : String(error),
      };
    }
  }

  function enableEngine(): void {
    void openSettingsWindow("intelligence");
  }

  // ── History list (left rail) ─────────────────────────────────────────────
  let conversations = $state<ConversationSummary[]>([]);
  let historyLoaded = $state(false);
  let searchQuery = $state("");
  let searchDebounce: ReturnType<typeof setTimeout> | null = null;
  // Generation token so a stale (out-of-order) history/search response is dropped.
  let historyGeneration = 0;

  async function refreshHistory(): Promise<void> {
    historyGeneration += 1;
    const generation = historyGeneration;
    const trimmed = searchQuery.trim();
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
      if (generation !== historyGeneration) return;
      conversations = rows;
    } catch {
      if (generation !== historyGeneration) return;
      conversations = [];
    } finally {
      if (generation === historyGeneration) historyLoaded = true;
    }
  }

  function onSearchInput(): void {
    if (searchDebounce !== null) clearTimeout(searchDebounce);
    searchDebounce = setTimeout(() => {
      searchDebounce = null;
      void refreshHistory();
    }, SEARCH_DEBOUNCE_MS);
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

  // ── Active conversation (right pane) ─────────────────────────────────────
  // One assistant turn in the transcript. Mirrors Quick Recall's AskTurn.
  interface ChatTurn {
    question: string;
    answer: string;
    toolActivities: AskToolActivityEntry[];
    // Live working-line label for the in-flight tool call (cleared on resume).
    toolActivity: string | null;
    sources: AskAiSource[];
    phase: "seeding" | "thinking" | "streaming" | "done" | "error";
    errorMessage: string | null;
    seededResultCount: number | null;
    summaryExpanded: boolean;
  }

  // The active thread id (one live PI session). null when no conversation open.
  let activeConversationId = $state<string | null>(null);
  let activeTitle = $state<string>("");
  // Whether the active thread has a LIVE PI session (a fresh start/followup this
  // mount). A conversation loaded from history starts with no live session and
  // resurrects on the next question.
  let hasLiveSession = $state(false);
  let turns = $state<ChatTurn[]>([]);
  let loadingConversation = $state(false);
  // True between a turn starting and that turn's terminal done/error event.
  let streaming = $state(false);

  // Per-frame thumbnail cache for Answer Source cards (best-effort).
  let thumbnailCache = $state(new Map<number, string>());

  let composerInput = $state("");
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let transcriptEl = $state<HTMLDivElement | null>(null);

  function makeTurn(question: string, phase: ChatTurn["phase"]): ChatTurn {
    return {
      question,
      answer: "",
      toolActivities: [],
      toolActivity: null,
      sources: [],
      phase,
      errorMessage: null,
      seededResultCount: null,
      summaryExpanded: false,
    };
  }

  // Render caches live OUTSIDE the reactive `turns` $state so the template-time
  // memoization below never writes to a $state proxy (which Svelte 5 forbids
  // mid-render: `state_unsafe_mutation`). Keyed by the turn object (stable proxy
  // identity); entries are reclaimed when a turn is dropped. Reading `turn.answer`
  // inside the render functions still tracks reactivity, so a streamed delta or a
  // freshly-loaded transcript re-renders correctly — only the cache itself is
  // non-reactive.
  const plainRenderCache = new WeakMap<
    ChatTurn,
    { answer: string; html: string }
  >();
  const segmentRenderCache = new WeakMap<
    ChatTurn,
    { answer: string; segments: AnswerSegment[] }
  >();

  // Trim/truncate the first question into a conversation title.
  function titleFromQuestion(question: string): string {
    const t = question.trim().replace(/\s+/g, " ");
    return t.length > TITLE_MAX ? `${t.slice(0, TITLE_MAX - 1)}…` : t;
  }

  // ── New chat / select / delete ───────────────────────────────────────────
  function startNewChat(): void {
    // A brand-new thread is created lazily on the first turn (save upserts the
    // row), so here we just clear the right pane and arm a fresh id.
    activeConversationId = crypto.randomUUID();
    activeTitle = "";
    hasLiveSession = false;
    turns = [];
    streaming = false;
    composerInput = "";
    void tick().then(() => composerEl?.focus());
  }

  async function selectConversation(summary: ConversationSummary): Promise<void> {
    if (summary.conversationId === activeConversationId && turns.length > 0) {
      return;
    }
    // Loading a thread from history means there is no live PI session for it —
    // the next question resurrects via ask_ai_start + priorTranscript.
    loadingConversation = true;
    activeConversationId = summary.conversationId;
    activeTitle = summary.title;
    hasLiveSession = false;
    turns = [];
    streaming = false;
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId: summary.conversationId,
      });
      if (convo === null || activeConversationId !== summary.conversationId) {
        return;
      }
      activeTitle = convo.title;
      turns = convo.turns.map(hydrateTurn);
      // Warm thumbnails for any persisted frame sources.
      for (const t of turns) void loadSourceThumbnails(t.sources);
      await tick();
      scrollTranscriptToBottom();
    } catch {
      // Best-effort: leave the pane empty on a load failure.
    } finally {
      if (activeConversationId === summary.conversationId) {
        loadingConversation = false;
      }
    }
  }

  // Load + select a conversation by id (Quick Recall → Chat handoff, #111). The
  // id may not be in the (capped/filtered) left-rail list, so we go straight to
  // get_conversation rather than requiring a ConversationSummary. Mirrors
  // selectConversation's no-live-session semantics: the next question resurrects
  // from priorTranscript, keeping the same conversationId. A teardown of any
  // current live session happens first so we never orphan a resident PI helper.
  async function loadConversationById(conversationId: string): Promise<void> {
    const id = conversationId.trim();
    if (id.length === 0) return;
    // Already on this thread with its transcript loaded — nothing to do.
    if (id === activeConversationId && turns.length > 0) return;
    await cancelLiveSession();
    loadingConversation = true;
    activeConversationId = id;
    activeTitle = "";
    hasLiveSession = false;
    turns = [];
    streaming = false;
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId: id,
      });
      if (activeConversationId !== id) return;
      if (convo === null) {
        // No stored thread for this id yet (e.g. the streaming-phase write hasn't
        // landed). Arm the id so the next question still threads into it.
        return;
      }
      activeTitle = convo.title;
      turns = convo.turns.map(hydrateTurn);
      for (const t of turns) void loadSourceThumbnails(t.sources);
      await tick();
      scrollTranscriptToBottom();
    } catch {
      // Best-effort: leave the pane on the armed (empty) thread on a load failure.
    } finally {
      if (activeConversationId === id) loadingConversation = false;
    }
  }

  // React to a handoff from Insights (the prop + nonce bump). Reads the nonce so
  // the SAME conversation handed off twice in a row still re-loads.
  $effect(() => {
    const id = openConversationId;
    // Touch the nonce so a repeat handoff of the same id re-runs this effect.
    void openConversationNonce;
    if (id === null || id.trim().length === 0) return;
    void untrack(() => loadConversationById(id));
  });

  // Hydrate a persisted ConversationTurn into a ChatTurn (the opaque JSON
  // tool-activities / sources are narrowed defensively).
  function hydrateTurn(turn: ConversationTurn): ChatTurn {
    const t = makeTurn(turn.question, normalizePhase(turn.phase));
    t.answer = turn.answer ?? "";
    t.toolActivities = coerceToolActivities(turn.toolActivities);
    t.sources = coerceSources(turn.sources);
    t.errorMessage = turn.errorMessage;
    t.seededResultCount = turn.seededResultCount;
    return t;
  }

  function normalizePhase(phase: string): ChatTurn["phase"] {
    return phase === "done" || phase === "error" || phase === "streaming"
      ? phase
      : "done";
  }

  function coerceToolActivities(value: unknown): AskToolActivityEntry[] {
    if (!Array.isArray(value)) return [];
    return value
      .filter((e): e is AskToolActivityEntry => {
        return (
          typeof e === "object" &&
          e !== null &&
          typeof (e as { label?: unknown }).label === "string"
        );
      })
      .map((e) => ({
        kind: (["search", "timeline", "show_text", "other"].includes(
          (e as AskToolActivityEntry).kind,
        )
          ? (e as AskToolActivityEntry).kind
          : "other") as AskToolKind,
        label: (e as AskToolActivityEntry).label,
      }));
  }

  function coerceSources(value: unknown): AskAiSource[] {
    if (!Array.isArray(value)) return [];
    return value.filter((s): s is AskAiSource => {
      return (
        typeof s === "object" &&
        s !== null &&
        ((s as AskAiSource).kind === "frame" || (s as AskAiSource).kind === "audio")
      );
    });
  }

  async function deleteConversationRow(
    summary: ConversationSummary,
    event: MouseEvent,
  ): Promise<void> {
    event.stopPropagation();
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
    if (summary.conversationId === activeConversationId) {
      // The open conversation was deleted — reset the right pane to empty.
      activeConversationId = null;
      activeTitle = "";
      hasLiveSession = false;
      turns = [];
      streaming = false;
    }
  }

  // ── Persistence (two writes per turn) ────────────────────────────────────
  async function saveTurn(
    conversationId: string,
    turnIndex: number,
    turn: ChatTurn,
  ): Promise<void> {
    const request: SaveConversationTurnRequest = {
      conversationId,
      title: activeTitle || titleFromQuestion(turn.question),
      origin: "chat",
      turnIndex,
      question: turn.question,
      answer: turn.answer,
      toolActivities: turn.toolActivities,
      sources: turn.sources,
      phase: turn.phase,
      errorMessage: turn.errorMessage,
      seededResultCount: turn.seededResultCount,
    };
    try {
      await invoke("save_conversation_turn", { request });
    } catch {
      // Best-effort persistence; the in-memory transcript stays authoritative.
    }
  }

  // ── Sending a question ───────────────────────────────────────────────────
  async function send(): Promise<void> {
    const question = composerInput.trim();
    if (question.length === 0 || streaming || !askAvailable) return;

    // Lazily arm a conversation id if the pane is empty (e.g. first ever visit).
    if (activeConversationId === null) {
      activeConversationId = crypto.randomUUID();
      hasLiveSession = false;
    }
    const conversationId = activeConversationId;
    const isFirstTurn = turns.length === 0;
    if (isFirstTurn && activeTitle.length === 0) {
      activeTitle = titleFromQuestion(question);
    }

    composerInput = "";
    // Append the turn locally and persist it immediately with phase "streaming"
    // + the question, so a refresh/restart shows the in-flight question.
    const turn = makeTurn(question, isFirstTurn ? "seeding" : "thinking");
    turns = [...turns, turn];
    const turnIndex = turns.length - 1;
    streaming = true;
    await tick();
    scrollTranscriptToBottom();

    // The persisted streaming-phase write carries no answer yet.
    const streamingSnapshot = makeTurn(question, "streaming");
    void saveTurn(conversationId, turnIndex, streamingSnapshot);

    try {
      if (isFirstTurn || !hasLiveSession) {
        // First turn of a brand-new chat, OR continuing a conversation loaded
        // from history (no live session): start a fresh PI session. When there
        // are prior turns we resurrect by feeding them as priorTranscript; the
        // conversationId stays the same so it persists into the same thread.
        const priorTranscript = isFirstTurn ? undefined : buildPriorTranscript();
        await invoke<void>("ask_ai_start", {
          request: {
            conversationId,
            question,
            seedQuery: question,
            priorTranscript,
          },
        });
        hasLiveSession = true;
      } else {
        // Continuing a live thread — route the raw question into the session.
        await invoke<void>("ask_ai_followup", {
          request: { conversationId, question },
        });
      }
    } catch (error) {
      if (activeConversationId !== conversationId) return;
      streaming = false;
      const t = turns[turnIndex];
      if (t) {
        t.phase = "error";
        t.errorMessage = error instanceof Error ? error.message : String(error);
        void saveTurn(conversationId, turnIndex, t);
      }
    }
  }

  // Build the resurrection transcript: completed Q/A pairs only, oldest first.
  // Rust owns the 12k-char oldest-first trim (ASK_AI_PRIOR_TRANSCRIPT_CHAR_CAP).
  function buildPriorTranscript(): { question: string; answer: string }[] {
    return turns
      .filter((t) => t.phase === "done" && t.answer.trim().length > 0)
      .map((t) => ({ question: t.question, answer: t.answer }));
  }

  function onComposerKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  }

  // ── Cancel the live session (teardown / new chat) ────────────────────────
  async function cancelLiveSession(): Promise<void> {
    if (activeConversationId === null || !hasLiveSession) return;
    const conversationId = activeConversationId;
    hasLiveSession = false;
    streaming = false;
    try {
      await invoke<void>("ask_ai_cancel", { request: { conversationId } });
    } catch {
      // Best-effort: the helper may already be gone.
    }
  }

  // ── Scroll helper ────────────────────────────────────────────────────────
  function scrollTranscriptToBottom(): void {
    const el = transcriptEl;
    if (el === null) return;
    el.scrollTop = el.scrollHeight;
  }

  // ── Answer rendering: markdown + inline graphical segments ───────────────
  // A turn's answer is split into typed segments: markdown HTML spans, and
  // recognized fenced blocks (```mnema-bars / ```mnema-dossier) parsed to chart
  // data. While streaming we render plain markdown (partial JSON shouldn't be
  // parsed); once the turn is "done" we upgrade to graphical segments. A parse
  // failure or unknown block degrades gracefully to a normal code block.
  interface BarsItem {
    label: string;
    value: number;
    sublabel?: string;
  }
  interface DossierItem {
    subject: string | null;
    statement: string;
    confidence: number;
  }
  type AnswerSegment =
    | { kind: "html"; html: string }
    | { kind: "bars"; title: string | null; items: BarsItem[] }
    | { kind: "dossier"; items: DossierItem[] };

  // A fenced block of the form ```mnema-bars\n{...}\n``` (or mnema-dossier).
  // Captured greedily-but-bounded; the JSON body is whatever sits between the
  // fences. We only special-case these two info strings; everything else stays
  // ordinary markdown.
  const GRAPHICAL_FENCE = /```mnema-(bars|dossier)[^\n]*\n([\s\S]*?)```/g;

  function parseBarsBlock(body: string): AnswerSegment | null {
    try {
      const data = JSON.parse(body) as {
        title?: unknown;
        bars?: unknown;
      };
      const rawBars = Array.isArray(data.bars) ? data.bars : null;
      if (rawBars === null) return null;
      const items = rawBars
        .map((b): BarsItem | null => {
          if (typeof b !== "object" || b === null) return null;
          const rec = b as { label?: unknown; value?: unknown; sublabel?: unknown };
          const label = typeof rec.label === "string" ? rec.label : null;
          const value = typeof rec.value === "number" ? rec.value : Number(rec.value);
          if (label === null || !Number.isFinite(value)) return null;
          const sublabel = typeof rec.sublabel === "string" ? rec.sublabel : undefined;
          return { label, value, sublabel };
        })
        .filter((x): x is BarsItem => x !== null);
      if (items.length === 0) return null;
      const title = typeof data.title === "string" ? data.title : null;
      return { kind: "bars", title, items };
    } catch {
      return null;
    }
  }

  function parseDossierBlock(body: string): AnswerSegment | null {
    try {
      const data = JSON.parse(body) as { items?: unknown };
      const rawItems = Array.isArray(data.items) ? data.items : null;
      if (rawItems === null) return null;
      const items = rawItems
        .map((it): DossierItem | null => {
          if (typeof it !== "object" || it === null) return null;
          const rec = it as { subject?: unknown; statement?: unknown; confidence?: unknown };
          const statement = typeof rec.statement === "string" ? rec.statement : null;
          if (statement === null || statement.trim().length === 0) return null;
          const subject = typeof rec.subject === "string" ? rec.subject : null;
          const confidenceRaw =
            typeof rec.confidence === "number" ? rec.confidence : Number(rec.confidence);
          const confidence = Number.isFinite(confidenceRaw)
            ? Math.max(0, Math.min(1, confidenceRaw))
            : 0;
          return { subject, statement, confidence };
        })
        .filter((x): x is DossierItem => x !== null);
      if (items.length === 0) return null;
      return { kind: "dossier", items };
    } catch {
      return null;
    }
  }

  // Split the answer into html / graphical segments. Markdown between/around the
  // recognized fences is rendered with renderMarkdown; a recognized fence with a
  // valid body becomes a chart segment, an invalid one falls back to plain
  // markdown (so the original fenced block renders as a code block — never crash).
  function buildSegments(answer: string): AnswerSegment[] {
    const segments: AnswerSegment[] = [];
    let lastIndex = 0;
    // Reset the shared regex's lastIndex (it is global/stateful).
    GRAPHICAL_FENCE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = GRAPHICAL_FENCE.exec(answer)) !== null) {
      const [full, variant, body] = match;
      const parsed =
        variant === "bars" ? parseBarsBlock(body) : parseDossierBlock(body);
      // Flush the markdown before this fence. If the fence fails to parse we
      // include its raw text in the markdown run so it renders as a code block.
      const preEnd = parsed !== null ? match.index : match.index + full.length;
      const pre = answer.slice(lastIndex, preEnd);
      if (pre.trim().length > 0) {
        segments.push({ kind: "html", html: renderMarkdown(pre) });
      }
      if (parsed !== null) {
        segments.push(parsed);
      }
      lastIndex = match.index + full.length;
    }
    const tail = answer.slice(lastIndex);
    if (tail.trim().length > 0) {
      segments.push({ kind: "html", html: renderMarkdown(tail) });
    }
    // An all-whitespace answer (or one fully consumed by an unparsed fence with
    // no surrounding text) yields no segments; render the raw markdown as a
    // safety net so nothing silently disappears.
    if (segments.length === 0 && answer.trim().length > 0) {
      segments.push({ kind: "html", html: renderMarkdown(answer) });
    }
    return segments;
  }

  // Render one turn's answer. While streaming (or seeding/thinking) we serve
  // plain markdown so partial JSON is never parsed; once "done" we upgrade to
  // graphical segments. Both caches are memoized (in non-reactive WeakMaps) so a
  // streamed delta re-renders only the live turn (O(n) over the stream).
  function renderPlainAnswer(turn: ChatTurn): string {
    const cached = plainRenderCache.get(turn);
    if (cached !== undefined && cached.answer === turn.answer) return cached.html;
    const html = turn.answer.length > 0 ? renderMarkdown(turn.answer) : "";
    plainRenderCache.set(turn, { answer: turn.answer, html });
    return html;
  }

  function answerSegments(turn: ChatTurn): AnswerSegment[] {
    const cached = segmentRenderCache.get(turn);
    if (cached !== undefined && cached.answer === turn.answer) {
      return cached.segments;
    }
    const segments = buildSegments(turn.answer);
    segmentRenderCache.set(turn, { answer: turn.answer, segments });
    return segments;
  }

  // ── Tool-activity formatting (mirrors Quick Recall's pure helpers) ────────
  function readString(
    params: Record<string, unknown>,
    key: string,
  ): string | null {
    const value = params[key];
    return typeof value === "string" && value.trim().length > 0
      ? value.trim()
      : null;
  }

  function formatToolActivity(
    tool: string | undefined,
    params: Record<string, unknown> | undefined,
  ): AskToolActivityEntry {
    const p = params ?? {};
    if (tool === "search") {
      const queryText = readString(p, "query");
      let label = queryText
        ? `Searching “${queryText}”`
        : "Searching your captures";
      const app = readString(p, "app");
      if (app) label += ` in ${app}`;
      return { kind: "search", label };
    }
    if (tool === "timeline") {
      let label = "Scanning timeline";
      const app = readString(p, "app");
      if (app) label += ` in ${app}`;
      return { kind: "timeline", label };
    }
    if (tool === "show_text") {
      return { kind: "show_text", label: "Reading a capture" };
    }
    return { kind: "other", label: tool ? `Running ${tool}` : "Working" };
  }

  function activitySummaryFor(
    toolActivities: AskToolActivityEntry[],
  ): string | null {
    if (toolActivities.length === 0) return null;
    let searches = 0;
    let timelines = 0;
    let reads = 0;
    let others = 0;
    for (const entry of toolActivities) {
      if (entry.kind === "search") searches += 1;
      else if (entry.kind === "timeline") timelines += 1;
      else if (entry.kind === "show_text") reads += 1;
      else others += 1;
    }
    const parts: string[] = [];
    if (searches > 0)
      parts.push(`${searches} ${searches === 1 ? "search" : "searches"}`);
    if (timelines > 0)
      parts.push(
        `${timelines} ${timelines === 1 ? "timeline scan" : "timeline scans"}`,
      );
    if (reads > 0) parts.push(`${reads} ${reads === 1 ? "read" : "reads"}`);
    if (others > 0) parts.push(`${others} ${others === 1 ? "step" : "steps"}`);
    return parts.length > 0 ? parts.join(" · ") : null;
  }

  function toggleSummary(turn: ChatTurn): void {
    turn.summaryExpanded = !turn.summaryExpanded;
  }

  // ── Answer Sources (mirrors Quick Recall) ────────────────────────────────
  function turnFrameSources(turn: ChatTurn): AskAiSource[] {
    return turn.sources.filter((s) => s.kind === "frame");
  }
  function turnAudioSources(turn: ChatTurn): AskAiSource[] {
    return turn.sources.filter((s) => s.kind === "audio");
  }

  async function loadSourceThumbnails(sources: AskAiSource[]): Promise<void> {
    const frameIds = sources
      .filter((s) => s.kind === "frame" && s.frameId != null)
      .map((s) => s.frameId as number)
      .filter((id) => !thumbnailCache.has(id));
    const uniqueIds = Array.from(new Set(frameIds));
    if (uniqueIds.length === 0) return;
    try {
      const response = await invoke<FrameScrubPreviewsDto>(
        "get_frame_scrub_previews",
        { request: { frameIds: uniqueIds } },
      );
      const next = new Map(thumbnailCache);
      for (const entry of response.previews) {
        if (entry.preview) {
          next.set(entry.frameId, framePreviewAssetUrl(entry.preview.filePath));
        }
      }
      thumbnailCache = next;
    } catch {
      // Thumbnails are best-effort; the card falls back to its glyph.
    }
  }

  // Hand off an Answer Source to the main timeline window (frame xor audio).
  async function selectSource(source: AskAiSource): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: source.kind,
        frameId: source.frameId,
        audioSegmentId: source.audioSegmentId,
        spanStartMs: source.spanStartMs ?? null,
        alignedFrameId: source.alignedFrameId ?? null,
      });
    } catch {
      // Best-effort hand-off.
    }
  }

  // Route link clicks inside a rendered answer through the OS browser.
  async function handleAnswerClick(event: MouseEvent): Promise<void> {
    const anchor = (event.target as HTMLElement | null)?.closest(
      "a[data-external]",
    ) as HTMLAnchorElement | null;
    if (anchor === null) return;
    event.preventDefault();
    const href = anchor.getAttribute("href");
    if (href !== null && href.length > 0) {
      void openUrl(href);
    }
  }

  // ── Stream event wiring ──────────────────────────────────────────────────
  onMount(() => {
    void loadAskAvailability();
    void refreshHistory();

    let destroyed = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenDelta: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenSource: (() => void) | undefined;
    let unlistenChanged: (() => void) | undefined;
    let unlistenCtx: (() => void) | undefined;

    // All stream events route to the LAST (live) turn, guarded on a matching
    // thread id (stale-thread guard, REQUIRED) and a non-empty transcript.
    listen<AskAiStatusEvent>("ask_ai_status", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turn = turns[turns.length - 1];
      if (!turn) return;
      if (event.payload.phase === "tool") {
        const activity = formatToolActivity(
          event.payload.tool,
          event.payload.params,
        );
        turn.toolActivity = activity.label;
        turn.toolActivities = [...turn.toolActivities, activity];
        return;
      }
      if (typeof event.payload.seededResultCount === "number") {
        turn.seededResultCount = event.payload.seededResultCount;
      }
      if (turn.phase !== "streaming") {
        turn.phase = event.payload.phase;
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenStatus = fn;
    });

    listen<AskAiDeltaEvent>("ask_ai_delta", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      if (!streaming) return;
      const turn = turns[turns.length - 1];
      if (!turn) return;
      turn.toolActivity = null;
      turn.phase = "streaming";
      turn.answer += event.payload.text;
      void tick().then(scrollTranscriptToBottom);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDelta = fn;
    });

    listen<AskAiDoneEvent>("ask_ai_done", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turnIndex = turns.length - 1;
      const turn = turns[turnIndex];
      if (!turn) return;
      streaming = false;
      turn.toolActivity = null;
      turn.phase = "done";
      // Finish write: final answer + sources + tool activities, phase "done".
      if (activeConversationId !== null) {
        void saveTurn(activeConversationId, turnIndex, turn);
      }
      void tick().then(scrollTranscriptToBottom);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDone = fn;
    });

    listen<AskAiErrorEvent>("ask_ai_error", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turnIndex = turns.length - 1;
      const turn = turns[turnIndex];
      if (!turn) return;
      streaming = false;
      // The error killed the live PI session Rust-side; the next question must
      // resurrect rather than follow up.
      hasLiveSession = false;
      turn.toolActivity = null;
      turn.phase = "error";
      turn.errorMessage = event.payload.message;
      if (activeConversationId !== null) {
        void saveTurn(activeConversationId, turnIndex, turn);
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenError = fn;
    });

    listen<AskAiSourceEvent>("ask_ai_source", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turn = turns[turns.length - 1];
      if (!turn) return;
      if (event.payload.sources.length > 0) {
        turn.sources = event.payload.sources;
        void loadSourceThumbnails(event.payload.sources);
      }
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenSource = fn;
    });

    // Refresh the history list whenever any conversation surface saves/deletes.
    listen("conversation_changed", () => {
      void refreshHistory();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenChanged = fn;
    });

    // Re-probe Ask AI availability when the engine config may have changed.
    listen("user_context_changed", () => {
      void loadAskAvailability();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenCtx = fn;
    });

    return () => {
      destroyed = true;
      unlistenStatus?.();
      unlistenDelta?.();
      unlistenDone?.();
      unlistenError?.();
      unlistenSource?.();
      unlistenChanged?.();
      unlistenCtx?.();
    };
  });

  onDestroy(() => {
    if (searchDebounce !== null) clearTimeout(searchDebounce);
    // Never leave a resident PI session outliving the surface.
    void cancelLiveSession();
  });
</script>

<section class="chat" aria-label="Chat">
  <!-- LEFT rail: new chat + search + history list -->
  <aside class="rail" aria-label="Conversations">
    <div class="rail-top">
      <button type="button" class="new-chat" onclick={startNewChat}>
        <span class="plus" aria-hidden="true">＋</span> New chat
      </button>
      <div class="search">
        <span class="search-glyph" aria-hidden="true">⌕</span>
        <input
          type="search"
          class="search-input"
          placeholder="Search conversations…"
          aria-label="Search conversations"
          autocomplete="off"
          spellcheck="false"
          bind:value={searchQuery}
          oninput={onSearchInput}
        />
      </div>
    </div>

    <div class="history" role="list" aria-label="Conversation history">
      {#if !historyLoaded}
        <div class="history-skeleton">
          {#each Array(5) as _, i (i)}
            <div class="sk-row">
              <Skeleton width="70%" height="11px" radius="5px" />
              <Skeleton width="40%" height="9px" radius="5px" muted />
            </div>
          {/each}
        </div>
      {:else if conversations.length === 0}
        <p class="rail-empty">
          {searchQuery.trim().length > 0
            ? "No conversations match."
            : "No conversations yet. Start one →"}
        </p>
      {:else}
        {#each conversations as c (c.conversationId)}
          <div
            class="history-item"
            class:active={c.conversationId === activeConversationId}
            role="listitem"
          >
            <button
              type="button"
              class="history-open"
              onclick={() => void selectConversation(c)}
              aria-current={c.conversationId === activeConversationId
                ? "true"
                : undefined}
            >
              <span class="history-title" title={c.title || c.preview}>
                {c.title || c.preview || "Untitled chat"}
              </span>
              <span class="history-meta">
                <span class="history-time">{relativeTime(c.updatedAtMs)}</span>
                <span class="history-dot" aria-hidden="true">·</span>
                <span class="history-count">
                  {c.turnCount}
                  {c.turnCount === 1 ? "turn" : "turns"}
                </span>
              </span>
            </button>
            <button
              type="button"
              class="history-delete"
              aria-label="Delete conversation"
              title="Delete conversation"
              onclick={(e) => void deleteConversationRow(c, e)}
            >
              ✕
            </button>
          </div>
        {/each}
      {/if}
    </div>
  </aside>

  <!-- RIGHT pane: active conversation transcript + composer -->
  <div class="pane">
    {#if activeConversationId === null}
      <div class="pane-empty">
        <p class="pane-empty-title">Ask the engine about your activity</p>
        <p class="pane-empty-detail">
          Pick a conversation on the left, or start a new chat. Answers draw on
          your history through the engine's brokered tools and can include inline
          charts.
        </p>
        {#if askAvailable}
          <button type="button" class="btn btn--accent" onclick={startNewChat}>
            ＋ New chat
          </button>
        {/if}
      </div>
    {:else}
      <!-- ONLY this transcript scrolls (not the page). -->
      <div class="transcript" bind:this={transcriptEl} aria-live="polite">
        <!-- Centered conversation column: user question right, AI answer left. -->
        <div class="thread-col">
          {#if loadingConversation}
            <div class="convo-skeleton">
              <Skeleton width="55%" height="13px" radius="6px" />
              <Skeleton width="100%" height="48px" radius="8px" muted />
            </div>
          {:else if turns.length === 0}
            <div class="thread-empty">
              <p class="thread-empty-title">{activeTitle || "New chat"}</p>
              <p class="thread-empty-detail">
                Type a question below and press Enter. The engine searches your
                captures through its brokered tools to answer.
              </p>
            </div>
          {:else}
            {#each turns as turn, ti (ti)}
              <article class="turn">
                <!-- USER question: right-aligned bubble -->
                <div class="msg msg-user">
                  <div class="user-bubble" title={turn.question}>
                    {turn.question}
                  </div>
                </div>

                <!-- ASSISTANT answer: left-aligned -->
                <div class="msg msg-assistant">
                  <div class="answer-col">
                    {#if turn.phase === "error"}
                      <p class="state state--error">
                        {turn.errorMessage ?? "The engine couldn't answer."}
                      </p>
                    {:else}
                      {#if turn.seededResultCount !== null && turn.seededResultCount > 0}
                        <p class="seeded">
                          Seeded with {turn.seededResultCount}
                          {turn.seededResultCount === 1 ? "result" : "results"}
                        </p>
                      {/if}

                      {#if turn.phase === "seeding"}
                        <p class="state state--working">
                          <span class="dot" aria-hidden="true"></span>
                          Searching your captures…
                        </p>
                      {:else if turn.phase === "thinking" && turn.toolActivity === null}
                        <p class="state state--working">
                          <span class="dot" aria-hidden="true"></span>
                          Thinking…
                        </p>
                      {:else}
                        {#if turn.phase === "streaming" || turn.phase === "done"}
                          <!-- Collapsed, expandable tool-activity summary chip. -->
                          {#if activitySummaryFor(turn.toolActivities) !== null}
                            <div class="activity">
                              <button
                                type="button"
                                class="activity-chip"
                                aria-expanded={turn.summaryExpanded}
                                onclick={() => toggleSummary(turn)}
                              >
                                <span
                                  class="activity-caret"
                                  class:open={turn.summaryExpanded}
                                  aria-hidden="true">▸</span
                                >
                                <span class="activity-summary"
                                  >{activitySummaryFor(turn.toolActivities)}</span
                                >
                              </button>
                              {#if turn.summaryExpanded}
                                <ul class="activity-list">
                                  {#each turn.toolActivities as activity, ai (ai)}
                                    <li class="activity-item">{activity.label}</li>
                                  {/each}
                                </ul>
                              {/if}
                            </div>
                          {/if}

                          <!-- The answer body. While streaming we render plain
                               markdown; once done we upgrade to graphical segments
                               (mnema-bars / mnema-dossier). -->
                          <!-- svelte-ignore a11y_no_static_element_interactions -->
                          <!-- svelte-ignore a11y_click_events_have_key_events -->
                          <div
                            class="answer"
                            class:answer--streaming={turn.phase === "streaming"}
                            onclick={handleAnswerClick}
                          >
                            {#if turn.phase === "done"}
                              {#each answerSegments(turn) as seg, si (si)}
                                {#if seg.kind === "html"}
                                  <div class="answer-md">{@html seg.html}</div>
                                {:else if seg.kind === "bars"}
                                  <figure class="graphic">
                                    {#if seg.title}
                                      <figcaption class="graphic-title">
                                        {seg.title}
                                      </figcaption>
                                    {/if}
                                    <MiniBars items={seg.items} />
                                  </figure>
                                {:else if seg.kind === "dossier"}
                                  <div class="graphic graphic--dossier">
                                    {#each seg.items as item, di (di)}
                                      <div class="dossier-card">
                                        <p class="dossier-statement">
                                          {item.statement}
                                        </p>
                                        <div class="dossier-foot">
                                          {#if item.subject}
                                            <span class="subject-chip">
                                              {item.subject}
                                            </span>
                                          {/if}
                                          <span class="conf-wrap">
                                            <ConfidenceBar confidence={item.confidence} />
                                          </span>
                                        </div>
                                      </div>
                                    {/each}
                                  </div>
                                {/if}
                              {/each}
                            {:else}
                              <div class="answer-md">{@html renderPlainAnswer(turn)}</div>
                            {/if}
                          </div>

                          <!-- Answer Sources: the captures this turn drew on. -->
                          {#if turn.phase === "done" && turn.sources.length > 0}
                            <div class="sources">
                              <span class="sources-heading">Sources</span>
                              {#if turnFrameSources(turn).length > 0}
                                <div class="source-section" role="presentation">
                                  <span class="source-label">Screen</span>
                                  <div class="source-row" role="presentation">
                                    {#each turnFrameSources(turn) as s, si (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${si}`)}
                                      <AnswerSourceCard
                                        kind="frame"
                                        appName={s.appName}
                                        windowTitle={s.windowTitle}
                                        startedAt={s.startedAt}
                                        endedAt={s.endedAt}
                                        thumbnailUrl={s.frameId != null
                                          ? (thumbnailCache.get(s.frameId) ?? null)
                                          : null}
                                        onselect={() => void selectSource(s)}
                                      />
                                    {/each}
                                  </div>
                                </div>
                              {/if}
                              {#if turnAudioSources(turn).length > 0}
                                <div class="source-section" role="presentation">
                                  <span class="source-label">Audio</span>
                                  <div class="source-row" role="presentation">
                                    {#each turnAudioSources(turn) as s, si (`${s.kind}-${s.frameId}-${s.audioSegmentId}-${s.startedAt}-${si}`)}
                                      <AnswerSourceCard
                                        kind="audio"
                                        appName={s.appName}
                                        windowTitle={s.windowTitle}
                                        startedAt={s.startedAt}
                                        endedAt={s.endedAt}
                                        sourceKind={s.sourceKind}
                                        onselect={() => void selectSource(s)}
                                      />
                                    {/each}
                                  </div>
                                </div>
                              {/if}
                            </div>
                          {/if}
                        {/if}

                        {#if turn.toolActivity !== null}
                          <p class="state state--working">
                            <span class="dot" aria-hidden="true"></span>
                            <span class="working-label">{turn.toolActivity}</span>
                          </p>
                        {/if}
                      {/if}
                    {/if}
                  </div>
                </div>
              </article>
            {/each}
          {/if}
        </div>
      </div>

      <!-- Composer (engine-on) or quiet enable card (engine-off). -->
      {#if askAvailable}
        <div class="composer-wrap">
          <div class="composer">
            <textarea
              bind:this={composerEl}
              bind:value={composerInput}
              class="composer-input"
              rows="1"
              placeholder="Ask about your activity…"
              aria-label="Ask about your activity"
              disabled={streaming}
              onkeydown={onComposerKeydown}
            ></textarea>
            <button
              type="button"
              class="composer-send"
              disabled={streaming || composerInput.trim().length === 0}
              onclick={() => void send()}
              aria-label="Send"
              title="Send (Enter)"
            >
              {#if streaming}
                <span class="dot" aria-hidden="true"></span>
              {:else}
                ↑
              {/if}
            </button>
          </div>
        </div>
      {:else}
        <div class="composer-wrap">
          <div class="composer engine-off">
            <span class="engine-off-dot" aria-hidden="true"></span>
            <span class="engine-off-text">
              The reasoning engine is off. Chat answers over your history once
              it's enabled.
            </span>
            <button type="button" class="engine-off-enable" onclick={enableEngine}>
              Enable engine
            </button>
          </div>
        </div>
      {/if}
    {/if}
  </div>
</section>

<style>
  /* Two-pane workspace filling the insights surface. Mirrors the terminal/green
     token system (--app-* / --cat-*) used across Overview/Subjects/Context.
     The surface fills the full height and OWNS its scrolling: only `.history`
     and `.transcript` scroll — the page itself never does (the Insights shell
     drops its padding/overflow for the chat tab via `.insights-main--chat`). */
  .chat {
    display: grid;
    grid-template-columns: 260px 1fr;
    gap: 0;
    /* Fill the flex-column parent (`.insights-main--chat`) via flex-grow, NOT a
       percentage height — WKWebView (Tauri) drops `height: 100%` against a
       flex-stretched ancestor, which collapsed the surface to content height. */
    flex: 1 1 auto;
    min-height: 0;
    height: 100%;
    border-top: 1px solid var(--app-border);
    overflow: hidden;
    background: var(--app-surface);
  }

  /* ── LEFT rail ───────────────────────────────────────────────────────── */
  .rail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
  }
  .rail-top {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-bottom: 1px solid var(--app-border);
  }
  .new-chat {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    height: 30px;
    font: inherit;
    font-size: 11.5px;
    letter-spacing: 0.02em;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: border-color 0.12s ease, background 0.12s ease;
  }
  .new-chat:hover {
    border-color: var(--app-accent);
  }
  .new-chat .plus {
    font-size: 13px;
    line-height: 1;
  }

  .search {
    display: flex;
    align-items: center;
    gap: 7px;
    height: 28px;
    padding: 0 9px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface);
  }
  .search:focus-within {
    border-color: var(--app-border-hover);
  }
  .search-glyph {
    color: var(--app-text-subtle);
    font-size: 12px;
  }
  .search-input {
    flex: 1 1 auto;
    min-width: 0;
    font: inherit;
    font-size: 11.5px;
    border: none;
    background: transparent;
    color: var(--app-text);
    outline: none;
  }
  .search-input::placeholder {
    color: var(--app-text-faint);
  }
  /* Hide the native search clear affordance for a consistent terminal look. */
  .search-input::-webkit-search-cancel-button {
    -webkit-appearance: none;
    appearance: none;
  }

  .history {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .history-skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 4px;
  }
  .sk-row {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .rail-empty {
    font-size: 11px;
    color: var(--app-text-faint);
    padding: 10px 6px;
    line-height: 1.5;
  }

  .history-item {
    display: flex;
    align-items: stretch;
    border: 1px solid transparent;
    border-radius: 7px;
    transition: background 0.12s ease, border-color 0.12s ease;
  }
  .history-item:hover {
    background: var(--app-surface-hover);
  }
  .history-item.active {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
  }
  .history-open {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 8px 9px;
    border: none;
    background: transparent;
    text-align: left;
    cursor: pointer;
    font: inherit;
  }
  .history-title {
    font-size: 11.5px;
    color: var(--app-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .history-item.active .history-title {
    color: var(--app-accent-strong);
  }
  .history-meta {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 9.5px;
    color: var(--app-text-faint);
    letter-spacing: 0.02em;
  }
  .history-delete {
    flex: 0 0 auto;
    width: 26px;
    border: none;
    background: transparent;
    color: var(--app-text-faint);
    font-size: 10px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.12s ease, color 0.12s ease;
  }
  .history-item:hover .history-delete {
    opacity: 1;
  }
  .history-delete:hover {
    color: var(--app-danger);
  }

  /* ── RIGHT pane ──────────────────────────────────────────────────────── */
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--app-surface);
  }
  .pane-empty {
    margin: auto;
    max-width: 380px;
    padding: 28px;
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: center;
  }
  .pane-empty-title {
    font-size: 14px;
    font-weight: 600;
    color: var(--app-text-strong);
    letter-spacing: -0.01em;
  }
  .pane-empty-detail {
    font-size: 12px;
    color: var(--app-text-muted);
    line-height: 1.6;
  }

  /* ONLY the transcript scrolls (not the page). */
  .transcript {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 22px 0 16px;
  }
  /* Centered conversation column. */
  .thread-col {
    max-width: 760px;
    margin: 0 auto;
    padding: 0 24px;
    display: flex;
    flex-direction: column;
    gap: 24px;
  }
  .convo-skeleton {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .thread-empty {
    margin: 4px 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .thread-empty-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
  }
  .thread-empty-detail {
    font-size: 12px;
    color: var(--app-text-muted);
    line-height: 1.6;
    max-width: 460px;
  }

  /* One transcript turn: a user bubble (right) then the AI answer (left). */
  .turn {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .msg {
    display: flex;
  }
  /* User question — right-aligned bubble. */
  .msg-user {
    justify-content: flex-end;
  }
  .user-bubble {
    max-width: 80%;
    padding: 9px 13px;
    border: 1px solid var(--app-border-strong);
    border-radius: 12px 12px 4px 12px;
    background: var(--app-surface-raised);
    color: var(--app-text-strong);
    font-size: 13px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  /* AI answer — left-aligned, fills the column width. */
  .msg-assistant {
    justify-content: flex-start;
  }
  .answer-col {
    min-width: 0;
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .seeded {
    font-size: 10px;
    color: var(--app-text-faint);
    letter-spacing: 0.02em;
  }

  .state {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 12px;
    color: var(--app-text-muted);
  }
  .state--error {
    color: var(--app-danger);
  }
  .state--working {
    color: var(--app-text-muted);
  }
  .working-label {
    color: var(--app-text);
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-accent);
    box-shadow: 0 0 0 3px var(--app-accent-glow);
    animation: chat-pulse 1.1s ease-in-out infinite;
    flex: 0 0 auto;
  }
  @keyframes chat-pulse {
    0%,
    100% {
      opacity: 0.4;
    }
    50% {
      opacity: 1;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot {
      animation: none;
    }
  }

  /* Tool-activity summary chip. */
  .activity {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .activity-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    align-self: flex-start;
    font: inherit;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 3px 9px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .activity-chip:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .activity-caret {
    font-size: 8px;
    transition: transform 0.12s ease;
  }
  .activity-caret.open {
    transform: rotate(90deg);
  }
  .activity-list {
    list-style: none;
    margin: 0;
    padding: 0 0 0 14px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .activity-item {
    font-size: 10.5px;
    color: var(--app-text-muted);
    line-height: 1.5;
  }

  /* Rendered answer body. */
  .answer {
    font-size: 13px;
    color: var(--app-text);
    line-height: 1.65;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .answer--streaming::after {
    content: "▍";
    color: var(--app-accent);
    animation: chat-caret 1s step-end infinite;
  }
  @keyframes chat-caret {
    50% {
      opacity: 0;
    }
  }
  .answer-md :global(p) {
    margin: 0 0 0.6em;
  }
  .answer-md :global(p:last-child) {
    margin-bottom: 0;
  }
  .answer-md :global(ul),
  .answer-md :global(ol) {
    margin: 0 0 0.6em;
    padding-left: 1.3em;
  }
  .answer-md :global(li) {
    margin: 0.15em 0;
  }
  .answer-md :global(h1),
  .answer-md :global(h2),
  .answer-md :global(h3) {
    font-size: 13px;
    font-weight: 600;
    color: var(--app-text-strong);
    margin: 0.6em 0 0.3em;
  }
  .answer-md :global(code) {
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 11.5px;
    padding: 1px 4px;
    border-radius: 4px;
    background: var(--app-surface-hover);
    border: 1px solid var(--app-border);
  }
  .answer-md :global(pre) {
    margin: 0 0 0.6em;
    padding: 10px 12px;
    border-radius: 8px;
    background: var(--app-surface-subtle);
    border: 1px solid var(--app-border);
    overflow-x: auto;
  }
  .answer-md :global(pre code) {
    padding: 0;
    border: none;
    background: transparent;
  }
  .answer-md :global(a) {
    color: var(--app-accent-strong);
    text-decoration: underline;
    text-decoration-color: var(--app-accent-border);
  }
  .answer-md :global(blockquote) {
    margin: 0 0 0.6em;
    padding-left: 11px;
    border-left: 2px solid var(--app-border);
    color: var(--app-text-muted);
  }
  .answer-md :global(table) {
    border-collapse: collapse;
    font-size: 11.5px;
  }
  .answer-md :global(th),
  .answer-md :global(td) {
    border: 1px solid var(--app-border);
    padding: 4px 8px;
    text-align: left;
  }

  /* Inline graphical answer segments. */
  .graphic {
    margin: 0;
    padding: 12px 13px;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface-subtle);
  }
  .graphic-title {
    font-size: 10.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-muted);
    margin: 0 0 10px;
  }
  .graphic--dossier {
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .dossier-card {
    padding: 11px 12px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    display: flex;
    flex-direction: column;
    gap: 9px;
  }
  .dossier-statement {
    font-size: 12.5px;
    color: var(--app-text-strong);
    line-height: 1.5;
    margin: 0;
  }
  .dossier-foot {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .subject-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 8px;
    border-radius: 4px;
    background: var(--app-accent-bg);
    border: 1px solid var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .conf-wrap {
    flex: 0 0 auto;
  }

  /* Answer Sources strip. */
  .sources {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-top: 4px;
  }
  .sources-heading {
    font-size: 10px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
  }
  .source-section {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .source-label {
    font-size: 9.5px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .source-row {
    display: flex;
    gap: 8px;
    overflow-x: auto;
    padding-bottom: 4px;
  }

  /* Composer (pinned to the bottom of the pane; centered to the column). */
  .composer-wrap {
    flex: 0 0 auto;
    border-top: 1px solid var(--app-border);
    background: var(--app-surface-subtle);
    padding: 12px 24px 14px;
  }
  .composer {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    max-width: 760px;
    margin: 0 auto;
  }
  .composer-input {
    flex: 1 1 auto;
    min-width: 0;
    resize: none;
    font: inherit;
    font-size: 12.5px;
    line-height: 1.5;
    max-height: 140px;
    padding: 9px 11px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface);
    color: var(--app-text);
    outline: none;
    field-sizing: content;
  }
  .composer-input:focus {
    border-color: var(--app-accent-border);
  }
  .composer-input::placeholder {
    color: var(--app-text-faint);
  }
  .composer-input:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  .composer-send {
    flex: 0 0 auto;
    width: 34px;
    height: 34px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 15px;
    border: 1px solid var(--app-accent-border);
    border-radius: 8px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: border-color 0.12s ease, opacity 0.12s ease;
  }
  .composer-send:hover:not(:disabled) {
    border-color: var(--app-accent);
  }
  .composer-send:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }

  /* Engine-off quiet card (replaces the composer row, same centered width). */
  .engine-off {
    align-items: center;
    gap: 10px;
  }
  .engine-off-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--app-status-dot);
    flex: 0 0 auto;
  }
  .engine-off-text {
    flex: 1 1 auto;
    font-size: 11.5px;
    color: var(--app-text-muted);
    line-height: 1.5;
  }
  .engine-off-enable {
    flex: 0 0 auto;
    font: inherit;
    font-size: 11.5px;
    padding: 5px 11px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
    background: var(--app-accent-bg);
    color: var(--app-accent-strong);
    cursor: pointer;
    transition: border-color 0.12s ease;
  }
  .engine-off-enable:hover {
    border-color: var(--app-accent);
  }

  /* Shared accent button (mirrors Overview's .btn--accent). */
  .btn {
    font: inherit;
    font-size: 11.5px;
    padding: 7px 14px;
    border-radius: 7px;
    cursor: pointer;
    border: 1px solid transparent;
    transition: border-color 0.12s ease, background 0.12s ease;
  }
  .btn--accent {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .btn--accent:hover {
    border-color: var(--app-accent);
  }
</style>
