<script lang="ts">
  import { tip } from "$lib/components/tooltip";
  // Chat — the conversation pane of the Insights surface (issue #110, ADR 0031).
  // The history list / search / new-chat / rename / delete now live in the
  // persistent shell rail (<InsightsRail>); Chat is JUST the active-conversation
  // pane (transcript + composer), driven by the shared conversation store (#102)
  // via its selection bus. Conversations are persisted to that store and answered
  // by the SAME Ask AI engine, which reaches capture data ONLY through the
  // brokered tools. Quick Recall persists to that SAME store too (issue #111), so
  // a launcher thread can be opened/continued here (the "Continue in Chat"
  // hand-off) under the same conversationId. Answers can additionally render
  // inline charts / dossier cards (acceptance #2) via the mnema-bars /
  // mnema-dossier fenced-block post-processor below.
  //
  // Layout: the active conversation fills the surface — a vertically-scrolling
  // transcript (ONLY the transcript scrolls, not the page) rendered as a centered
  // column where the user's question is a right-aligned bubble and the engine's
  // answer is left-aligned (with inline charts / dossier cards + Answer Sources),
  // and a bottom composer (Enter sends, Shift+Enter newlines). When the engine is
  // off the composer is replaced by a quiet "enable" card.
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
  import { message } from "@tauri-apps/plugin-dialog";
  import { openSettings } from "$lib/surface-windows";
  import { framePreviewAssetUrl } from "$lib/frame-preview";
  import { openCapturedUrl } from "$lib/open-captured-url";
  import { askAiClock } from "$lib/askAiClock";
  import { humanizeError } from "$lib/format-error";
  import { appIconFallback } from "$lib/app-privacy-exclusion";
  import AnswerProse from "$lib/AnswerProse.svelte";
  import AnswerSourceCard from "$lib/components/AnswerSourceCard.svelte";
  import FrameDetailModal from "$lib/components/FrameDetailModal.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import Timeline from "$lib/insights/charts/Timeline.svelte";
  import ConfidenceBar from "$lib/insights/charts/ConfidenceBar.svelte";
  import Skeleton from "$lib/insights/Skeleton.svelte";
  import {
    type Conversation,
    type ConversationTurn,
    type AskAiAvailability,
    type AskAiSource,
    type AnswerBlock,
    type ToolActivityEntry,
    type TurnView,
    type TurnSnapshot,
    type TurnUpdate,
    type AskAiUpdateEvent,
    contextWindowForModel,
    defaultEngineModel,
    defaultEnginePinProvider,
  } from "$lib/insights/conversation";
  import ModelPicker from "$lib/insights/ModelPicker.svelte";
  import { conversationStore } from "$lib/insights/conversationStore.svelte";
  import type { FrameScrubPreviewsDto } from "$lib/types/app-infra";
  import type {
    AiRuntimeModel,
    AiRuntimeModelsResult,
    AiRuntimeSettings,
    RecordingSettings,
    RecordingSettingsDomainUpdateResponse,
  } from "$lib/types/recording";

  // Quick Recall → Chat handoff (issue #111, ADR 0031) and the rail's row
  // selection now route through the shared `conversationStore` selection BUS
  // (`pendingOpen`): a surface calls `requestOpen(id)` / `requestNewChat()` and
  // Chat watches the bus to do the actual load. Because Quick Recall persisted a
  // handed-off thread under the same id, opening it here continues it seamlessly
  // (a follow-up routes through ask_ai_followup, which reloads history
  // server-side).
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
        reason: humanizeError(error),
      };
    }
  }

  function enableEngine(): void {
    void openSettings("intelligence");
  }

  // ── History list (left rail) ─────────────────────────────────────────────
  // The history list, debounced search, date grouping, rename, and delete now
  // ALL live in the shared `conversationStore` singleton (the rail markup below
  // binds to it directly). Chat only OWNS the right-pane active thread; it writes
  // `conversationStore.activeConversationId` (for the rail highlight) and opens a
  // thread when the store's selection bus (`pendingOpen`) changes.

  // ── Active conversation (right pane) ─────────────────────────────────────
  // One assistant turn in the transcript. This is the backend-owned `TurnView`
  // render model (issue #110, ADR 0031) plus two UI-only toggles. The frontend
  // ONLY renders: it applies versioned `TurnUpdate` ops from `ask_ai_update`,
  // snapshots on attach, and re-snapshots on a version gap. It does NO fence
  // parsing, NO tool-label formatting, NO icon-cache batching, NO local phase
  // machine — those all moved server-side.
  interface ChatTurn {
    turnIndex: number;
    question: string;
    phase: "seeding" | "thinking" | "streaming" | "done" | "error";
    // Render-ready answer blocks (prose markdown stays rendered on the frontend
    // via AnswerProse; the graphical variants carry already-parsed data).
    blocks: AnswerBlock[];
    // The model's reasoning ("thinking") text, or null when none — the Thinking
    // disclosure renders only when this is non-empty.
    reasoning: string | null;
    // Render-ready tool-activity log (label + optional app + resolved icon path,
    // all computed server-side).
    toolActivities: ToolActivityEntry[];
    // Live working-line entry for the in-flight tool call (cleared on done).
    liveActivity: ToolActivityEntry | null;
    // Sources stay opaque AskAiSource[]: the card renderer + thumbnail/open logic
    // depend on its fields. The backend's `Sources` update carries the same JSON.
    sources: AskAiSource[];
    errorMessage: string | null;
    seededResultCount: number | null;
    // Tokens occupying the model's context window after this turn's latest
    // completion request; null when the provider reported no usage (and on
    // hydrated past turns — usage isn't persisted).
    contextTokens: number | null;
    // Last applied `ask_ai_update` version for this turn (0 if none — e.g. a
    // hydrated past turn that isn't live).
    version: number;
    // UI-only disclosure toggles.
    reasoningExpanded: boolean;
    summaryExpanded: boolean;
  }

  // The active thread id. null when no conversation open.
  let activeConversationId = $state<string | null>(null);
  let activeTitle = $state<string>("");
  // Origin metadata of the active thread (ADR 0058): `origin === "trigger"`
  // flips the FIRST turn into Document View (titled page, no question bubble);
  // follow-ups render as normal chat beneath. Hydrated from get_conversation —
  // purely a render fork, the streaming/store paths are identical.
  let activeOrigin = $state<string | null>(null);
  let activeTriggerName = $state<string | null>(null);
  let activeCreatedAtMs = $state<number | null>(null);
  const isTriggerDoc = $derived(activeOrigin === "trigger");
  // "ran Jul 20, 2:49 PM" — the document header's metadata line.
  const docRanAt = $derived(
    activeCreatedAtMs !== null
      ? new Date(activeCreatedAtMs).toLocaleString(undefined, {
          month: "short",
          day: "numeric",
          hour: "numeric",
          minute: "2-digit",
        })
      : null,
  );
  // Mirror the active id UP to the store so the (future) rail can highlight the
  // open row. One-way: the store value is only READ by the rail, never read back
  // into Chat's state, so there is no loop.
  $effect(() => {
    conversationStore.activeConversationId = activeConversationId;
  });
  // The title shown for the active thread: prefer the store's row (so a rename of
  // the open thread reflects immediately), falling back to the locally hydrated
  // `activeTitle` for a thread not (yet) in the capped/filtered rail list.
  const displayTitle = $derived(
    conversationStore.conversations.find(
      (c) => c.conversationId === activeConversationId,
    )?.title ?? activeTitle,
  );
  let turns = $state<ChatTurn[]>([]);
  let loadingConversation = $state(false);
  // Set when opening a thread from the rail/handoff throws — so a failed load
  // renders a recoverable error state (with Retry) instead of silently dropping
  // the user on an empty "new chat" invite that looks like nothing happened.
  let conversationLoadError = $state<string | null>(null);
  // True between a turn starting and that turn's terminal done/error event.
  let streaming = $state(false);
  // The live activity line just above the composer: what the engine is doing
  // RIGHT NOW for the in-flight turn. Mirrors the active turn's `liveActivity`
  // (driven by the backend `LiveActivity` updates); cleared on done/error and on
  // thread switch.
  let liveActivity = $state<ToolActivityEntry | null>(null);

  // ── Per-thread model pin ─────────────────────────────────────────────────
  // A thread can be pinned to a model from the merged provider-tagged pool
  // (ai_runtime_list_models, ADR 0034) or a free-form model id (allowed per
  // ADR 0033 — never a key-entry surface). The active thread's pin is
  // (activePinProvider, activePinModel); null/null means the global default
  // model. The backend's single resolver handles any (provider, model) pin,
  // falling back through feature override to the global default. The picker UI
  // (trigger + searchable dropdown) lives in <ModelPicker>; this owns the pin
  // and the settings snapshot it feeds down.
  // Snapshot of the aiRuntime settings backing the default `{provider, model}`
  // pin labels. Refreshed on settings changes.
  let aiRuntimeSnapshot = $state<AiRuntimeSettings | null>(null);
  // The Ask AI model override (access.askAiModel, ADR 0034): a bare model id
  // that replaces the default model's id for unpinned Ask AI threads. Empty →
  // use the global default model. Refreshed alongside aiRuntimeSnapshot.
  let askAiModelOverride = $state<string | null>(null);
  let activePinProvider = $state<string | null>(null);
  let activePinModel = $state<string | null>(null);
  // Open state for the model picker pill, bound to <ModelPicker>; reset on
  // thread switch so a stale dropdown never lingers across conversations.
  let enginePickerOpen = $state(false);

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
    // <ModelPicker> watches the connected-provider set and invalidates its own
    // model pool, so there is nothing to reset here beyond the snapshot it reads.
  }

  // Commit the model chosen in <ModelPicker> — a pooled model or a typed id
  // attributed to a provider — or clear the pin (null → default). The picker
  // closes itself; here we just record the pin and persist it.
  async function handleModelSelect(
    engine: { provider: string; model: string } | null,
  ): Promise<void> {
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

  // Context-window occupancy shown in the composer bar: the latest turn that
  // carries a provider-reported count. Null hides the readout (no live turn
  // yet, or a cold-loaded thread — usage isn't persisted, so it reappears on
  // the next answer).
  const contextTokens = $derived(
    turns.findLast((t) => t.contextTokens !== null)?.contextTokens ?? null,
  );

  // The engine answering this thread (pin → Ask AI override → global default —
  // same precedence as the backend resolver).
  const activeEngineProvider = $derived(
    activePinProvider ??
      (aiRuntimeSnapshot !== null
        ? defaultEnginePinProvider(aiRuntimeSnapshot)
        : null),
  );
  const activeEngineModel = $derived(
    activePinModel ??
      askAiModelOverride ??
      (aiRuntimeSnapshot !== null ? defaultEngineModel(aiRuntimeSnapshot) : null),
  );

  // Provider-reported context windows: the active provider's model listing
  // (the same call the picker makes), fetched lazily once per provider id and
  // only once usage is actually on screen. Best-effort — a failed listing just
  // leaves the known-family table as the only source.
  let providerModels = $state<Record<string, AiRuntimeModel[]>>({});
  const providerModelsRequested = new Set<string>();
  $effect(() => {
    const provider = activeEngineProvider;
    if (provider === null || contextTokens === null) return;
    if (providerModelsRequested.has(provider)) return;
    const config = aiRuntimeSnapshot?.providers.find((p) => p.id === provider);
    if (config === undefined) return;
    providerModelsRequested.add(provider);
    void invoke<AiRuntimeModelsResult>("ai_runtime_list_models", {
      request: { providers: [$state.snapshot(config)] },
    })
      .then((result) => {
        providerModels[provider] = result.models;
      })
      .catch(() => {});
  });

  // The context-window size of the model answering this thread: the provider-
  // reported size when its listing advertises one, else the known-family
  // table. Null (unknown model, or a local server that doesn't say) keeps the
  // readout text-only — the used count still shows, the ring needs a real
  // denominator.
  const contextWindow = $derived.by(() => {
    const model = activeEngineModel;
    if (model === null) return null;
    const reported =
      activeEngineProvider !== null
        ? (providerModels[activeEngineProvider]?.find((m) => m.id === model)
            ?.contextWindow ?? null)
        : null;
    return reported ?? contextWindowForModel(model);
  });
  // 0..1 occupancy for the ring (clamped — usage can exceed a guessed window).
  const contextFraction = $derived(
    contextTokens !== null && contextWindow !== null
      ? Math.min(1, contextTokens / contextWindow)
      : null,
  );

  // "812" / "12.4k" / "1.2M" — compact enough for the composer bar.
  function formatTokenCount(tokens: number): string {
    if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
    if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}k`;
    return `${tokens}`;
  }

  function makeTurn(
    turnIndex: number,
    question: string,
    phase: ChatTurn["phase"],
  ): ChatTurn {
    return {
      turnIndex,
      question,
      phase,
      blocks: [],
      reasoning: null,
      toolActivities: [],
      liveActivity: null,
      sources: [],
      errorMessage: null,
      seededResultCount: null,
      contextTokens: null,
      version: 0,
      reasoningExpanded: false,
      summaryExpanded: false,
    };
  }

  // Trim/truncate the first question into a conversation title.
  function titleFromQuestion(question: string): string {
    const t = question.trim().replace(/\s+/g, " ");
    return t.length > TITLE_MAX ? `${t.slice(0, TITLE_MAX - 1)}…` : t;
  }

  // ── New chat / select / delete ───────────────────────────────────────────
  // `prefill` (a Subject→Chat hand-off) seeds the composer with a question to
  // review/edit; it is NOT auto-sent, mirroring the example-question affordance.
  function startNewChat(prefill: string | null = null): void {
    // A brand-new thread is created lazily on the first turn (ask_ai_start
    // upserts the row from title/origin), so here we just clear the right pane
    // and arm a fresh id.
    activeConversationId = crypto.randomUUID();
    activeTitle = "";
    activeOrigin = null;
    activeTriggerName = null;
    activeCreatedAtMs = null;
    turns = [];
    conversationLoadError = null;
    streaming = false;
    liveActivity = null;
    activePinProvider = null;
    activePinModel = null;
    enginePickerOpen = false;
    composerInput = prefill ?? "";
    void tick().then(() => {
      composerEl?.focus();
      // Drop the caret at the end so the user can keep typing after a prefill.
      if (composerEl) {
        const end = composerEl.value.length;
        composerEl.setSelectionRange(end, end);
      }
    });
  }

  // Empty-state example questions: tapping one prefills the composer (the user
  // then reviews/edits and presses Enter) — it does NOT auto-send.
  const EXAMPLE_QUESTIONS = [
    "What did I work on today?",
    "Summarize my afternoon",
    "Which apps did I spend the most time in?",
    "Find that article I was reading earlier",
  ];

  function useExample(text: string): void {
    composerInput = text;
    void tick().then(() => composerEl?.focus());
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
    conversationLoadError = null;
    activeConversationId = id;
    activeTitle = "";
    activeOrigin = null;
    activeTriggerName = null;
    activeCreatedAtMs = null;
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
      await hydrateConversation(convo);
      if (activeConversationId !== id) return;
      await tick();
      scrollTranscriptToBottom();
    } catch (error) {
      // A load failure must not masquerade as an empty "new chat" pane — record
      // it so the transcript renders a recoverable error state with Retry.
      if (activeConversationId === id) {
        conversationLoadError =
          humanizeError(error);
      }
    } finally {
      if (activeConversationId === id) loadingConversation = false;
    }
  }

  // Apply a hydrated Conversation to the active pane: title, engine pin, turns,
  // then adopt any live snapshot so a reattach to an in-flight turn is race-free.
  // Callers have already set `activeConversationId` to this convo's id.
  async function hydrateConversation(convo: Conversation): Promise<void> {
    const conversationId = convo.conversationId;
    activeTitle = convo.title;
    activeOrigin = convo.origin;
    activeTriggerName = convo.triggerName ?? null;
    activeCreatedAtMs = convo.createdAtMs;
    activePinProvider = convo.provider ?? null;
    activePinModel = convo.model ?? null;
    turns = convo.turns.map(hydrateTurn);
    for (const t of turns) void loadSourceThumbnails(t.sources);
    // A persisted "streaming" last turn is still in flight server-side; the
    // snapshot below replaces it with the authoritative live view + version.
    const last = turns[turns.length - 1];
    streaming = last?.phase === "streaming";
    liveActivity = null;
    await adoptLiveSnapshot(conversationId);
  }

  // Snapshot-on-attach: fetch the conversation's in-flight LiveTurn (if any) and
  // adopt it onto the matching turn, bootstrapping a reattach to a streaming turn
  // race-free. `ask_ai_update` events at/below the adopted version are then
  // ignored, and a version gap re-snapshots. A null snapshot means no live turn
  // (all from DB) — nothing to do.
  async function adoptLiveSnapshot(conversationId: string): Promise<void> {
    let snapshot: TurnSnapshot | null;
    try {
      snapshot = await invoke<TurnSnapshot | null>("ask_ai_snapshot", {
        request: { conversationId },
      });
    } catch {
      return;
    }
    if (snapshot === null || activeConversationId !== conversationId) return;
    const turn = turns.find((t) => t.turnIndex === snapshot.view.turnIndex);
    if (!turn) return;
    adoptView(turn, snapshot.view, snapshot.version);
    const live = turn.phase !== "done" && turn.phase !== "error";
    streaming = live;
    liveActivity = live ? turn.liveActivity : null;
  }

  // Replace a turn's render fields from a backend `TurnView` and stamp its
  // version. Used by snapshot-on-attach and version-gap re-snapshot.
  function adoptView(turn: ChatTurn, view: TurnView, version: number): void {
    turn.phase = normalizePhase(view.phase);
    turn.blocks = view.blocks;
    turn.reasoning = view.reasoning;
    turn.toolActivities = view.toolActivities;
    turn.liveActivity = view.liveActivity;
    turn.sources = coerceSources(view.sources);
    turn.errorMessage = view.errorMessage;
    turn.seededResultCount = view.seededResultCount;
    turn.contextTokens = view.contextTokens;
    turn.version = version;
    void loadSourceThumbnails(turn.sources);
  }

  // React to the store's selection BUS. Each request (rail row click, "+ New
  // chat", or a Quick Recall handoff) bumps `pendingOpen.nonce`, so reading the
  // whole `{id, nonce}` re-runs this on every request — even a repeat of the same
  // id. `id === null` means "start a fresh empty chat". We track the last-seen
  // nonce and bail at nonce 0 so initial mount (nothing requested yet) does NOT
  // fire a spurious startNewChat().
  let lastOpenNonce = 0;
  $effect(() => {
    const pending = conversationStore.pendingOpen; // track {id, nonce}
    untrack(() => {
      if (pending.nonce === 0 || pending.nonce === lastOpenNonce) return;
      lastOpenNonce = pending.nonce;
      if (pending.id === null) startNewChat(pending.prefill);
      else void loadConversationById(pending.id);
    });
  });

  // Hydrate a persisted ConversationTurn into a ChatTurn. The backend's
  // get_conversation populates `turn.blocks` for EVERY turn (new + legacy
  // parsed-on-read), so blocks are taken DIRECTLY with no frontend parsing.
  //
  // Tool activities: the persisted `tool_activities` JSON on the turn row is
  // still the raw `{tool, params}` shape (Slices 4/5 did not migrate it). The
  // live render-ready path (label + icon) only exists on streaming/snapshot
  // views, not on cold history. So we map each raw `{tool}` to a minimal
  // render-ready `ToolActivityEntry` (kind + generic label) JUST for the
  // collapsed activity summary on a reloaded thread's past turns — we do NOT
  // re-introduce the streaming icon/label resolution path. A live turn that
  // reattaches replaces these via the snapshot's render-ready toolActivities.
  function hydrateTurn(turn: ConversationTurn): ChatTurn {
    const t = makeTurn(turn.turnIndex, turn.question, normalizePhase(turn.phase));
    t.blocks = turn.blocks ?? [];
    t.reasoning = turn.reasoning;
    t.toolActivities = coerceToolActivities(turn.toolActivities);
    t.sources = coerceSources(turn.sources);
    t.errorMessage = turn.errorMessage;
    t.seededResultCount = turn.seededResultCount;
    return t;
  }

  function normalizePhase(phase: string): ChatTurn["phase"] {
    return phase === "done" ||
      phase === "error" ||
      phase === "streaming" ||
      phase === "thinking" ||
      phase === "seeding"
      ? phase
      : "done";
  }

  // Map the persisted raw `{tool, params}` per-turn log to minimal render-ready
  // entries (kind + generic label) so a reloaded thread's collapsed activity
  // summary still counts correctly (activitySummaryFor reads `kind`). Streaming
  // and snapshot views deliver fully render-ready entries instead.
  function coerceToolActivities(value: unknown): ToolActivityEntry[] {
    if (!Array.isArray(value)) return [];
    return value
      .map((e): ToolActivityEntry | null => {
        if (typeof e !== "object" || e === null) return null;
        const rec = e as { tool?: unknown; kind?: unknown; label?: unknown };
        // Already render-ready (a snapshot/live entry round-tripped to the DB).
        if (typeof rec.label === "string" && typeof rec.kind === "string") {
          return { kind: rec.kind, label: rec.label };
        }
        const tool = typeof rec.tool === "string" ? rec.tool : null;
        if (tool === "search")
          return { kind: "search", label: "Searched your captures" };
        if (tool === "timeline")
          return { kind: "timeline", label: "Scanned timeline" };
        if (tool === "show_text")
          return { kind: "show_text", label: "Read a capture" };
        if (tool === "recall_context")
          return { kind: "recall_context", label: "Recalled what I know about you" };
        return { kind: "other", label: tool ? `Ran ${tool}` : "Working" };
      })
      .filter((x): x is ToolActivityEntry => x !== null);
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
    // Append the turn locally and render live from `ask_ai_update` events. The
    // backend owns persistence AND the render model: ask_ai_start upserts the row
    // and drives this turn's view via versioned updates (version starts at 0).
    // Defensive: settle any prior turn still flagged working in memory. A
    // cancelled/displaced turn that never received a terminal update would
    // otherwise keep showing its inline working line beneath a finished answer
    // once a new turn is appended below it.
    for (const t of turns) {
      if (t.phase === "streaming" || t.phase === "thinking" || t.phase === "seeding") {
        t.liveActivity = null;
        t.phase = "done";
      }
    }
    const turnIndex = turns.length;
    const turn = makeTurn(turnIndex, question, isFirstTurn ? "seeding" : "thinking");
    turns = [...turns, turn];
    streaming = true;
    liveActivity = {
      kind: "other",
      label: isFirstTurn ? "Searching your captures…" : "Thinking…",
    };
    await tick();
    scrollTranscriptToBottom();

    try {
      if (isFirstTurn) {
        // Persist any model pin chosen before the thread existed
        // (handleModelSelect deferred it to avoid creating a phantom empty-title
        // row). Do this
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
            ...askAiClock(),
          },
        });
      } else {
        // Continuing a thread — route the raw question into the session. The
        // backend reloads history from the store, so this always works (even on
        // a thread reopened from history).
        await invoke<void>("ask_ai_followup", {
          request: { conversationId, question, ...askAiClock() },
        });
      }
    } catch (error) {
      if (activeConversationId !== conversationId) return;
      streaming = false;
      liveActivity = null;
      const t = turns[turnIndex];
      if (t) {
        t.phase = "error";
        t.errorMessage = humanizeError(error);
      }
      // A failed send cleared the composer above — put the question back so the
      // user can edit + resend without retyping (only if they haven't started
      // typing something else in the meantime).
      restoreFailedQuestion(question);
    }
  }

  // Restore a failed turn's question into the composer so it isn't lost. Only
  // when the composer is empty, so we never clobber a new draft the user typed
  // while the turn was in flight.
  function restoreFailedQuestion(question: string): void {
    if (composerInput.trim().length === 0) {
      composerInput = question;
    }
  }

  // Retry a failed turn: re-issue the SAME question. The error turn is terminal
  // (and therefore trailing for its index), so we drop it and let send() re-run
  // the start/follow-up path — turns.length lands back on the right turnIndex, so
  // a failed first turn re-starts and a failed follow-up re-follows-up.
  async function retryTurn(turn: ChatTurn): Promise<void> {
    if (streaming || !askAvailable) return;
    // Only the trailing turn can be retried: send() re-derives turnIndex from
    // turns.length, so dropping a non-trailing turn would orphan the stream and
    // collide turnIndexes. Mid-thread errors keep their message but no Retry.
    if (turn.turnIndex !== turns.length - 1) return;
    const question = turn.question;
    turns = turns.filter((t) => t.turnIndex !== turn.turnIndex);
    composerInput = question;
    await send();
  }

  function onComposerKeydown(event: KeyboardEvent): void {
    if (event.isComposing) return;
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      void send();
    }
  }

  // ── Stop the in-flight turn (composer stop button) ───────────────────────
  // Asks the backend to cancel. The backend now ALWAYS emits a terminal `done`
  // update on cancel (Slice 4), so the `ask_ai_update` `done` settles the UI
  // (streaming flag + working line); there is no local settle hack.
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
  // Auto-scroll is PINNED: a streaming turn only drags the view to the bottom
  // when the user is already there. If they've scrolled up to read, new tokens
  // no longer yank them down — a "Jump to latest" pill appears instead so the
  // jump is theirs to take. `atBottom` is recomputed from the scroll position;
  // `BOTTOM_EPSILON_PX` tolerates sub-pixel rounding + the bottom padding.
  const BOTTOM_EPSILON_PX = 40;
  let atBottom = $state(true);

  function scrollTranscriptToBottom(): void {
    const el = transcriptEl;
    if (el === null) return;
    el.scrollTop = el.scrollHeight;
    atBottom = true;
  }

  // Recompute the pinned flag from the live scroll position (transcript onscroll).
  function onTranscriptScroll(): void {
    const el = transcriptEl;
    if (el === null) return;
    atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= BOTTOM_EPSILON_PX;
  }

  // Auto-scroll ONLY when pinned to the bottom; otherwise leave the user where
  // they are (the "Jump to latest" pill handles catching up).
  function maybeAutoScroll(): void {
    if (atBottom) void tick().then(scrollTranscriptToBottom);
  }

  function jumpToLatest(): void {
    void tick().then(scrollTranscriptToBottom);
  }

  function activitySummaryFor(
    toolActivities: ToolActivityEntry[],
  ): string | null {
    if (toolActivities.length === 0) return null;
    let searches = 0;
    let timelines = 0;
    let reads = 0;
    let recalls = 0;
    let others = 0;
    for (const entry of toolActivities) {
      if (entry.kind === "search") searches += 1;
      else if (entry.kind === "timeline") timelines += 1;
      else if (entry.kind === "show_text") reads += 1;
      else if (entry.kind === "recall_context") recalls += 1;
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
    if (recalls > 0)
      parts.push(`${recalls} ${recalls === 1 ? "recall" : "recalls"}`);
    if (others > 0) parts.push(`${others} ${others === 1 ? "step" : "steps"}`);
    return parts.length > 0 ? parts.join(" · ") : null;
  }

  function toggleSummary(turn: ChatTurn): void {
    turn.summaryExpanded = !turn.summaryExpanded;
  }

  // ── Copy a completed answer ──────────────────────────────────────────────
  // A quiet hover affordance on done turns copies the answer's raw Markdown (the
  // prose blocks joined; graphical blocks have no useful clipboard form), keyed
  // by turnIndex so the "Copied" flash stays scoped to the copied turn. Mirrors
  // Quick Recall's per-turn copy.
  let copiedTurnIndex = $state<number | null>(null);
  let copyResetTimer: ReturnType<typeof setTimeout> | null = null;

  function answerPlainText(turn: ChatTurn): string {
    return turn.blocks
      .filter((b): b is { kind: "prose"; markdown: string } => b.kind === "prose")
      .map((b) => b.markdown)
      .join("\n\n")
      .trim();
  }

  async function copyAnswer(turn: ChatTurn): Promise<void> {
    const text = answerPlainText(turn);
    if (text.length === 0) return;
    try {
      await navigator.clipboard.writeText(text);
      copiedTurnIndex = turn.turnIndex;
      if (copyResetTimer !== null) clearTimeout(copyResetTimer);
      copyResetTimer = setTimeout(() => {
        copiedTurnIndex = null;
        copyResetTimer = null;
      }, 1600);
    } catch {
      // Best-effort: a rejected clipboard write just leaves the button idle.
    }
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
    return (turn.reasoning ?? "").trim().length > 0;
  }
  function reasoningIsLive(turn: ChatTurn): boolean {
    return (
      (turn.reasoning ?? "").trim().length > 0 &&
      turn.blocks.length === 0 &&
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

  // In-place frame peek (FrameDetailModal). A frame source — or an audio source
  // that carries an aligned frame — opens the modal instead of hopping to the raw
  // Timeline window; the old timeline hand-off survives only as the modal's
  // demoted escape hatch (`onOpenInTimeline`) and as the fallback for audio with
  // no frame to peek.
  let frameModalOpen = $state(false);
  let frameModalId = $state<number | null>(null);
  let frameModalApp = $state<string | null>(null);
  let frameModalTitle = $state<string | null>(null);
  let frameModalCapturedAt = $state<string | null>(null);
  let frameModalOpenInTimeline = $state<(() => void) | null>(null);

  // Select an Answer Source: peek a frame in place when one is available, else
  // keep the old raw-Timeline hand-off (audio with no aligned frame).
  function selectSource(source: AskAiSource): void {
    const frameId =
      source.kind === "frame" ? source.frameId : (source.alignedFrameId ?? null);
    if (frameId == null) {
      void openSourceInTimeline(source);
      return;
    }
    frameModalId = frameId;
    frameModalApp = source.appName;
    frameModalTitle = source.windowTitle;
    frameModalCapturedAt = source.startedAt;
    frameModalOpenInTimeline = () => void openSourceInTimeline(source);
    frameModalOpen = true;
  }

  // Hand off an Answer Source to the main timeline window (frame xor audio) — the
  // legacy behavior, now the modal's escape hatch + the audio-no-frame fallback.
  async function openSourceInTimeline(source: AskAiSource): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: source.kind,
        frameId: source.frameId,
        audioSegmentId: source.audioSegmentId,
        spanStartMs: source.spanStartMs ?? null,
        alignedFrameId: source.alignedFrameId ?? null,
      });
    } catch (error) {
      // Opening the source in the timeline failed — surface it rather than
      // letting the click look like a no-op.
      const detail = humanizeError(error);
      await message(detail, {
        title: "Couldn't open in timeline",
        kind: "error",
      });
    }
  }

  // Per-source in-flight latch: `openSourceUrl` is a SINGLE function shared across
  // every frame source chip rendered in the loop, so a plain boolean would wrongly
  // disable ALL chips while one opens. Track the frameId of the in-flight open
  // instead, ignore a re-click on that same source, and disable only its chip.
  let openingFrameId = $state<number | null>(null);

  // Open the captured page behind a frame source in the default browser via the
  // shared brokered helper. Frame sources only (audio has frameId/url null). The
  // raw URL stays in Rust; the UI never sees it. The helper owns the feedback: a
  // no-openable-URL result shows a brief info note and a real opener failure
  // shows an error dialog (mirroring the timeline's "Couldn't open URL: …").
  async function openSourceUrl(source: AskAiSource): Promise<void> {
    const frameId = source.frameId;
    if (frameId == null || openingFrameId === frameId) return;
    openingFrameId = frameId;
    try {
      await openCapturedUrl(frameId);
    } finally {
      if (openingFrameId === frameId) openingFrameId = null;
    }
  }

  // ── Versioned update transport (the SOLE Ask AI stream listener) ─────────
  // The backend owns the render model and streams versioned `TurnUpdate` ops via
  // `ask_ai_update`. The frontend applies each op in order; a version gap (we
  // were detached, e.g. backgrounded) self-heals from `ask_ai_snapshot`.
  //
  // `applyUpdate` mirrors the Rust `apply_update_to_view` reducer EXACTLY so a
  // live stream and a snapshot/reload can never diverge. Mutating the $state turn
  // object here is safe (it runs in an event callback, not during render — unlike
  // the Svelte 5 render-memo gotcha that bans writes-from-template).
  function applyUpdate(turn: ChatTurn, update: TurnUpdate): void {
    switch (update.op) {
      case "phase":
        turn.phase = normalizePhase(update.phase);
        break;
      case "appendProse": {
        const last = turn.blocks[turn.blocks.length - 1];
        if (last && last.kind === "prose") {
          // Replace the last block so the $state array notices the change.
          turn.blocks = [
            ...turn.blocks.slice(0, -1),
            { kind: "prose", markdown: last.markdown + update.text },
          ];
        } else {
          turn.blocks = [
            ...turn.blocks,
            { kind: "prose", markdown: update.text },
          ];
        }
        break;
      }
      case "openBlock":
        turn.blocks = [...turn.blocks, update.block];
        break;
      case "reasoning":
        turn.reasoning = (turn.reasoning ?? "") + update.text;
        break;
      case "toolActivity":
        turn.toolActivities = [...turn.toolActivities, update.entry];
        break;
      case "liveActivity":
        turn.liveActivity = update.entry;
        break;
      case "sources":
        turn.sources = coerceSources(update.sources);
        void loadSourceThumbnails(turn.sources);
        break;
      case "contextTokens":
        turn.contextTokens = update.tokens;
        break;
      case "error":
        turn.errorMessage = update.message;
        turn.phase = "error";
        break;
      case "done":
        turn.phase = "done";
        turn.liveActivity = null;
        break;
    }
  }

  // Apply one `ask_ai_update` event to the active conversation, honouring the
  // version contract: exactly-next applies the op; a gap re-snapshots (or
  // re-hydrates if the turn already finalized); stale/duplicate is ignored.
  async function handleUpdateEvent(event: AskAiUpdateEvent): Promise<void> {
    if (event.conversationId !== activeConversationId) return;
    let turn = turns.find((t) => t.turnIndex === event.turnIndex);
    if (!turn) {
      // No local turn for this index: a turn was started on this SAME open
      // conversation from another window (e.g. Quick Recall). Hydrate the missing
      // turn from the authoritative live snapshot — the exact reattach path
      // `adoptLiveSnapshot` uses — insert it in order, then fall through so the
      // version contract below applies this and subsequent ops. A null / mismatched
      // snapshot means there is nothing to attach to, so we drop as before.
      const conversationId = event.conversationId;
      let snapshot: TurnSnapshot | null;
      try {
        snapshot = await invoke<TurnSnapshot | null>("ask_ai_snapshot", {
          request: { conversationId },
        });
      } catch {
        return;
      }
      if (activeConversationId !== conversationId) return;
      if (snapshot === null || snapshot.view.turnIndex !== event.turnIndex) return;
      // Re-check after the await: the listener fires handleUpdateEvent without
      // awaiting, so a concurrent invocation for this same missing turnIndex may
      // have hydrated it while we were suspended in ask_ai_snapshot — appending
      // again would duplicate the turn. Only the first resolver hydrates; the
      // rest fall through to the version contract below.
      turn = turns.find((t) => t.turnIndex === event.turnIndex);
      if (!turn) {
        const hydrated = makeTurn(
          snapshot.view.turnIndex,
          snapshot.view.question,
          normalizePhase(snapshot.view.phase),
        );
        turns = [...turns, hydrated].sort((a, b) => a.turnIndex - b.turnIndex);
        turn = turns.find((t) => t.turnIndex === event.turnIndex);
        if (!turn) return;
        adoptView(turn, snapshot.view, snapshot.version);
        reconcileComposer(turn);
      }
    }

    if (event.version === turn.version + 1) {
      applyUpdate(turn, event.update);
      turn.version = event.version;
      // A streamed error on the trailing turn: put the question back so it can be
      // edited + resent (the in-transcript Retry re-issues it verbatim).
      if (event.update.op === "error" && turn.turnIndex === turns.length - 1) {
        restoreFailedQuestion(turn.question);
      }
      reconcileComposer(turn);
      maybeAutoScroll();
      return;
    }
    if (event.version <= turn.version) return; // already applied / stale.

    // Gap: we missed updates (detached). Self-heal from the live snapshot.
    const conversationId = event.conversationId;
    let snapshot: TurnSnapshot | null;
    try {
      snapshot = await invoke<TurnSnapshot | null>("ask_ai_snapshot", {
        request: { conversationId },
      });
    } catch {
      return;
    }
    if (activeConversationId !== conversationId) return;
    if (snapshot !== null && snapshot.view.turnIndex === turn.turnIndex) {
      adoptView(turn, snapshot.view, snapshot.version);
      reconcileComposer(turn);
      maybeAutoScroll();
      return;
    }
    // Snapshot is null (turn already finalized/removed server-side) — fall back
    // to get_conversation and re-hydrate the matching turn.
    try {
      const convo = await invoke<Conversation | null>("get_conversation", {
        conversationId,
      });
      if (convo === null || activeConversationId !== conversationId) return;
      const fresh = convo.turns.find((t) => t.turnIndex === turn.turnIndex);
      if (fresh) {
        const hydrated = hydrateTurn(fresh);
        turns = turns.map((t) => (t.turnIndex === turn.turnIndex ? hydrated : t));
        reconcileComposer(hydrated);
      }
    } catch {
      // Best-effort: leave the turn as-is.
    }
  }

  // Keep the composer-level streaming flag + live activity line in sync with the
  // active (last) turn after an update settles it. When the turn is working but
  // has no explicit live-activity line (plain seeding/thinking, before a tool
  // call or the first answer token), synthesize the phase label so the composer
  // working line never goes blank mid-turn — mirroring the in-transcript states.
  function reconcileComposer(turn: ChatTurn): void {
    if (turn.turnIndex !== turns.length - 1) return;
    const live = turn.phase !== "done" && turn.phase !== "error";
    streaming = live;
    if (!live) {
      liveActivity = null;
    } else if (turn.liveActivity !== null) {
      liveActivity = turn.liveActivity;
    } else {
      liveActivity = {
        kind: "other",
        label:
          turn.phase === "seeding"
            ? "Searching your captures…"
            : turn.phase === "streaming"
              ? "Writing…"
              : "Thinking…",
      };
    }
  }

  // ── Stream event wiring ──────────────────────────────────────────────────
  onMount(() => {
    void loadAskAvailability();
    // MCP connectors (Workstream C): warm-on-open discovery — background-connect
    // enabled MCP servers so a turn finds their tools ready. Fire-and-forget.
    void invoke("mcp_warm_connectors").catch(() => {});
    // The shared store owns the history list + its `conversation_changed`
    // refresh listener (set up once, lives for the app session); ensureStarted()
    // is idempotent, so calling it here just kicks the first fetch.
    void conversationStore.ensureStarted();
    void loadPinnableEngines();

    let destroyed = false;
    let unlistenUpdate: (() => void) | undefined;
    let unlistenCtx: (() => void) | undefined;
    let unlistenSettings: (() => void) | undefined;

    // The SOLE Ask AI stream listener: versioned render-model updates for the
    // active conversation. Stale-thread + version guards live in handleUpdateEvent.
    listen<AskAiUpdateEvent>("ask_ai_update", (event) => {
      void handleUpdateEvent(event.payload);
    }).then((fn) => {
      if (destroyed) fn();
      else unlistenUpdate = fn;
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
      unlistenUpdate?.();
      unlistenCtx?.();
      unlistenSettings?.();
    };
  });

  onDestroy(() => {
    // The history search debounce now lives in the shared store (which outlives
    // this surface), so there is nothing list-related to tear down here.
    // Intentionally do NOT cancel the in-flight turn here. Under the
    // stateless-per-turn-over-persistent-store model (ADR 0033), a streaming
    // turn outlives this surface: the backend keeps running and persists its
    // partial, so leaving Chat and returning reattaches via hydrateConversation.
    // Only an explicit Stop (stopStreaming) or app exit ends a task. The event
    // listeners are torn down in the onMount cleanup, so no deltas reach this
    // destroyed component.
  });
</script>

<!-- Inline app chip for tool-activity lines: the backend-resolved icon (or a
     letter fallback) + the app name, matching the app-icon look used elsewhere.
     The icon path is resolved server-side (entry.appIconPath); here it is a pure
     path→URL conversion — no client resolve/batch. -->
{#snippet toolAppChip(entry: ToolActivityEntry)}
  {@const app = entry.app ?? ""}
  <span class="tool-app">
    <span class="tool-app-icon" aria-hidden="true">
      {#if entry.appIconPath}
        <img src={convertFileSrc(entry.appIconPath)} alt="" />
      {:else}
        {appIconFallback(app, app)}
      {/if}
    </span>
    <span class="tool-app-name">{app}</span>
  </span>
{/snippet}

<!-- Answer-shaped placeholder shown beneath the working line while a turn is
     seeding/thinking (before any prose block arrives), so the answer region
     reads as "an answer is forming" rather than a lone pulsing dot. -->
{#snippet answerSkeleton()}
  <div class="answer-sk" aria-hidden="true">
    <Skeleton variant="text" width="94%" height="12px" />
    <Skeleton variant="text" width="68%" height="12px" muted />
  </div>
{/snippet}

<!-- The active conversation: transcript + composer. The history list / search /
     new-chat / rename / delete all live in the persistent shell rail
     (<InsightsRail>); this surface is JUST the conversation pane. -->
<section class="chat" aria-label="Chat">
{#if activeConversationId === null}
  <div class="pane-empty">
    <p class="pane-empty-title">Ask the engine about your activity</p>
    <p class="pane-empty-detail">
      Pick a conversation on the left, or start a new chat. Answers draw on
      your history through the engine's brokered tools and can include inline
      charts.
    </p>
    {#if askAvailable}
      <button type="button" class="btn btn--accent" onclick={() => startNewChat()}>
        ＋ New chat
      </button>
    {:else}
      <!-- Engine-off + no conversation: surface the same enable affordance the
           composer's engine-off card shows, so the empty pane isn't a dead end. -->
      <button type="button" class="btn btn--accent" onclick={enableEngine}>
        Enable engine
      </button>
    {/if}
  </div>
{:else}
  <!-- ONLY this transcript scrolls (not the page). -->
  <div
    class="transcript"
    bind:this={transcriptEl}
    aria-live="polite"
    onscroll={onTranscriptScroll}
  >
    <!-- Centered conversation column: user question right, AI answer left. -->
    <div class="thread-col">
      {#if loadingConversation}
        <div class="convo-skeleton">
          <Skeleton width="55%" height="13px" radius="6px" />
          <Skeleton width="100%" height="48px" radius="8px" muted />
        </div>
      {:else if conversationLoadError}
        <div class="thread-load-error" role="alert">
          <p class="thread-empty-title">Couldn't open this conversation.</p>
          <p class="thread-empty-detail">{conversationLoadError}</p>
          <button
            type="button"
            class="btn btn--accent thread-retry"
            onclick={() => {
              const id = activeConversationId;
              if (id !== null) void loadConversationById(id);
            }}
          >
            ↻ Try again
          </button>
        </div>
      {:else if turns.length === 0}
        <div class="thread-empty">
          <p class="thread-empty-title">{displayTitle || "New chat"}</p>
          <p class="thread-empty-detail">
            Type a question below and press Enter. The engine searches your
            captures through its brokered tools to answer.
          </p>
          <!-- Quiet example questions: tapping one prefills the composer to
               review/edit (it does NOT auto-send). -->
          <div class="example-row" role="presentation">
            {#each EXAMPLE_QUESTIONS as example (example)}
              <button
                type="button"
                class="example-q"
                onclick={() => useExample(example)}
              >
                {example}
              </button>
            {/each}
          </div>
        </div>
      {:else}
        {#each turns as turn, ti (ti)}
          <!-- Document View render fork (ADR 0058): the FIRST turn of an
               origin=trigger conversation renders as a titled document — no
               question bubble (the "question" is the automated firing prompt,
               not user text). An errored first run falls back to the plain
               chat turn (honest error, no special document-error chrome). -->
          {@const docTurn =
            isTriggerDoc && turn.turnIndex === 0 && turn.phase !== "error"}
          {#if isTriggerDoc && ti === 1 && turns[0]?.phase !== "error"}
            <!-- Follow-ups continue the same conversation as normal chat. -->
            <div class="followup-divider" role="presentation">follow-up</div>
          {/if}
          <article class="turn">
            {#if docTurn}
              <!-- Document header: eyebrow (trigger identity), title, ran-at
                   metadata, accent rule — per the final Triggers design. -->
              <header class="doc-head">
                <div class="doc-eyebrow">
                  <span>trigger run</span>
                  {#if activeTriggerName}
                    <span class="doc-sep" aria-hidden="true">·</span>
                    <span class="doc-tname">{activeTriggerName}</span>
                  {/if}
                </div>
                <h1 class="doc-title">{displayTitle}</h1>
                {#if docRanAt}
                  <div class="doc-meta">ran {docRanAt}</div>
                {/if}
                <hr class="doc-rule" />
              </header>
            {:else}
              <!-- USER question: right-aligned bubble -->
              <div class="msg msg-user">
                <div class="user-bubble" use:tip={turn.question}>
                  {turn.question}
                </div>
              </div>
            {/if}

            <!-- ASSISTANT answer: left-aligned -->
            <div class="msg msg-assistant">
              <div class="answer-col" class:answer-col--doc={docTurn}>
                {#if turn.phase === "error"}
                  <div class="turn-error" role="alert">
                    <p class="state state--error">
                      {turn.errorMessage ?? "The engine couldn't answer."}
                    </p>
                    <!-- Re-issue the same question. The composer is also restored
                         with the question, so this and a manual edit-and-resend
                         both work. Retry is gated to the TRAILING turn: send()
                         re-derives turnIndex from turns.length, so retrying a
                         mid-thread error would collide turnIndexes and orphan
                         the stream. -->
                    {#if ti === turns.length - 1}
                      <button
                        type="button"
                        class="turn-retry"
                        disabled={streaming || !askAvailable}
                        onclick={() => void retryTurn(turn)}
                      >
                        <span class="turn-retry-ico" aria-hidden="true">↻</span>
                        Retry
                      </button>
                    {/if}
                  </div>
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

                  {#if turn.phase === "seeding"}
                    <p class="state state--working">
                      <span class="dot" aria-hidden="true"></span>
                      Searching your captures…
                    </p>
                    {@render answerSkeleton()}
                  {:else if turn.phase === "thinking" && turn.liveActivity === null}
                    <p class="state state--working">
                      <span class="dot" aria-hidden="true"></span>
                      Thinking…
                    </p>
                    {@render answerSkeleton()}
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
                                    in {@render toolAppChip(activity)}
                                  {/if}
                                </li>
                              {/each}
                            </ul>
                          {/if}
                        </div>
                      {/if}

                      <!-- The answer body: render-ready blocks streamed from
                           the backend, switched on `kind`. Prose carries raw
                           markdown (AnswerProse renders + hardens it); the
                           graphical blocks carry already-parsed data. The
                           streaming caret rides only the LAST prose block, and
                           only until the turn settles. -->
                      <div class="answer">
                        {#each turn.blocks as block, bi (bi)}
                            {#if block.kind === "prose"}
                              <AnswerProse
                                source={block.markdown}
                                isStreaming={turn.phase !== "done" &&
                                  bi === turn.blocks.length - 1}
                              />
                            {:else if block.kind === "bars"}
                              <figure class="graphic">
                                {#if block.title}
                                  <figcaption class="graphic-title">
                                    {block.title}
                                  </figcaption>
                                {/if}
                                <MiniBars items={block.items} />
                              </figure>
                            {:else if block.kind === "dossier"}
                              <div class="graphic graphic--dossier">
                                {#each block.items as item, di (di)}
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
                            {:else if block.kind === "timeline"}
                              <!-- Timeline owns its own caption (the same
                                   uppercase-muted .timeline-title idiom as
                                   .graphic-title), so the .graphic wrapper
                                   here doesn't repeat it as a figcaption. -->
                              <figure class="graphic">
                                <Timeline title={block.title} intervals={block.items} />
                              </figure>
                            {/if}
                        {/each}
                      </div>

                      <!-- Quiet hover Copy on a completed answer (raw Markdown).
                           Always in the DOM for keyboard reach; CSS reveals it on
                           turn hover/focus. -->
                      {#if turn.phase === "done" && answerPlainText(turn).length > 0}
                        <div class="answer-tools">
                          <button
                            type="button"
                            class="answer-copy"
                            class:is-copied={copiedTurnIndex === turn.turnIndex}
                            onclick={() => void copyAnswer(turn)}
                          >
                            {copiedTurnIndex === turn.turnIndex ? "✓ Copied" : "Copy"}
                          </button>
                        </div>
                      {/if}

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
                                    url={s.url}
                                    onselect={() => void selectSource(s)}
                                    onopenurl={() => openSourceUrl(s)}
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
                                    url={s.url}
                                    onselect={() => void selectSource(s)}
                                  />
                                {/each}
                              </div>
                            </div>
                          {/if}
                        </div>
                      {/if}
                    {/if}

                    {#if turn.liveActivity !== null}
                      <p class="state state--working">
                        <span class="dot" aria-hidden="true"></span>
                        <span class="working-label"
                          >{turn.liveActivity.label}</span
                        >
                        {#if turn.liveActivity.app}
                          in
                          {@render toolAppChip(turn.liveActivity)}
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

  <!-- "Jump to latest" — only when the user has scrolled up off the bottom while
       there's content (streaming no longer yanks them down; this is their opt-in
       to catch up). Anchored just above the composer. -->
  {#if !atBottom && turns.length > 0}
    <button
      type="button"
      class="jump-latest"
      onclick={jumpToLatest}
      aria-label="Jump to latest"
    >
      ↓ Jump to latest
    </button>
  {/if}

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
            in {@render toolAppChip(liveActivity)}
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
          placeholder={isTriggerDoc
            ? "Ask a follow-up about this run…"
            : "Ask about your activity…"}
          aria-label={isTriggerDoc
            ? "Ask a follow-up about this run"
            : "Ask about your activity"}
          disabled={streaming}
          onkeydown={onComposerKeydown}
        ></textarea>
        <div class="composer-bar">
          <!-- Per-thread model pin (searchable, provider-grouped picker).
               Owns its own UI + model pool; reports the chosen engine up. -->
          <ModelPicker
            aiRuntime={aiRuntimeSnapshot}
            {askAiModelOverride}
            pinProvider={activePinProvider}
            pinModel={activePinModel}
            bind:open={enginePickerOpen}
            onselect={handleModelSelect}
          />
          <!-- Context-window occupancy for this thread (provider-reported
               tokens from the latest answer). Hidden until a turn reports. -->
          {#if contextTokens !== null}
            <span
              class="composer-context"
              use:tip={contextWindow !== null && contextFraction !== null
                ? `${formatTokenCount(contextTokens)} of ${formatTokenCount(contextWindow)} tokens (${Math.round(contextFraction * 100)}% of context window)`
                : "Tokens in the model's context window"}
            >
              {#if contextFraction !== null}
                <!-- Occupancy ring, shown only when the model's total window is
                     known: dasharray fills the circumference (2πr, r=6.5 →
                     ~40.84) by the used fraction, starting at 12 o'clock. -->
                <svg class="composer-context-ring" viewBox="0 0 16 16" aria-hidden="true">
                  <circle class="ring-track" cx="8" cy="8" r="6.5" />
                  <circle
                    class="ring-fill"
                    cx="8"
                    cy="8"
                    r="6.5"
                    stroke-dasharray="{(contextFraction * 40.84).toFixed(2)} 40.84"
                    transform="rotate(-90 8 8)"
                  />
                </svg>
              {/if}
              {formatTokenCount(contextTokens)} tokens
            </span>
          {/if}
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
            use:tip={streaming ? "Stop generating" : "Send (Enter)"}
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
</section>

<!-- In-place frame peek for Answer Sources. Its "open full timeline →" escape
     hatch replays the old raw-Timeline hand-off captured at open time. -->
<FrameDetailModal
  open={frameModalOpen}
  frameId={frameModalId}
  appName={frameModalApp}
  windowTitle={frameModalTitle}
  capturedAt={frameModalCapturedAt}
  onClose={() => (frameModalOpen = false)}
  onOpenInTimeline={frameModalOpenInTimeline ?? undefined}
/>

<style>
  /* The conversation pane filling the insights surface. Mirrors the terminal/
     green token system (--app-* / --cat-*) used across Overview/Subjects/Context.
     The surface fills the full height and OWNS its scrolling: only `.transcript`
     scrolls — the page itself never does (the Insights shell drops its padding/
     overflow for the chat tab via `.insights-main--chat`). The history list now
     lives in the persistent shell rail, so this is a single column. */
  .chat {
    display: flex;
    flex-direction: column;
    min-width: 0;
    /* Fill the flex-column parent (`.insights-main--chat`) via flex-grow, NOT a
       percentage height — WKWebView (Tauri) drops `height: 100%` against a
       flex-stretched ancestor, which collapsed the surface to content height. */
    flex: 1 1 auto;
    min-height: 0;
    border-top: 1px solid var(--app-border);
    overflow: hidden;
    background: var(--app-surface);
    /* Anchor for the floating "Jump to latest" pill. */
    position: relative;
  }

  /* Floating "Jump to latest" pill — sits just above the composer, centered. */
  .jump-latest {
    position: absolute;
    left: 50%;
    bottom: 96px;
    transform: translateX(-50%);
    z-index: 4;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: 11px;
    padding: 5px 12px;
    border: 1px solid var(--app-accent-border);
    border-radius: 999px;
    background: var(--app-surface-raised);
    color: var(--app-accent-strong);
    cursor: pointer;
    box-shadow: var(--app-shadow-popover, 0 4px 14px rgba(0, 0, 0, 0.35));
    transition:
      border-color 0.12s ease,
      background 0.12s ease;
  }
  .jump-latest:hover {
    border-color: var(--app-accent);
    background: var(--app-surface-hover);
  }
  .jump-latest:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .jump-latest:active {
    transform: translateX(-50%) translateY(1px);
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
  /* Recoverable load-failure state for a thread that threw while opening. */
  .thread-load-error {
    margin: 4px 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 16px;
    border: 1px solid var(--app-danger-border);
    border-radius: 9px;
    background: var(--app-danger-bg);
  }
  .thread-retry {
    margin-top: 2px;
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
  /* Quiet example-question chips: the same pill look as .activity-chip. Tapping
     one prefills the composer (review/edit, not send). */
  .example-row {
    display: flex;
    flex-wrap: wrap;
    gap: 7px;
    margin-top: 4px;
  }
  .example-q {
    font: inherit;
    font-size: 11px;
    letter-spacing: 0.01em;
    padding: 5px 11px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    transition: border-color 0.12s ease, color 0.12s ease;
  }
  .example-q:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .example-q:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }

  /* ── Document View (ADR 0058): origin=trigger first turn ─────────────────
     Titled full-width page per the final Triggers design (triggers-ui.html
     Screen 4): accent uppercase eyebrow, 26px title, ran-at metadata, 2px
     accent rule, then the answer body with report typography. */
  .doc-head {
    margin-bottom: 2px;
  }
  .doc-eyebrow {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-size: 9.5px;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--app-accent-strong);
    margin-bottom: 10px;
  }
  .doc-tname {
    color: var(--app-text-muted);
  }
  .doc-sep {
    color: var(--app-text-faint);
  }
  .doc-title {
    margin: 0 0 6px;
    font-size: 26px;
    line-height: 1.25;
    font-weight: 650;
    letter-spacing: -0.02em;
    color: var(--app-text-strong);
  }
  .doc-meta {
    font-size: 11px;
    color: var(--app-text-muted);
  }
  .doc-rule {
    height: 2px;
    background: var(--app-accent);
    opacity: 0.7;
    border: 0;
    margin: 16px 0 0;
  }
  /* Report typography for the document body: h2 reads as an uppercase section
     header with a dashed rule (the mockup's .doc-body h2). Scoped under the
     doc modifier so normal chat prose is untouched. */
  .answer-col--doc :global(.answer-prose) {
    line-height: 1.7;
  }
  .answer-col--doc :global(.answer-prose h2) {
    margin: 26px 0 10px;
    font-size: 13px;
    font-weight: 650;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--app-text-strong);
    padding-bottom: 6px;
    border-bottom: 1px dashed var(--app-border);
  }
  .answer-col--doc :global(.answer-prose > h2:first-child) {
    margin-top: 0;
  }

  /* "FOLLOW-UP" divider between the document and the chat turns beneath it. */
  .followup-divider {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 4px 0 -4px;
    font-size: 9.5px;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--app-text-subtle);
  }
  .followup-divider::before,
  .followup-divider::after {
    content: "";
    flex: 1 1 auto;
    height: 1px;
    background: var(--app-border);
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
  /* Failed-turn block: the error line + a Retry that re-issues the question. */
  .turn-error {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
  }
  .turn-retry {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: 11px;
    padding: 4px 11px;
    border: 1px solid var(--app-danger-border);
    border-radius: 7px;
    background: var(--app-danger-bg);
    color: var(--app-danger-text);
    cursor: pointer;
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease,
      opacity 0.12s ease;
  }
  .turn-retry:hover:not(:disabled) {
    border-color: var(--app-danger);
  }
  .turn-retry:focus-visible {
    outline: none;
    box-shadow: var(--app-ring-danger);
  }
  .turn-retry:not(:disabled):active {
    transform: translateY(1px);
  }
  .turn-retry:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .turn-retry-ico {
    font-size: 12px;
    line-height: 1;
  }

  /* Quiet hover Copy on a completed answer. Hidden until the turn is hovered or
     the button itself is focused (keyboard reach), then a quiet pill. */
  .answer-tools {
    display: flex;
    margin-top: 2px;
    min-height: 18px;
  }
  .answer-copy {
    font: inherit;
    font-size: 10.5px;
    letter-spacing: 0.02em;
    padding: 2px 9px;
    border: 1px solid var(--app-border);
    border-radius: 999px;
    background: var(--app-surface-subtle);
    color: var(--app-text-muted);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.12s ease,
      border-color 0.12s ease,
      color 0.12s ease;
  }
  .turn:hover .answer-copy,
  .answer-copy:focus-visible,
  .answer-copy.is-copied {
    opacity: 1;
  }
  .answer-copy:hover {
    border-color: var(--app-border-hover);
    color: var(--app-text-strong);
  }
  .answer-copy:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .answer-copy.is-copied {
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  @media (prefers-reduced-motion: reduce) {
    .turn-retry:not(:disabled):active {
      transform: none;
    }
    /* Keep the pill centered (translateX) but drop the press-down nudge. */
    .jump-latest:active {
      transform: translateX(-50%);
    }
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
    box-shadow: var(--app-ring);
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
  .activity-chip:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
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
  /* Answer-shaped placeholder while a turn seeds/thinks (before prose lands). */
  .answer-sk {
    display: flex;
    flex-direction: column;
    gap: 7px;
    margin-top: 8px;
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
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .composer:focus-within {
    border-color: var(--app-accent-border);
    box-shadow: var(--app-ring);
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
  /* Quiet context-window readout, tucked against the send button. */
  .composer-context {
    flex: 0 0 auto;
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-family: var(--app-font-mono, ui-monospace, monospace);
    font-size: 10.5px;
    color: var(--app-text-muted);
    cursor: default;
  }
  .composer-context-ring {
    width: 12px;
    height: 12px;
  }
  .composer-context-ring circle {
    fill: none;
    stroke-width: 2.5;
  }
  .composer-context-ring .ring-track {
    stroke: currentColor;
    opacity: 0.25;
  }
  .composer-context-ring .ring-fill {
    stroke: currentColor;
    stroke-linecap: round;
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
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      color 0.12s ease,
      box-shadow 0.12s ease,
      opacity 0.12s ease;
  }
  .composer-send:hover:not(:disabled) {
    border-color: var(--app-accent);
  }
  .composer-send:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .composer-send:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  /* The send glyph morphs into a stop square while a turn streams. The Stop
     action must not read as the affirmative green Send — give it a neutral
     danger palette so interrupting is visually distinct. */
  .composer-send--stop {
    font-size: 10px;
    border-color: var(--app-danger-border);
    background: var(--app-danger-bg);
    color: var(--app-danger-text);
  }
  .composer-send--stop:hover:not(:disabled) {
    border-color: var(--app-danger);
  }
  .composer-send--stop:focus-visible {
    box-shadow: var(--app-ring-danger);
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
    transition:
      border-color 0.12s ease,
      box-shadow 0.12s ease;
  }
  .engine-off-enable:hover {
    border-color: var(--app-accent);
  }
  .engine-off-enable:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .engine-off-enable:not(:disabled):active {
    transform: translateY(1px);
  }

  /* Shared accent button (mirrors Overview's .btn--accent). */
  .btn {
    font: inherit;
    font-size: 11.5px;
    padding: 7px 14px;
    border-radius: 7px;
    cursor: pointer;
    border: 1px solid transparent;
    transition:
      border-color 0.12s ease,
      background 0.12s ease,
      box-shadow 0.12s ease,
      filter 0.12s ease,
      transform 0.06s ease;
  }
  .btn:focus-visible {
    outline: none;
    box-shadow: var(--app-ring);
  }
  .btn:not(:disabled):active {
    transform: translateY(1px);
  }
  .btn--accent {
    background: var(--app-accent-bg);
    border-color: var(--app-accent-border);
    color: var(--app-accent-strong);
  }
  .btn--accent:hover {
    border-color: var(--app-accent);
  }
  .btn--accent:not(:disabled):active {
    filter: brightness(0.95);
  }
</style>
