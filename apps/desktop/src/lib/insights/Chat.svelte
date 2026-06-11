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
  //                 newest-first history list grouped under date headers
  //                 (Today / Yesterday / This week / earlier months). Rows show
  //                 the effective title (user-set → generated → first-question
  //                 fallback, served by the backend); rename (inline, via
  //                 set_conversation_title) and delete (Tauri confirm) hide
  //                 behind hover / focus-within. Select → load via
  //                 get_conversation.
  //   RIGHT pane  — the active conversation: a vertically-scrolling transcript
  //                 (ONLY the transcript scrolls, not the page) rendered as a
  //                 centered column where the user's question is a right-aligned
  //                 bubble and the engine's answer is left-aligned (with inline
  //                 charts / dossier cards + Answer Sources), and a bottom
  //                 composer (Enter sends, Shift+Enter newlines). When the engine
  //                 is off the composer is replaced by a quiet "enable" card.
  //
  // Persistence is now owned by the Rust host: `ask_ai_start` upserts the
  // conversation row (from `title`/`origin`) and `run_ask_ai_turn` persists each
  // turn (streaming → done/error) keyed by a backend-computed turnIndex. The
  // frontend renders live from the `ask_ai_*` events and never writes a turn
  // itself. Reopening a thread HYDRATES from the store (`get_conversation`); a
  // turn still in phase "streaming" keeps streaming live via the global
  // delta/done/source listeners (they filter by conversationId). A follow-up
  // always calls `ask_ai_followup` — the backend reloads history server-side, so
  // there is no client-side resurrect.
  import { onMount, onDestroy, tick, untrack } from "svelte";
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { confirm } from "@tauri-apps/plugin-dialog";
  import { openSettingsWindow } from "$lib/surface-windows";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import {
    appIconFallback,
    canonicalBundleIdForComparison,
    iconPathForBundleId,
    mergeIconResolutions,
    unresolvedIconBundleIds,
    type AppIconResolution,
  } from "$lib/app-privacy-exclusion";
  import AnswerProse from "$lib/AnswerProse.svelte";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import {
    type ConversationSummary,
    type Conversation,
    type ConversationTurn,
    type AskAiAvailability,
    type AskAiStatusEvent,
    type AskAiDeltaEvent,
    type AskAiDoneEvent,
    type AskAiErrorEvent,
    type AskAiSourceEvent,
    type AskAiSource,
    type AskToolKind,
    type AskToolActivityEntry,
    type PinnableEngine,
    defaultEnginePinProvider,
    defaultEngineModel,
    engineProviderLabel,
    pinnableEnginesFromModelPool,
  } from "$lib/insights/conversation";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import type {
    AiRuntimeModel,
    AiRuntimeSettings,
    RecordingSettings,
    RecordingSettingsDomainUpdateResponse,
  } from "$lib/types/recording";

  // Quick Recall → Chat handoff (issue #111, ADR 0031). When the Insights page
  // receives an `insights_open_conversation` signal it sets `openConversationId`
  // (the handed-off thread) and bumps `openConversationNonce` so the SAME id
  // handed off twice still re-triggers the load. We select + load that
  // conversation via the normal `get_conversation` path; because Quick Recall
  // persisted it under the same id, the thread continues seamlessly (a follow-up
  // routes through ask_ai_followup, which reloads history server-side).
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

  // ── Date grouping (left rail) ────────────────────────────────────────────
  // The flat history list renders under quiet section headers computed from
  // each conversation's last-activity timestamp (`updatedAtMs`, the same field
  // the list is sorted by): Today / Yesterday / This week (the rest of the
  // last 7 calendar days) / earlier months ("May 2026"). Buckets are keyed by
  // label in first-seen order, so the existing sort order is preserved within
  // each group (and search results never produce a duplicated header).
  interface HistoryGroup {
    label: string;
    items: ConversationSummary[];
  }

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

  let historyGroups = $derived.by((): HistoryGroup[] => {
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);
    const todayStartMs = todayStart.getTime();
    const groups: HistoryGroup[] = [];
    const byLabel = new Map<string, HistoryGroup>();
    for (const c of conversations) {
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

  // ── Active conversation (right pane) ─────────────────────────────────────
  // One assistant turn in the transcript. Mirrors Quick Recall's AskTurn.
  interface ChatTurn {
    question: string;
    answer: string;
    // The model's streamed reasoning ("thinking") text, accumulated across the
    // `ask_ai_reasoning` events. Empty when the model emits none (most models) —
    // the Thinking disclosure renders only when this is non-empty.
    reasoning: string;
    // Per-turn disclosure toggle for the collapsed "Thought process" chip; the
    // live reasoning panel ignores this and is always shown while it streams.
    reasoningExpanded: boolean;
    toolActivities: AskToolActivityEntry[];
    // Live working-line entry for the in-flight tool call (cleared on resume).
    // Carries the label plus the optional app scope for the icon chip.
    toolActivity: AskToolActivityEntry | null;
    sources: AskAiSource[];
    phase: "seeding" | "thinking" | "streaming" | "done" | "error";
    errorMessage: string | null;
    seededResultCount: number | null;
    summaryExpanded: boolean;
  }

  // The active thread id. null when no conversation open.
  let activeConversationId = $state<string | null>(null);
  let activeTitle = $state<string>("");
  let turns = $state<ChatTurn[]>([]);
  let loadingConversation = $state(false);
  // True between a turn starting and that turn's terminal done/error event.
  let streaming = $state(false);
  // The live activity line just above the composer: what the engine is doing
  // RIGHT NOW for the in-flight turn. Fed by the `ask_ai_status` phases
  // (seeding → thinking → tool, with the tool's label + optional app chip) plus
  // the answer deltas ("Writing…"); cleared on done/error and on thread switch.
  let liveActivity = $state<AskToolActivityEntry | null>(null);

  // ── Per-thread model pin ─────────────────────────────────────────────────
  // A thread can be pinned to a model from the merged provider-tagged pool
  // (ai_runtime_list_models, ADR 0034) or a free-form model id (allowed per
  // ADR 0033 — never a key-entry surface). The active thread's pin is
  // (activePinProvider, activePinModel); null/null means the global default
  // model. The backend's single resolver handles any (provider, model) pin,
  // falling back through feature override to the global default.
  // Snapshot of the aiRuntime settings backing the default `{provider, model}`
  // pin labels. Refreshed on settings changes.
  let aiRuntimeSnapshot = $state<AiRuntimeSettings | null>(null);
  // The Ask AI model override (access.askAiModel, ADR 0034): a bare model id
  // that replaces the default model's id for unpinned Ask AI threads. Empty →
  // use the global default model. Refreshed alongside aiRuntimeSnapshot.
  let askAiModelOverride = $state<string | null>(null);
  let defaultProvider = $derived(
    aiRuntimeSnapshot === null ? null : defaultEnginePinProvider(aiRuntimeSnapshot),
  );
  let activePinProvider = $state<string | null>(null);
  let activePinModel = $state<string | null>(null);
  // The model the unpinned ("Default") choice resolves to, for the picker label
  // only. The backend's precedence is pin → Ask AI override (access.askAiModel)
  // → global default model, so the override wins here when set.
  let resolvedDefaultModel = $derived(
    askAiModelOverride !== null
      ? askAiModelOverride
      : aiRuntimeSnapshot === null
        ? null
        : defaultEngineModel(aiRuntimeSnapshot),
  );
  let activeModelLabel = $derived.by(() => {
    if (activePinProvider === null || activePinModel === null) {
      return resolvedDefaultModel === null
        ? "Default"
        : `Default · ${resolvedDefaultModel}`;
    }
    if (defaultProvider !== null && activePinProvider === defaultProvider) {
      return activePinModel;
    }
    // A non-default-provider pin keeps its provider context.
    return `${engineProviderLabel(activePinProvider)} · ${activePinModel}`;
  });
  let enginePickerOpen = $state(false);

  // The merged provider-tagged model pool from `ai_runtime_list_models` across
  // every connected provider. Loaded lazily on first picker open, cached until
  // a settings change invalidates it.
  let modelPool = $state<AiRuntimeModel[]>([]);
  let modelsLoaded = $state(false);
  let modelsLoading = $state(false);
  let modelsFailed = $state(false);
  let pinnableEngines = $derived(pinnableEnginesFromModelPool(modelPool));
  // Default-provider models render as bare ids (the common case)…
  let discoveredModels = $derived(
    defaultProvider === null
      ? []
      : pinnableEngines
          .filter((e) => e.provider === defaultProvider)
          .map((e) => e.model),
  );
  // …while the rest of the pool keeps its provider-tagged label so
  // multi-provider users keep their other providers' models selectable.
  let extraConfiguredEngines = $derived(
    pinnableEngines.filter((e) => e.provider !== defaultProvider),
  );

  // Free-form "Custom model…" entry at the bottom of the picker.
  let customModelOpen = $state(false);
  let customModelInput = $state("");
  let customModelEl = $state<HTMLInputElement | null>(null);

  async function loadPinnableEngines(): Promise<void> {
    try {
      const settings = await invoke<RecordingSettings>("get_recording_settings");
      aiRuntimeSnapshot = settings.aiRuntime;
      const override = settings.access?.askAiModel?.trim() ?? "";
      askAiModelOverride = override.length > 0 ? override : null;
    } catch {
      aiRuntimeSnapshot = null;
      askAiModelOverride = null;
    }
    // The provider list / default model may have changed: drop the cached model
    // pool so the next picker open re-discovers against the current providers.
    // If the picker is open right now, refresh it in place.
    modelsLoaded = false;
    modelsFailed = false;
    modelPool = [];
    if (enginePickerOpen) void loadModelList();
  }

  async function loadModelList(): Promise<void> {
    if (modelsLoading) return;
    modelsLoading = true;
    modelsFailed = false;
    try {
      // No request body: the backend lists models across the PERSISTED
      // connected providers (the Settings card's draft list is its concern).
      modelPool = await invoke<AiRuntimeModel[]>("ai_runtime_list_models");
      modelsLoaded = true;
    } catch {
      // Offline/unlisted providers degrade quietly: the picker still offers
      // Default + the custom-model entry.
      modelPool = [];
      modelsFailed = true;
    } finally {
      modelsLoading = false;
    }
  }

  function toggleModelPicker(): void {
    enginePickerOpen = !enginePickerOpen;
    customModelOpen = false;
    customModelInput = "";
    if (enginePickerOpen && !modelsLoaded) void loadModelList();
  }

  function openCustomModel(): void {
    customModelOpen = true;
    void tick().then(() => customModelEl?.focus());
  }

  function onCustomModelKeydown(event: KeyboardEvent): void {
    if (event.key === "Enter") {
      event.preventDefault();
      const model = customModelInput.trim();
      if (model.length === 0 || defaultProvider === null) return;
      customModelOpen = false;
      customModelInput = "";
      void selectEngine({ provider: defaultProvider, model });
    } else if (event.key === "Escape") {
      event.stopPropagation();
      customModelOpen = false;
      customModelInput = "";
    }
  }

  // Pin the active thread to a (provider, model) — a discovered model, a
  // configured engine, or a typed custom id — or clear the pin (null → default).
  async function selectEngine(
    engine: { provider: string; model: string } | null,
  ): Promise<void> {
    enginePickerOpen = false;
    customModelOpen = false;
    customModelInput = "";
    const conversationId = activeConversationId;
    if (conversationId === null) return;
    activePinProvider = engine?.provider ?? null;
    activePinModel = engine?.model ?? null;
    // A not-yet-started thread has no conversation row. Persisting the pin now
    // would upsert an empty-title row (a phantom "Untitled chat" in the rail),
    // so defer: the pin rides into the store on the first turn (see send()).
    if (turns.length === 0) return;
    try {
      await invoke("set_conversation_engine", {
        request: {
          conversationId,
          provider: engine?.provider ?? null,
          model: engine?.model ?? null,
        },
      });
    } catch {
      // Best-effort: the pin will be re-read on the next hydrate.
    }
  }

  // Per-frame thumbnail cache for Answer Source cards (best-effort).
  let thumbnailCache = $state(new Map<number, string>());

  let composerInput = $state("");
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let transcriptEl = $state<HTMLDivElement | null>(null);

  function makeTurn(question: string, phase: ChatTurn["phase"]): ChatTurn {
    return {
      question,
      answer: "",
      reasoning: "",
      reasoningExpanded: false,
      toolActivities: [],
      toolActivity: null,
      sources: [],
      phase,
      errorMessage: null,
      seededResultCount: null,
      summaryExpanded: false,
    };
  }

  // The segment cache lives OUTSIDE the reactive `turns` $state so the
  // template-time memoization below never writes to a $state proxy (which Svelte 5
  // forbids mid-render: `state_unsafe_mutation`). Keyed by the turn object (stable
  // proxy identity); entries are reclaimed when a turn is dropped. Reading
  // `turn.answer` inside `answerSegments` still tracks reactivity, so a freshly-
  // loaded transcript re-renders correctly — only the cache itself is non-reactive.
  // The Markdown render itself is now memoized inside AnswerProse, so there is no
  // plain-render cache here anymore.
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
    // A brand-new thread is created lazily on the first turn (ask_ai_start
    // upserts the row from title/origin), so here we just clear the right pane
    // and arm a fresh id.
    activeConversationId = crypto.randomUUID();
    activeTitle = "";
    turns = [];
    streaming = false;
    liveActivity = null;
    activePinProvider = null;
    activePinModel = null;
    enginePickerOpen = false;
    composerInput = "";
    void tick().then(() => composerEl?.focus());
  }

  async function selectConversation(summary: ConversationSummary): Promise<void> {
    if (summary.conversationId === activeConversationId && turns.length > 0) {
      return;
    }
    loadingConversation = true;
    activeConversationId = summary.conversationId;
    activeTitle = summary.title;
    turns = [];
    streaming = false;
    liveActivity = null;
    activePinProvider = null;
    activePinModel = null;
    enginePickerOpen = false;
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId: summary.conversationId,
      });
      if (convo === null || activeConversationId !== summary.conversationId) {
        return;
      }
      hydrateConversation(convo);
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
  // get_conversation rather than requiring a ConversationSummary. If the latest
  // turn is still streaming (the backend persisted its in-flight partial), we
  // keep the global delta/done/source listeners live so ongoing tokens append.
  async function loadConversationById(conversationId: string): Promise<void> {
    const id = conversationId.trim();
    if (id.length === 0) return;
    // Already on this thread with its transcript loaded — nothing to do.
    if (id === activeConversationId && turns.length > 0) return;
    loadingConversation = true;
    activeConversationId = id;
    activeTitle = "";
    turns = [];
    streaming = false;
    liveActivity = null;
    activePinProvider = null;
    activePinModel = null;
    enginePickerOpen = false;
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
      hydrateConversation(convo);
      await tick();
      scrollTranscriptToBottom();
    } catch {
      // Best-effort: leave the pane on the armed (empty) thread on a load failure.
    } finally {
      if (activeConversationId === id) loadingConversation = false;
    }
  }

  // Apply a hydrated Conversation to the active pane: title, engine pin, turns,
  // and reattach to a still-streaming last turn (so the live delta/done/source
  // listeners keep appending its partial answer). Callers have already set
  // `activeConversationId` to this convo's id.
  function hydrateConversation(convo: Conversation): void {
    activeTitle = convo.title;
    activePinProvider = convo.provider ?? null;
    activePinModel = convo.model ?? null;
    turns = convo.turns.map(hydrateTurn);
    for (const t of turns) void loadSourceThumbnails(t.sources);
    // Reattach: a persisted "streaming" last turn is still in flight server-side;
    // keep `streaming` true so the global delta listener appends ongoing tokens.
    const last = turns[turns.length - 1];
    streaming = last?.phase === "streaming";
    liveActivity = streaming ? { kind: "other", label: "Writing…" } : null;
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
    t.reasoning = turn.reasoning ?? "";
    t.toolActivities = coerceToolActivities(turn.toolActivities);
    void resolveToolAppIcons(t.toolActivities.map((a) => a.app));
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
        app:
          typeof (e as AskToolActivityEntry).app === "string"
            ? ((e as AskToolActivityEntry).app as string)
            : null,
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

  // ── Inline rename (left rail hover action) ───────────────────────────────
  // Clicking ✎ swaps the row's title for a text input pre-filled with the
  // current title. Enter commits via `set_conversation_title`, Escape cancels,
  // and blur commits-if-changed-and-non-empty (else cancels). The commit
  // optimistically rewrites the local row (and `activeTitle` when it's the
  // open thread) so the rail doesn't flicker while the backend's
  // `conversation_changed` refresh catches up.
  let renamingId = $state<string | null>(null);
  let renameDraft = $state("");

  // Tauri's WKWebView doesn't hand focus around reliably on click, so the
  // inline input focuses/selects itself programmatically once it's in the DOM
  // (a Svelte action runs post-mount; inputs DO focus fine — the quirk is
  // buttons). Keydown is attached on the input itself for the same reason.
  function autofocusSelect(node: HTMLInputElement): void {
    node.focus();
    node.select();
  }

  function startRename(summary: ConversationSummary, event: MouseEvent): void {
    event.stopPropagation();
    renamingId = summary.conversationId;
    renameDraft = summary.title || summary.preview || "";
  }

  function cancelRename(): void {
    renamingId = null;
    renameDraft = "";
  }

  async function commitRename(): Promise<void> {
    const id = renamingId;
    if (id === null) return;
    const title = renameDraft.trim();
    const row = conversations.find((c) => c.conversationId === id);
    const current = (row?.title || row?.preview || "").trim();
    renamingId = null;
    renameDraft = "";
    // Empty or unchanged → cancel (the less surprising blur behavior).
    if (title.length === 0 || title === current) return;
    // Optimistic: rewrite the row text now; `conversation_changed` re-fetches
    // the authoritative list right after the backend persists.
    conversations = conversations.map((c) =>
      c.conversationId === id ? { ...c, title } : c,
    );
    if (id === activeConversationId) activeTitle = title;
    try {
      await invoke("set_conversation_title", {
        request: { conversationId: id, title },
      });
    } catch {
      // The rename didn't land (e.g. the row vanished) — no
      // conversation_changed will fire, so re-fetch to undo the optimism.
      void refreshHistory();
    }
  }

  function onRenameKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename();
    } else if (event.key === "Escape") {
      event.stopPropagation();
      cancelRename();
    }
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
      turns = [];
      streaming = false;
      liveActivity = null;
      activePinProvider = null;
      activePinModel = null;
    }
  }

  // ── Sending a question ───────────────────────────────────────────────────
  async function send(): Promise<void> {
    const question = composerInput.trim();
    if (question.length === 0 || streaming || !askAvailable) return;

    // Lazily arm a conversation id if the pane is empty (e.g. first ever visit).
    if (activeConversationId === null) {
      activeConversationId = crypto.randomUUID();
    }
    const conversationId = activeConversationId;
    const isFirstTurn = turns.length === 0;
    if (isFirstTurn && activeTitle.length === 0) {
      activeTitle = titleFromQuestion(question);
    }
    const title = activeTitle || titleFromQuestion(question);

    composerInput = "";
    // Append the turn locally and render live from events. The backend owns
    // persistence: ask_ai_start upserts the row (from title/origin) and the turn
    // (streaming → done/error) keyed by a backend-computed turnIndex.
    const turn = makeTurn(question, isFirstTurn ? "seeding" : "thinking");
    turns = [...turns, turn];
    const turnIndex = turns.length - 1;
    streaming = true;
    liveActivity = {
      kind: "other",
      label: isFirstTurn ? "Searching your captures…" : "Thinking…",
    };
    await tick();
    scrollTranscriptToBottom();

    try {
      if (isFirstTurn) {
        // Persist any model pin chosen before the thread existed (selectEngine
        // deferred it to avoid creating a phantom empty-title row). Do this
        // BEFORE ask_ai_start so the spawned turn reads the pin from the store.
        if (activePinProvider !== null && activePinModel !== null) {
          try {
            await invoke("set_conversation_engine", {
              request: {
                conversationId,
                provider: activePinProvider,
                model: activePinModel,
              },
            });
          } catch {
            // Best-effort: the turn falls back to the default engine.
          }
        }
        // First turn of a thread — start (and upsert the conversation row).
        await invoke<void>("ask_ai_start", {
          request: {
            conversationId,
            question,
            seedQuery: question,
            origin: "chat",
            title,
          },
        });
      } else {
        // Continuing a thread — route the raw question into the session. The
        // backend reloads history from the store, so this always works (even on
        // a thread reopened from history).
        await invoke<void>("ask_ai_followup", {
          request: { conversationId, question },
        });
      }
    } catch (error) {
      if (activeConversationId !== conversationId) return;
      streaming = false;
      liveActivity = null;
      const t = turns[turnIndex];
      if (t) {
        t.phase = "error";
        t.errorMessage = error instanceof Error ? error.message : String(error);
      }
    }
  }

  function onComposerKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  }

  // ── Stop the in-flight turn (composer stop button) ───────────────────────
  // Asks the backend to cancel and nothing more: the backend emits the turn's
  // terminal ask_ai_done/ask_ai_error, and the normal listeners settle the UI
  // (streaming flag, activity line) — no double-handling here.
  async function stopStreaming(): Promise<void> {
    const conversationId = activeConversationId;
    if (conversationId === null || !streaming) return;
    try {
      await invoke<void>("ask_ai_cancel", { request: { conversationId } });
    } catch {
      // Best-effort: the task may already have finished.
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
    | { kind: "html"; markdown: string }
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
  // recognized fences becomes a raw-markdown `html` segment (AnswerProse renders
  // it); a recognized fence with a valid body becomes a chart segment, an invalid
  // one falls back to that raw markdown (so the original fenced block renders as a
  // code block — never crash). The slices stay RAW here; the Markdown→HTML pass
  // now lives in AnswerProse, which also memoizes it.
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
        segments.push({ kind: "html", markdown: pre });
      }
      if (parsed !== null) {
        segments.push(parsed);
      }
      lastIndex = match.index + full.length;
    }
    const tail = answer.slice(lastIndex);
    if (tail.trim().length > 0) {
      segments.push({ kind: "html", markdown: tail });
    }
    // An all-whitespace answer (or one fully consumed by an unparsed fence with
    // no surrounding text) yields no segments; keep the raw markdown as a
    // safety net so nothing silently disappears.
    if (segments.length === 0 && answer.trim().length > 0) {
      segments.push({ kind: "html", markdown: answer });
    }
    return segments;
  }

  // Split one turn's answer into typed segments. While streaming (or seeding/
  // thinking) the answer is rendered as plain markdown straight from AnswerProse
  // (partial JSON is never parsed); once "done" we upgrade to these graphical
  // segments. Memoized in a non-reactive WeakMap so a freshly-loaded transcript
  // re-segments only when a turn's answer actually changed.
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
      const label = queryText
        ? `Searching “${queryText}”`
        : "Searching your captures";
      // The app scope renders as an icon + name chip, not as label text.
      return { kind: "search", label, app: readString(p, "app") };
    }
    if (tool === "timeline") {
      return {
        kind: "timeline",
        label: "Scanning timeline",
        app: readString(p, "app"),
      };
    }
    if (tool === "show_text") {
      return { kind: "show_text", label: "Reading a capture" };
    }
    return { kind: "other", label: tool ? `Running ${tool}` : "Working" };
  }

  // ── Tool-activity app icons (mirrors the App Privacy Exclusion idiom) ─────
  // The tool's `app` param may be a bundle id (resolvable via
  // `resolve_app_icons`) or a free-form display name (resolves to no icon →
  // letter fallback). Resolutions are id-keyed facts merged across turns; a
  // null resolution stays in the requested set so it is never re-fetched.
  let toolAppIconPaths = $state<Record<string, string>>({});
  const requestedToolAppIconIds = new Set<string>();

  async function resolveToolAppIcons(
    apps: Array<string | null | undefined>,
  ): Promise<void> {
    const unresolved = unresolvedIconBundleIds(
      apps,
      toolAppIconPaths,
      requestedToolAppIconIds,
    );
    if (unresolved.length === 0) return;
    for (const id of unresolved) {
      requestedToolAppIconIds.add(canonicalBundleIdForComparison(id));
    }
    try {
      const icons = await invoke<AppIconResolution[]>("resolve_app_icons", {
        request: { bundleIds: unresolved },
      });
      const result = mergeIconResolutions(toolAppIconPaths, icons);
      if (result.changed) toolAppIconPaths = result.iconPathsByBundleId;
    } catch {
      // Icons are decorative; the letter fallback keeps working. The ids stay
      // marked requested so a failing resolver isn't re-polled per event.
    }
  }

  function toolAppIconSrc(app: string | null | undefined): string | null {
    if (!app) return null;
    const iconPath = iconPathForBundleId(app, toolAppIconPaths);
    return iconPath ? convertFileSrc(iconPath) : null;
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

  function toggleReasoning(turn: ChatTurn): void {
    turn.reasoningExpanded = !turn.reasoningExpanded;
  }

  // The "Thinking" disclosure renders only once reasoning text has arrived. It
  // is LIVE (an always-expanded streaming panel with a pulsing "Thinking…"
  // header) while reasoning has streamed but the answer hasn't started and the
  // turn isn't terminal; otherwise it SETTLES into the collapsed "Thought
  // process" chip.
  function hasReasoning(turn: ChatTurn): boolean {
    return turn.reasoning.trim().length > 0;
  }
  function reasoningIsLive(turn: ChatTurn): boolean {
    return (
      turn.reasoning.trim().length > 0 &&
      turn.answer.length === 0 &&
      turn.phase !== "done" &&
      turn.phase !== "error"
    );
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

  // ── Stream event wiring ──────────────────────────────────────────────────
  onMount(() => {
    void loadAskAvailability();
    void refreshHistory();
    void loadPinnableEngines();

    let destroyed = false;
    let unlistenStatus: (() => void) | undefined;
    let unlistenReasoning: (() => void) | undefined;
    let unlistenDelta: (() => void) | undefined;
    let unlistenDone: (() => void) | undefined;
    let unlistenError: (() => void) | undefined;
    let unlistenSource: (() => void) | undefined;
    let unlistenChanged: (() => void) | undefined;
    let unlistenCtx: (() => void) | undefined;
    let unlistenSettings: (() => void) | undefined;

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
        turn.toolActivity = activity;
        turn.toolActivities = [...turn.toolActivities, activity];
        liveActivity = activity;
        if (activity.app) void resolveToolAppIcons([activity.app]);
        return;
      }
      liveActivity = {
        kind: "other",
        label:
          event.payload.phase === "seeding"
            ? "Searching your captures…"
            : "Thinking…",
      };
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
      // Answer tokens are arriving — promote the activity line to "Writing…"
      // (guarded so we don't rewrite the same state on every delta).
      if (liveActivity?.label !== "Writing…") {
        liveActivity = { kind: "other", label: "Writing…" };
      }
      void tick().then(scrollTranscriptToBottom);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDelta = fn;
    });

    // Streamed reasoning ("thinking") chunks: interleaved before/between the
    // answer deltas. Mirrors the delta listener (same stale-thread guard +
    // non-empty transcript) but appends to `turn.reasoning`, then pins the
    // transcript to the bottom as the live Thinking panel grows.
    listen<{ conversationId: string; text: string }>(
      "ask_ai_reasoning",
      (event) => {
        if (event.payload.conversationId !== activeConversationId) return;
        if (!streaming) return;
        const turn = turns[turns.length - 1];
        if (!turn) return;
        turn.reasoning += event.payload.text;
        void tick().then(scrollTranscriptToBottom);
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenReasoning = fn;
    });

    listen<AskAiDoneEvent>("ask_ai_done", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turn = turns[turns.length - 1];
      if (!turn) return;
      streaming = false;
      liveActivity = null;
      turn.toolActivity = null;
      turn.phase = "done";
      // Persistence is owned by the backend (run_ask_ai_turn finalized this turn).
      void tick().then(scrollTranscriptToBottom);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenDone = fn;
    });

    listen<AskAiErrorEvent>("ask_ai_error", (event) => {
      if (event.payload.conversationId !== activeConversationId) return;
      const turn = turns[turns.length - 1];
      if (!turn) return;
      streaming = false;
      liveActivity = null;
      turn.toolActivity = null;
      turn.phase = "error";
      turn.errorMessage = event.payload.message;
      // The backend persisted the error turn; a follow-up still works (it reloads
      // history server-side), so there is no client resurrect.
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

    // Re-probe Ask AI availability + the configured engines when the engine
    // config may have changed.
    listen("user_context_changed", () => {
      void loadAskAvailability();
      void loadPinnableEngines();
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenCtx = fn;
    });

    // Settings saved in the Settings window broadcast
    // `recording_settings_domain_changed` ({ domain, settings }); the Ask AI
    // toggle/model live in the `access` domain and the Reasoning Engine config
    // in `ai_runtime`, so refresh availability + pinnable engines on those.
    listen<RecordingSettingsDomainUpdateResponse>(
      "recording_settings_domain_changed",
      (event) => {
        const domain = event.payload.domain;
        if (domain !== "ai_runtime" && domain !== "access") return;
        void loadAskAvailability();
        void loadPinnableEngines();
      },
    ).then((fn) => {
      if (destroyed) fn();
      else unlistenSettings = fn;
    });

    return () => {
      destroyed = true;
      unlistenStatus?.();
      unlistenReasoning?.();
      unlistenDelta?.();
      unlistenDone?.();
      unlistenError?.();
      unlistenSource?.();
      unlistenChanged?.();
      unlistenCtx?.();
      unlistenSettings?.();
    };
  });

  onDestroy(() => {
    if (searchDebounce !== null) clearTimeout(searchDebounce);
    // Intentionally do NOT cancel the in-flight turn here. Under the
    // stateless-per-turn-over-persistent-store model (ADR 0033), a streaming
    // turn outlives this surface: the backend keeps running and persists its
    // partial, so leaving Chat and returning reattaches via hydrateConversation.
    // Only an explicit Stop (stopStreaming) or app exit ends a task. The event
    // listeners are torn down in the onMount cleanup, so no deltas reach this
    // destroyed component.
  });
