<script lang="ts">
  // Chat — the conversation pane of the Chat surface (`/chat`; still mounted by
  // the Insights Chat tab until IN1 removes it). ADR 0031/0033: conversations
  // persist in the shared store and are answered by the Ask AI engine, which
  // reaches capture data only through brokered tools. The backend owns every
  // turn's render model and streams versioned `ask_ai_update` ops; this pane
  // only renders (docs/agents/ask-ai-streaming.md). Quick Recall persists to the
  // same store, so its threads open and continue here under the same id.
  //
  // What to show is driven by the shared store's selection BUS (`pendingOpen`):
  // the chat list, ⌘J, `/chat?c=` / `?q=` and handoffs request; Chat loads.
  import { onMount, tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { tip } from "$lib/components/tooltip";
  import { openSettings } from "$lib/surface-windows";
  import { askAiClock } from "$lib/askAiClock";
  import { humanizeError } from "$lib/format-error";
  import {
    type Conversation,
    type AskAiAvailability,
    type TurnSnapshot,
    type AskAiUpdateEvent,
    type ConversationSummary,
    contextWindowForModel,
    defaultEngineModel,
    defaultEnginePinProvider,
    providerLabelById,
    shortModelLabel,
  } from "$lib/insights/conversation";
  import { conversationStore as store } from "$lib/insights/conversationStore.svelte";
  import { isReasonCode, reasonCopy } from "$lib/insights/engine-state";
  import type { AiRuntimeModel, AiRuntimeModelsResult, AiRuntimeSettings, RecordingSettings, RecordingSettingsDomainUpdateResponse } from "$lib/types/recording";
  import ChatTurnView from "./ChatTurn.svelte";
  import ChatComposer from "./ChatComposer.svelte";
  import ChatEmpty from "./ChatEmpty.svelte";
  import { chatWhen, providerWhere } from "./chat-format";
  import { adoptView, applyUpdate, hydrateTurn, makeTurn, normalizePhase, type ChatTurn } from "./turn-model";
  import IconEdit from "~icons/lucide/pencil";
  import IconTrash from "~icons/lucide/trash-2";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconDown from "~icons/lucide/arrow-down";

  interface Props {
    /** OUT: what an unpinned chat resolves to (the list footer shows it). */
    defaultModel?: string | null;
    /** A chat was deleted from the header (the page shows the toast). */
    ondeleted?: () => void;
  }
  let { defaultModel = $bindable(null), ondeleted }: Props = $props();

  const TITLE_MAX = 80;

  // ── Ask AI availability ──────────────────────────────────────────────────
  let askAvailability = $state<AskAiAvailability | null>(null);
  const askAvailable = $derived(askAvailability?.available === true);

  async function loadAskAvailability(): Promise<void> {
    try {
      askAvailability = await invoke<AskAiAvailability>("ask_ai_availability");
    } catch (error) {
      askAvailability = { available: false, reason: humanizeError(error) };
    }
  }

  // The real reason in plain words + one action (CH-01/CH-07), never a raw code.
  const composerOff = $derived.by(() => {
    const reason = askAvailability?.reason ?? "";
    if (reason === "ask_ai_disabled") {
      return { text: "Ask AI is off. Past chats stay readable here.", action: "Turn on in Settings →", warn: false, retry: false };
    }
    if (reason === "ai_runtime_disabled" || reason === "no_providers") {
      return { text: "The reasoning engine is off. Chat answers over your history once it's enabled.", action: "Open settings", warn: false, retry: false };
    }
    const copy = reasonCopy(reason, aiRuntime);
    return copy.kind === "unreachable"
      ? { text: copy.text, action: "Check again", warn: true, retry: true }
      : { text: copy.text, action: "Open settings", warn: true, retry: false };
  });

  // ── Engine settings + the per-chat model pin (ADR 0034) ──────────────────
  let aiRuntime = $state<AiRuntimeSettings | null>(null);
  let askAiModelOverride = $state<string | null>(null);
  let pinProvider = $state<string | null>(null);
  let pinModel = $state<string | null>(null);
  let pickerOpen = $state(false);

  async function loadEngineSettings(): Promise<void> {
    try {
      const settings = await invoke<RecordingSettings>("get_recording_settings");
      aiRuntime = settings.aiRuntime;
      const override = settings.access?.askAiModel?.trim() ?? "";
      askAiModelOverride = override.length > 0 ? override : null;
    } catch {
      aiRuntime = null;
      askAiModelOverride = null;
    }
  }

  // Same precedence as the backend resolver: pin → Ask AI override → default.
  const engineProvider = $derived(pinProvider ?? (aiRuntime ? defaultEnginePinProvider(aiRuntime) : null));
  const engineModel = $derived(pinModel ?? askAiModelOverride ?? (aiRuntime ? defaultEngineModel(aiRuntime) : null));
  const engineWhere = $derived(engineProvider === null ? null : providerWhere(aiRuntime?.providers.find((p) => p.id === engineProvider)));
  const sendTo = $derived(engineProvider === null ? null : providerLabelById(aiRuntime?.providers, engineProvider));
  $effect(() => {
    defaultModel = askAiModelOverride ?? (aiRuntime ? defaultEngineModel(aiRuntime) : null);
  });

  async function handleModelSelect(engine: { provider: string; model: string } | null): Promise<void> {
    const conversationId = activeConversationId;
    pinProvider = engine?.provider ?? null;
    pinModel = engine?.model ?? null;
    // No row exists before the first turn; persisting now would upsert a phantom
    // "Untitled chat". send() persists the pin with the first turn.
    if (conversationId === null || turns.length === 0) return;
    try {
      await invoke("set_conversation_engine", {
        request: { conversationId, provider: engine?.provider ?? null, model: engine?.model ?? null },
      });
    } catch {
      // Best-effort: the pin is re-read on the next hydrate.
    }
  }

  // ── The active conversation ──────────────────────────────────────────────
  let activeConversationId = $state<string | null>(null);
  let activeTitle = $state("");
  let activeCreatedAtMs = $state<number | null>(null);
  let turns = $state<ChatTurn[]>([]);
  let loadingConversation = $state(false);
  // A failed open renders a recoverable error, never an empty "new chat".
  let conversationLoadError = $state<string | null>(null);
  // `get_conversation` found nothing under the id: "this chat was deleted".
  let missingConversation = $state(false);
  let streaming = $state(false);
  let composerInput = $state("");
  let composer = $state<ReturnType<typeof ChatComposer> | null>(null);
  let transcriptEl = $state<HTMLDivElement | null>(null);

  // Provider-reported context windows for the per-answer token bar: the active
  // provider's listing, fetched once and only once usage is on screen.
  let providerModels = $state<Record<string, AiRuntimeModel[]>>({});
  const providerModelsRequested = new Set<string>();
  const hasUsage = $derived(turns.some((t) => t.contextTokens !== null));
  $effect(() => {
    const provider = engineProvider;
    if (provider === null || !hasUsage || providerModelsRequested.has(provider)) return;
    const config = aiRuntime?.providers.find((p) => p.id === provider);
    if (config === undefined) return;
    providerModelsRequested.add(provider);
    void invoke<AiRuntimeModelsResult>("ai_runtime_list_models", { request: { providers: [$state.snapshot(config)] } })
      .then((result) => (providerModels[provider] = result.models))
      .catch(() => {});
  });
  const contextWindow = $derived.by(() => {
    if (engineModel === null) return null;
    const reported = engineProvider ? providerModels[engineProvider]?.find((m) => m.id === engineModel)?.contextWindow : null;
    return reported ?? contextWindowForModel(engineModel);
  });

  // One-way mirror up to the store (the list highlights the open row).
  $effect(() => {
    store.activeConversationId = activeConversationId;
  });
  const activeSummary = $derived(store.conversations.find((c) => c.conversationId === activeConversationId));
  const displayTitle = $derived(activeSummary?.title || activeTitle || "New chat");
  // While opening (or failed), the header shows the list row's numbers.
  const headCount = $derived(loadingConversation || conversationLoadError !== null ? (activeSummary?.turnCount ?? 0) : turns.length);
  const headStarted = $derived(activeCreatedAtMs ?? activeSummary?.createdAtMs ?? null);
  const mode = $derived(
    loadingConversation || conversationLoadError !== null ? "chat" : missingConversation ? "missing" : turns.length === 0 ? "new" : "chat",
  );

  function titleFromQuestion(question: string): string {
    const t = question.trim().replace(/\s+/g, " ");
    return t.length > TITLE_MAX ? `${t.slice(0, TITLE_MAX - 1)}…` : t;
  }

  function resetPane(id: string | null): void {
    activeConversationId = id;
    activeTitle = "";
    activeCreatedAtMs = null;
    turns = [];
    conversationLoadError = null;
    missingConversation = false;
    streaming = false;
    pinProvider = null;
    pinModel = null;
    pickerOpen = false;
  }

  // `prefill` seeds the composer to review/edit; it is sent only via the bus's
  // `send` flag (the shell's ask field).
  function startNewChat(prefill: string | null = null): void {
    resetPane(crypto.randomUUID());
    composerInput = prefill ?? "";
    composer?.focus();
  }

  async function loadConversationById(conversationId: string): Promise<void> {
    const id = conversationId.trim();
    if (id.length === 0) return;
    if (id === activeConversationId && turns.length > 0) return;
    resetPane(id);
    loadingConversation = true;
    try {
      const convo = await invoke<Conversation | null>("get_conversation", { conversationId: id });
      if (activeConversationId !== id) return;
      if (convo === null) {
        missingConversation = true;
        return;
      }
      await hydrateConversation(convo);
      if (activeConversationId !== id) return;
      await tick();
      scrollToBottom();
    } catch (error) {
      if (activeConversationId === id) conversationLoadError = humanizeError(error);
    } finally {
      if (activeConversationId === id) loadingConversation = false;
    }
  }

  async function hydrateConversation(convo: Conversation): Promise<void> {
    activeTitle = convo.title;
    activeCreatedAtMs = convo.createdAtMs;
    pinProvider = convo.provider ?? null;
    pinModel = convo.model ?? null;
    turns = convo.turns.map(hydrateTurn);
    // A persisted "streaming" last turn is still in flight; the snapshot below
    // replaces it with the authoritative live view + version.
    streaming = turns.at(-1)?.phase === "streaming";
    await adoptLiveSnapshot(convo.conversationId);
  }

  async function fetchSnapshot(conversationId: string): Promise<TurnSnapshot | null> {
    try {
      return await invoke<TurnSnapshot | null>("ask_ai_snapshot", { request: { conversationId } });
    } catch {
      return null;
    }
  }

  // Snapshot-on-attach: adopt the in-flight LiveTurn (if any) race-free.
  async function adoptLiveSnapshot(conversationId: string): Promise<void> {
    const snapshot = await fetchSnapshot(conversationId);
    if (snapshot === null || activeConversationId !== conversationId) return;
    const turn = turns.find((t) => t.turnIndex === snapshot.view.turnIndex);
    if (!turn) return;
    adoptView(turn, snapshot.view, snapshot.version);
    streaming = turn.phase !== "done" && turn.phase !== "error";
  }

  // The selection bus. Each request bumps `pendingOpen.nonce`; a remount replays
  // the latest request once (lastOpenNonce starts at 0), which is how leaving
  // and returning reattaches to the open chat.
  let lastOpenNonce = 0;
  let pendingSend = $state(false);
  $effect(() => {
    const pending = store.pendingOpen;
    untrack(() => {
      if (pending.nonce === 0 || pending.nonce === lastOpenNonce) return;
      lastOpenNonce = pending.nonce;
      if (pending.id !== null) {
        void loadConversationById(pending.id);
        return;
      }
      startNewChat(pending.prefill);
      if (pending.send) {
        pendingSend = true;
        store.settleOpen(null); // consumed: a remount must not send it again
      }
    });
  });
  // `?q=` sends once availability is known; if Ask AI can't run, the question
  // just stays in the composer.
  $effect(() => {
    if (!pendingSend || askAvailability === null) return;
    untrack(() => {
      pendingSend = false;
      if (askAvailable) void send();
    });
  });

  // ── Sending ──────────────────────────────────────────────────────────────
  async function send(): Promise<void> {
    const question = composerInput.trim();
    if (question.length === 0 || streaming || !askAvailable) return;
    if (activeConversationId === null) activeConversationId = crypto.randomUUID();
    const conversationId = activeConversationId;
    const nonce = lastOpenNonce;
    // A trailing local-only error has no backend row; drop it so turnIndex
    // matches the backend's turn count (CH-04).
    if (turns.at(-1)?.localOnly) turns = turns.slice(0, -1);
    const isFirstTurn = turns.length === 0;
    if (isFirstTurn && activeTitle.length === 0) activeTitle = titleFromQuestion(question);
    const title = activeTitle || titleFromQuestion(question);
    composerInput = "";
    // Settle any prior turn still flagged working (a displaced turn that never
    // got its terminal update would keep a live line under a finished answer).
    for (const t of turns) {
      if (t.phase === "streaming" || t.phase === "thinking") {
        t.liveActivity = null;
        t.phase = "done";
      }
    }
    const turnIndex = turns.length;
    const turn = makeTurn(turnIndex, question, "thinking");
    turn.startedAtMs = turn.atMs = Date.now();
    turns = [...turns, turn];
    streaming = true;
    await tick();
    scrollToBottom();
    try {
      if (isFirstTurn) {
        // Persist a pin chosen before the row existed, BEFORE the turn reads it.
        if (pinProvider !== null && pinModel !== null) {
          await invoke("set_conversation_engine", { request: { conversationId, provider: pinProvider, model: pinModel } }).catch(() => {});
        }
        await invoke<void>("ask_ai_start", {
          request: { conversationId, question, origin: "chat", title, ...askAiClock() },
        });
        if (activeCreatedAtMs === null) activeCreatedAtMs = Date.now();
        // The bus now means "this chat": a remount reopens it instead of a blank one.
        if (store.pendingOpen.nonce === nonce) store.settleOpen(conversationId);
      } else {
        // The backend reloads history from the store, so a follow-up always works.
        await invoke<void>("ask_ai_followup", { request: { conversationId, question, ...askAiClock() } });
      }
    } catch (error) {
      if (activeConversationId !== conversationId) return;
      streaming = false;
      const t = turns[turnIndex];
      if (t) {
        t.phase = "error";
        // A raw resolve code stays as-is for turnErrorCopy to map.
        t.errorMessage = typeof error === "string" && isReasonCode(error) ? error : humanizeError(error);
        t.localOnly = true;
      }
      restoreFailedQuestion(question);
    }
  }

  // Put a failed question back, unless the user already typed something new.
  function restoreFailedQuestion(question: string): void {
    if (composerInput.trim().length === 0) composerInput = question;
  }

  // Retry the trailing failed turn: send() derives turnIndex from turns.length,
  // so only the last turn can be retried (CH-04).
  async function retryTurn(turn: ChatTurn): Promise<void> {
    if (streaming || !askAvailable || turn.turnIndex !== turns.length - 1) return;
    composerInput = turn.question;
    await send();
  }

  // The backend always emits a terminal `done` on cancel, which settles the UI.
  async function stopStreaming(): Promise<void> {
    if (activeConversationId === null || !streaming) return;
    await invoke<void>("ask_ai_cancel", { request: { conversationId: activeConversationId } }).catch(() => {});
  }

  // ── Scrolling: pinned to the bottom only while the user is there ─────────
  let atBottom = $state(true);
  function scrollToBottom(): void {
    if (transcriptEl === null) return;
    transcriptEl.scrollTop = transcriptEl.scrollHeight;
    atBottom = true;
  }
  function onTranscriptScroll(): void {
    const el = transcriptEl;
    if (el !== null) atBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 40;
  }

  // ── The versioned update transport (the SOLE Ask AI stream listener) ─────
  // Exactly-next applies the op; a gap re-snapshots (or re-hydrates a finalized
  // turn); stale/duplicate is ignored.
  async function handleUpdateEvent(event: AskAiUpdateEvent): Promise<void> {
    const conversationId = event.conversationId;
    if (conversationId !== activeConversationId) return;
    let turn = turns.find((t) => t.turnIndex === event.turnIndex);
    if (!turn) {
      // A turn started on this open chat from another window (Quick Recall):
      // hydrate it from the live snapshot, then fall through to the contract.
      const snapshot = await fetchSnapshot(conversationId);
      if (activeConversationId !== conversationId || snapshot === null || snapshot.view.turnIndex !== event.turnIndex) return;
      // Re-check: a concurrent event for the same index may have hydrated it.
      turn = turns.find((t) => t.turnIndex === event.turnIndex);
      if (!turn) {
        const hydrated = makeTurn(snapshot.view.turnIndex, snapshot.view.question, normalizePhase(snapshot.view.phase));
        turns = [...turns, hydrated].sort((a, b) => a.turnIndex - b.turnIndex);
        turn = turns.find((t) => t.turnIndex === event.turnIndex);
        if (!turn) return;
        adoptView(turn, snapshot.view, snapshot.version);
        reconcileStreaming(turn);
      }
    }
    if (event.version === turn.version + 1) {
      applyUpdate(turn, event.update);
      turn.version = event.version;
      if (event.update.op === "error" && turn.turnIndex === turns.length - 1) restoreFailedQuestion(turn.question);
      reconcileStreaming(turn);
      if (atBottom) void tick().then(scrollToBottom);
      return;
    }
    if (event.version <= turn.version) return;
    // Gap: we missed updates. Self-heal from the snapshot, else the store.
    const snapshot = await fetchSnapshot(conversationId);
    if (activeConversationId !== conversationId) return;
    if (snapshot !== null && snapshot.view.turnIndex === turn.turnIndex) {
      adoptView(turn, snapshot.view, snapshot.version);
      reconcileStreaming(turn);
      if (atBottom) void tick().then(scrollToBottom);
      return;
    }
    try {
      const convo = await invoke<Conversation | null>("get_conversation", { conversationId });
      if (convo === null || activeConversationId !== conversationId) return;
      const index = turn.turnIndex;
      const fresh = convo.turns.find((t) => t.turnIndex === index);
      if (fresh) {
        const hydrated = hydrateTurn(fresh);
        turns = turns.map((t) => (t.turnIndex === index ? hydrated : t));
        reconcileStreaming(hydrated);
      }
    } catch {
      // Best-effort: leave the turn as-is.
    }
  }

  function reconcileStreaming(turn: ChatTurn): void {
    if (turn.turnIndex === turns.length - 1) streaming = turn.phase !== "done" && turn.phase !== "error";
  }

  // ── Header actions ───────────────────────────────────────────────────────
  const headerSummary = (): ConversationSummary =>
    activeSummary ?? ({ conversationId: activeConversationId ?? "", title: displayTitle, preview: "" } as ConversationSummary);
  async function deleteActive(): Promise<void> {
    if (await store.deleteConversation(headerSummary())) ondeleted?.();
  }

  onMount(() => {
    void loadAskAvailability();
    // Warm MCP connectors so a turn finds their tools ready (fire-and-forget).
    void invoke("mcp_warm_connectors").catch(() => {});
    void store.ensureStarted();
    void loadEngineSettings();

    let destroyed = false;
    const unlisteners: (() => void)[] = [];
    const keep = (fn: () => void) => (destroyed ? fn() : unlisteners.push(fn));
    void listen<AskAiUpdateEvent>("ask_ai_update", (e) => void handleUpdateEvent(e.payload)).then(keep);
    const refresh = () => {
      void loadAskAvailability();
      void loadEngineSettings();
    };
    void listen("user_context_changed", refresh).then(keep);
    // An inline "Sign in again" (ChatErrorTurn) landed: bring the composer back.
    void listen("chatgpt_login_update", () => void loadAskAvailability()).then(keep);
    void listen<RecordingSettingsDomainUpdateResponse>("recording_settings_domain_changed", (e) => {
      if (e.payload.domain === "ai_runtime" || e.payload.domain === "access") refresh();
    }).then(keep);
    // A streaming turn outlives this pane (ADR 0033): never cancel on teardown;
    // returning reattaches via the bus replay + snapshot.
    return () => {
      destroyed = true;
      for (const fn of unlisteners) fn();
    };
  });
</script>

{#snippet composerSlot(docked: boolean)}
  <ChatComposer
    bind:this={composer}
    bind:value={composerInput}
    bind:pickerOpen
    {docked}
    {streaming}
    available={askAvailability === null ? null : askAvailable}
    off={composerOff}
    onoffaction={() => (composerOff.retry ? void loadAskAvailability() : void openSettings("intelligence"))}
    {sendTo}
    {aiRuntime}
    {askAiModelOverride}
    {pinProvider}
    {pinModel}
    onselect={(engine) => void handleModelSelect(engine)}
    onsend={() => void send()}
    onstop={() => void stopStreaming()}
  />
{/snippet}

<section class="ch-main" data-mode={mode} aria-label="Conversation">
  {#if mode === "chat"}
    <header class="ch-head">
      <h1>{displayTitle}</h1>
      {#if headCount > 0}<span class="mx-label">{headCount} turn{headCount === 1 ? "" : "s"}{headStarted ? ` · started ${chatWhen(headStarted)}` : ""}</span>{/if}
      <div class="ch-head__acts">
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Rename chat" use:tip={"Rename"} disabled={!activeSummary} onclick={() => activeSummary && store.startRename(activeSummary)}>
          <IconEdit width="15" height="15" aria-hidden="true" />
        </button>
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm" aria-label="Delete chat" use:tip={"Delete"} onclick={() => void deleteActive()}>
          <IconTrash width="15" height="15" aria-hidden="true" />
        </button>
      </div>
    </header>
  {/if}

  <div class="ch-scroll" bind:this={transcriptEl} onscroll={onTranscriptScroll} aria-live="polite">
    {#if mode === "new"}
      <ChatEmpty suggest={askAvailable} onpick={(text) => { composerInput = text; composer?.focus(); }}>
        {@render composerSlot(false)}
      </ChatEmpty>
    {:else if mode === "missing"}
      <div class="ch-thread">
        <div class="mx-empty mx-empty--center">
          <span class="mx-empty__glyph"><IconTrash width="18" height="18" aria-hidden="true" /></span>
          <h4 class="mx-empty__title">This chat was deleted</h4>
          <p class="mx-empty__text">It isn’t on this Mac anymore.</p>
          <div class="mx-empty__acts"><button type="button" class="mx-btn mx-btn--sm" onclick={() => store.requestNewChat()}>New chat</button></div>
        </div>
      </div>
    {:else}
      <div class="ch-thread">
        {#if loadingConversation}
          <div aria-busy="true">
            <span class="mx-sr">Loading chat</span>
            <div class="mx-skel ch-skel-q"></div>
            <div class="mx-skel mx-skel--title ch-skel-title"></div>
            <div class="mx-skel-lines"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
            <div class="mx-skel-chart ch-skel-chart"><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div><div class="mx-skel"></div></div>
          </div>
        {:else if conversationLoadError !== null}
          <div class="mx-empty mx-empty--center" data-tone="danger" role="alert">
            <span class="mx-empty__glyph"><IconAlert width="18" height="18" aria-hidden="true" /></span>
            <h4 class="mx-empty__title">Couldn’t open this chat</h4>
            <p class="mx-empty__text">It’s still saved on this Mac. {conversationLoadError}</p>
            <div class="mx-empty__acts">
              <button type="button" class="mx-btn mx-btn--sm" onclick={() => activeConversationId && void loadConversationById(activeConversationId)}>
                <IconRetry width="13" height="13" aria-hidden="true" />Try again
              </button>
            </div>
          </div>
        {:else}
          {#each turns as turn, ti (turn.turnIndex)}
            <ChatTurnView
              {turn}
              trailing={ti === turns.length - 1}
              model={engineModel === null ? null : shortModelLabel(engineModel)}
              where={engineWhere}
              {contextWindow}
              settings={aiRuntime}
              retryDisabled={streaming || !askAvailable}
              onRetry={() => void retryTurn(turn)}
            />
          {/each}
        {/if}
      </div>
    {/if}
  </div>

  {#if mode === "chat" && !loadingConversation && conversationLoadError === null}
    <div class="ch-dock">
      {#if !atBottom}
        <button type="button" class="mx-btn mx-btn--sm ch-jump" onclick={() => void tick().then(scrollToBottom)}>
          <IconDown width="13" height="13" aria-hidden="true" />Jump to latest
        </button>
      {/if}
      {@render composerSlot(true)}
    </div>
  {/if}
</section>

<style>
  .ch-main { position: relative; flex: 1 1 auto; min-height: 0; min-width: 0; display: grid; grid-template-rows: auto minmax(0, 1fr) auto; }
  .ch-head {
    grid-row: 1;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    padding: 0 var(--s-4) 0 var(--s-6);
    height: 48px;
    border-bottom: 1px solid var(--mx-hairline);
    background: color-mix(in srgb, var(--app-bg) 70%, transparent);
  }
  .ch-head h1 { margin: 0; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: 600 var(--text-lg)/1.2 var(--font-sans); letter-spacing: -0.01em; color: var(--app-text-strong); }
  .ch-head .mx-label { white-space: nowrap; font-size: var(--text-xs); }
  .ch-head__acts { margin-left: auto; display: flex; gap: 2px; }
  .ch-scroll { grid-row: 2; min-height: 0; overflow: auto; }
  .ch-dock { grid-row: 3; position: relative; padding: 0 var(--s-6) var(--s-4); background: linear-gradient(to top, var(--app-bg) 70%, transparent); }
  .ch-jump { position: absolute; left: 50%; bottom: calc(100% + 8px); translate: -50% 0; z-index: 4; }
  .ch-thread { max-width: 760px; margin: 0 auto; padding: var(--s-6) var(--s-6) var(--s-5); }
  .ch-thread > .mx-empty--center { margin-top: 12vh; }
  .ch-skel-q { width: 46%; height: 40px; margin: 0 0 var(--s-5) auto; border-radius: var(--r-lg); }
  .ch-skel-title { width: 30%; margin-bottom: 16px; }
  .ch-skel-chart { margin-top: 24px; }
</style>