</script>

<!-- Inline app chip for tool-activity lines: resolved icon (or letter
     fallback) + the app name, matching the app-icon look used elsewhere. -->
{#snippet toolAppChip(app: string)}
  <span class="tool-app">
    <span class="tool-app-icon" aria-hidden="true">
      {#if toolAppIconSrc(app) !== null}
        <img src={toolAppIconSrc(app)} alt="" />
      {:else}
        {appIconFallback(app, app)}
      {/if}
    </span>
    <span class="tool-app-name">{app}</span>
  </span>
{/snippet}

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
        {#each historyGroups as group (group.label)}
          <div class="history-group-label" role="presentation">
            {group.label}
          </div>
          {#each group.items as c (c.conversationId)}
            <div
              class="history-item"
              class:active={c.conversationId === activeConversationId}
              role="listitem"
            >
              {#if renamingId === c.conversationId}
                <!-- Inline rename: Enter commits, Escape cancels, blur
                     commits-if-changed (else cancels). Focus/select is
                     programmatic (WKWebView focus quirk). -->
                <input
                  type="text"
                  class="history-rename-input"
                  aria-label="Rename conversation"
                  spellcheck="false"
                  autocomplete="off"
                  bind:value={renameDraft}
                  use:autofocusSelect
                  onkeydown={onRenameKeydown}
                  onblur={() => void commitRename()}
                />
              {:else}
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
                  </span>
                </button>
                <!-- Quiet row actions: hidden until the row is hovered or
                     holds keyboard focus (`:focus-within`). -->
                <div class="history-actions">
                  <button
                    type="button"
                    class="history-action"
                    aria-label="Rename conversation"
                    title="Rename conversation"
                    onclick={(e) => startRename(c, e)}
                  >
                    ✎
                  </button>
                  <button
                    type="button"
                    class="history-action history-action--delete"
                    aria-label="Delete conversation"
                    title="Delete conversation"
                    onclick={(e) => void deleteConversationRow(c, e)}
                  >
                    ✕
                  </button>
                </div>
              {/if}
            </div>
          {/each}
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
                      <!-- Thinking disclosure: the model's reasoning, ABOVE the
                           answer body. Rendered only when reasoning text arrived.
                           While reasoning streams but the answer hasn't started
                           (and the turn isn't terminal) it's a LIVE expanded
                           panel; otherwise it settles into a collapsed "Thought
                           process" chip. Reasoning is PLAIN TEXT (Svelte-escaped),
                           never routed through AnswerProse, so it reads as
                           distinct from the answer. -->
                      {#if hasReasoning(turn)}
                        {#if reasoningIsLive(turn)}
                          <div class="thinking thinking--live">
                            <p class="state state--working">
                              <span class="dot" aria-hidden="true"></span>
                              Thinking…
                            </p>
                            <div class="thinking-text">{turn.reasoning}</div>
                          </div>
                        {:else}
                          <div class="thinking">
                            <button
                              type="button"
                              class="activity-chip"
                              aria-expanded={turn.reasoningExpanded}
                              onclick={() => toggleReasoning(turn)}
                            >
                              <span
                                class="activity-caret"
                                class:open={turn.reasoningExpanded}
                                aria-hidden="true">▸</span
                              >
                              <span class="activity-summary">Thought process</span>
                            </button>
                            {#if turn.reasoningExpanded}
                              <div class="thinking-text">{turn.reasoning}</div>
                            {/if}
                          </div>
                        {/if}
                      {/if}

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
                                    <li class="activity-item">
                                      {activity.label}
                                      {#if activity.app}
                                        in {@render toolAppChip(activity.app)}
                                      {/if}
                                    </li>
                                  {/each}
                                </ul>
                              {/if}
                            </div>
                          {/if}

                          <!-- The answer body. While streaming AnswerProse renders
                               plain markdown (with its own caret); once done we
                               upgrade to graphical segments (mnema-bars /
                               mnema-dossier), with prose runs still through
                               AnswerProse. -->
                          <div class="answer">
                            {#if turn.phase === "done"}
                              {#each answerSegments(turn) as seg, si (si)}
                                {#if seg.kind === "html"}
                                  <AnswerProse source={seg.markdown} isStreaming={false} />
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
                              <AnswerProse source={turn.answer} isStreaming={true} />
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
                            <span class="working-label"
                              >{turn.toolActivity.label}</span
                            >
                            {#if turn.toolActivity.app}
                              in
                              {@render toolAppChip(turn.toolActivity.app)}
                            {/if}
                          </p>
                        {:else if turn.phase === "streaming"}
                          <!-- Answer text is arriving: label the phase so the
                               caret in AnswerProse reads as the insertion
                               point, not an unexplained blink. -->
                          <p class="state state--working">
                            <span class="dot" aria-hidden="true"></span>
                            Writing…
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
          <!-- Live activity line: what the engine is doing right now (seeding →
               thinking → tool → writing). Space is reserved so the composer
               doesn't jump when it appears/clears. -->
          <div class="composer-activity" aria-live="polite">
            {#if liveActivity !== null}
              <span class="dot" aria-hidden="true"></span>
              <span class="working-label">{liveActivity.label}</span>
              {#if liveActivity.app}
                in {@render toolAppChip(liveActivity.app)}
              {/if}
            {/if}
          </div>
          <!-- One bordered composer block: the textarea on top and a slim bottom
               row inside the same border — model picker left, send/stop right. -->
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
            <div class="composer-bar">
              <!-- Per-thread model pin: choose which model answers this thread —
                   a model discovered from the default engine, a configured
                   engine, or a typed custom id — or "Default" to clear the pin.
                   The label shows what "Default" resolves to when unpinned. -->
              <div class="engine-pick-menu">
                <button
                  type="button"
                  class="engine-pick-trigger"
                  aria-haspopup="listbox"
                  aria-expanded={enginePickerOpen}
                  aria-label="Model for this thread"
                  onclick={toggleModelPicker}
                >
                  <span class="engine-pick-current">{activeModelLabel}</span>
                  <span class="engine-pick-caret" aria-hidden="true">▾</span>
                </button>
                {#if enginePickerOpen}
                  <ul class="engine-pick-list" role="listbox" aria-label="Model">
                    <li role="presentation">
                      <button
                        type="button"
                        class="engine-pick-option"
                        class:engine-pick-option--active={activePinProvider === null}
                        role="option"
                        aria-selected={activePinProvider === null}
                        onclick={() => void selectEngine(null)}
                      >
                        Default
                        {#if activePinProvider === null}
                          <span class="engine-pick-check" aria-hidden="true">✓</span>
                        {/if}
                      </button>
                    </li>
                    {#if modelsLoading}
                      <li role="presentation">
                        <span class="engine-pick-note">Loading models…</span>
                      </li>
                    {:else if modelsFailed}
                      <li role="presentation">
                        <span class="engine-pick-note">Couldn't load models</span>
                      </li>
                    {/if}
                    {#each discoveredModels as modelId (modelId)}
                      {@const selected =
                        defaultProvider === activePinProvider &&
                        modelId === activePinModel}
                      <li role="presentation">
                        <button
                          type="button"
                          class="engine-pick-option"
                          class:engine-pick-option--active={selected}
                          role="option"
                          aria-selected={selected}
                          onclick={() => {
                            if (defaultProvider !== null) {
                              void selectEngine({
                                provider: defaultProvider,
                                model: modelId,
                              });
                            }
                          }}
                        >
                          {modelId}
                          {#if selected}
                            <span class="engine-pick-check" aria-hidden="true">✓</span>
                          {/if}
                        </button>
                      </li>
                    {/each}
                    {#each extraConfiguredEngines as engine (`${engine.provider}-${engine.model}`)}
                      {@const selected =
                        engine.provider === activePinProvider &&
                        engine.model === activePinModel}
                      <li role="presentation">
                        <button
                          type="button"
                          class="engine-pick-option"
                          class:engine-pick-option--active={selected}
                          role="option"
                          aria-selected={selected}
                          onclick={() => void selectEngine(engine)}
                        >
                          {engine.label}
                          {#if selected}
                            <span class="engine-pick-check" aria-hidden="true">✓</span>
                          {/if}
                        </button>
                      </li>
                    {/each}
                    <li role="presentation">
                      {#if customModelOpen}
                        <input
                          bind:this={customModelEl}
                          bind:value={customModelInput}
                          class="engine-pick-custom-input"
                          type="text"
                          placeholder="model id"
                          aria-label="Custom model id"
                          spellcheck="false"
                          autocomplete="off"
                          onkeydown={onCustomModelKeydown}
                        />
                      {:else}
                        <button
                          type="button"
                          class="engine-pick-option"
                          onclick={openCustomModel}
                        >
                          Custom model…
                        </button>
                      {/if}
                    </li>
                  </ul>
                {/if}
              </div>
              <!-- Send ⇄ Stop morph: while a turn streams the button becomes a
                   stop control that asks the backend to cancel; the resulting
                   done/error event settles the UI. -->
              <button
                type="button"
                class="composer-send"
                class:composer-send--stop={streaming}
                disabled={!streaming && composerInput.trim().length === 0}
                onclick={() => (streaming ? void stopStreaming() : void send())}
                aria-label={streaming ? "Stop" : "Send"}
                title={streaming ? "Stop generating" : "Send (Enter)"}
              >
                {#if streaming}
                  ■
                {:else}
                  ↑
                {/if}
              </button>
            </div>
          </div>
        </div>
      {:else}
        <div class="composer-wrap">
          <div class="engine-off">
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
  /* Quiet date-section headers (Today / Yesterday / This week / "May 2026"),
     in the same uppercase-faint idiom as .sources-heading. */
  .history-group-label {
    font-size: 9.5px;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--app-text-faint);
    padding: 8px 6px 3px;
  }
  .history-group-label:first-child {
    padding-top: 2px;
  }
  /* Row actions (rename + delete): hidden until the row is hovered or holds
     keyboard focus — pure hover-only would lock keyboard users out. */
  .history-actions {
    flex: 0 0 auto;
    display: flex;
    align-items: center;
    padding-right: 3px;
    opacity: 0;
    transition: opacity 0.12s ease;
  }
  .history-item:hover .history-actions,
  .history-item:focus-within .history-actions {
    opacity: 1;
  }
  .history-action {
    width: 22px;
    height: 22px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    border-radius: 5px;
    background: transparent;
    color: var(--app-text-faint);
    font-size: 10px;
    cursor: pointer;
    transition: color 0.12s ease, background 0.12s ease;
  }
  .history-action:hover {
    color: var(--app-text-strong);
    background: var(--app-surface-hover);
  }
  .history-action--delete:hover {
    color: var(--app-danger);
  }
  /* Inline rename input: replaces the row content while editing. */
  .history-rename-input {
    flex: 1 1 auto;
    min-width: 0;
    margin: 4px 5px;
    padding: 4px 6px;
    font: inherit;
    font-size: 11.5px;
    border: 1px solid var(--app-accent-border);
    border-radius: 5px;
    background: var(--app-surface);
    color: var(--app-text);
    outline: none;
  }
  .history-rename-input:focus {
    border-color: var(--app-accent);
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
  /* Inline app chip in tool-activity lines: the .app-rule-icon look at 16px. */
  .tool-app {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    vertical-align: middle;
  }
  .tool-app-icon {
    display: grid;
    width: 16px;
    height: 16px;
    flex: 0 0 16px;
    place-items: center;
    overflow: hidden;
    border: 1px solid var(--app-border);
    border-radius: 4px;
    background: var(--app-surface);
    color: var(--app-text-muted);
    font-size: 9px;
    font-weight: 800;
    line-height: 1;
  }
  .tool-app-icon img {
    width: 13px;
    height: 13px;
    object-fit: contain;
  }
  .tool-app-name {
    color: var(--app-text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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

  /* Thinking disclosure: the model's reasoning. Quiet/secondary to the answer —
     reuses the activity-chip styling for its collapsed "Thought process" chip,
     and a muted inset panel for the streamed reasoning text (live or expanded). */
  .thinking {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .thinking-text {
    max-height: 180px;
    overflow: auto;
    padding: 8px 10px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
  /* Per-thread model pin (inside the composer's bottom bar). */
  .engine-pick-menu {
    position: relative;
  }
  .engine-pick-trigger {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font: inherit;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 4px 10px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .engine-pick-trigger:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .engine-pick-caret {
    font-size: 8px;
  }
  /* Long custom model ids stay on one line inside the pill. */
  .engine-pick-current {
    max-width: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .engine-pick-list {
    position: absolute;
    bottom: calc(100% + 4px);
    left: 0;
    min-width: 200px;
    max-height: 240px;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 4px;
    border: 1px solid var(--app-border);
    border-radius: 8px;
    background: var(--app-surface-raised);
    box-shadow: 0 8px 24px var(--app-shadow, rgba(0, 0, 0, 0.25));
    z-index: 20;
  }
  .engine-pick-option {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    width: 100%;
    font: inherit;
    font-size: 11px;
    text-align: left;
    padding: 6px 9px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--app-text);
    cursor: pointer;
    transition: background 0.12s ease;
  }
  .engine-pick-option:hover {
    background: var(--app-surface-hover);
  }
  .engine-pick-option--active {
    color: var(--app-accent-strong);
  }
  .engine-pick-check {
    color: var(--app-accent-strong);
    font-size: 10px;
  }
  /* Muted one-liner inside the dropdown (loading / discovery failure). */
  .engine-pick-note {
    display: block;
    font-size: 11px;
    padding: 6px 9px;
    color: var(--app-text-faint);
  }
  /* Free-form "Custom model…" id input, matching the option rows. */
  .engine-pick-custom-input {
    width: 100%;
    font: inherit;
    font-size: 11px;
    padding: 6px 9px;
    border: 1px solid var(--app-border);
    border-radius: 6px;
    background: var(--app-surface-subtle);
    color: var(--app-text);
  }
  .engine-pick-custom-input:focus {
    outline: none;
    border-color: var(--app-border-hover);
  }

  /* Live activity line above the composer block. Space is reserved (min-height)
     so the composer doesn't jump when the line appears/clears. */
  .composer-activity {
    display: flex;
    align-items: center;
    gap: 7px;
    max-width: 760px;
    min-height: 17px;
    margin: 0 auto 6px;
    font-size: 11px;
    color: var(--app-text-muted);
  }

  /* One bordered composer block: textarea on top, slim bottom row inside the
     same border (model picker left, send/stop right). */
  .composer {
    display: flex;
    flex-direction: column;
    max-width: 760px;
    margin: 0 auto;
    border: 1px solid var(--app-border);
    border-radius: 9px;
    background: var(--app-surface);
    transition: border-color 0.12s ease;
  }
  .composer:focus-within {
    border-color: var(--app-accent-border);
  }
  .composer-input {
    flex: 0 0 auto;
    min-width: 0;
    resize: none;
    font: inherit;
    font-size: 12.5px;
    line-height: 1.5;
    max-height: 140px;
    padding: 10px 12px 4px;
    border: none;
    border-radius: 9px 9px 0 0;
    background: transparent;
    color: var(--app-text);
    outline: none;
    field-sizing: content;
  }
  .composer-input::placeholder {
    color: var(--app-text-faint);
  }
  .composer-input:disabled {
    opacity: 0.6;
    cursor: not-allowed;
  }
  /* Slim bottom row inside the composer block. */
  .composer-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 8px 8px 10px;
  }
  .composer-send {
    flex: 0 0 auto;
    width: 30px;
    height: 26px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 13px;
    border: 1px solid var(--app-accent-border);
    border-radius: 7px;
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
  /* The send glyph morphs into a stop square while a turn streams. */
  .composer-send--stop {
    font-size: 10px;
  }

  /* Engine-off quiet card (replaces the composer block, same centered width). */
  .engine-off {
    display: flex;
    align-items: center;
    gap: 10px;
    max-width: 760px;
    margin: 0 auto;
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
