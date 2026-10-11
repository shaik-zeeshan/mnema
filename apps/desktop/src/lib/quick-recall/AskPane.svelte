<!-- Quick Recall's ask slot (recall.html `.rc-gate` / `.rc-ans` / the askx
     notices): the hero slot above the moments when a question is typed. Three
     shapes: "Press Tab to ask <model>" when auto-answer is off for this model
     (`gate`), a notice when Ask AI can't answer (the moments stay below), or
     the answer streaming in place with its cited evidence rows numbered under
     it. ONE turn per question over the unchanged `ask_ai_start` session: an
     edit cancels the in-flight turn and asks again after the pause, "Continue
     in Chat ↵" takes the thread on. Rendering only — the backend owns the
     `TurnView`, the blocks and the `ask_ai_update` ops (lib/chat/turn-model).
     The page routes keys here first (`handleKey`) and renders Field's
     `fieldFoot` with `footKeys()` in BarShell's foot. -->
<script lang="ts">
  import { quickRecallSearch as search } from "./searchStore.svelte";
  import { onMount, onDestroy, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { message } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import AnswerProse from "$lib/AnswerProse.svelte";
  import MiniBars from "$lib/insights/charts/MiniBars.svelte";
  import { askAiClock } from "$lib/askAiClock";
  import { humanizeError } from "$lib/format-error";
  import { closeCurrentWindow, openSettings } from "$lib/surface-windows";
  import { adoptView, answerPlainText, applyUpdate, hydrateTurn, makeTurn, type ChatTurn } from "$lib/chat/turn-model";
  import { turnErrorCopy } from "$lib/insights/engine-state";
  import type {
    AskAiAvailability,
    AskAiSource,
    AskAiUpdateEvent,
    Conversation,
    TurnSnapshot,
  } from "$lib/insights/conversation";
  import type { QuickRecallAutoAnswer, RecordingSettings } from "$lib/types/recording";
  import { appIcons } from "./app-icons.svelte";
  import { askEngine, askNotice } from "./question-detect";
  import IconSparkle from "~icons/lucide/sparkle";
  import IconStop from "~icons/lucide/square";
  import IconAlert from "~icons/lucide/triangle-alert";
  import IconRetry from "~icons/lucide/rotate-ccw";
  import IconChat from "~icons/lucide/message-circle";
  import IconMic from "~icons/lucide/mic";
  import IconSpeaker from "~icons/lucide/volume-2";
  import IconEye from "~icons/lucide/eye";
  import IconOff from "~icons/lucide/ban";
  import IconCloudOff from "~icons/lucide/cloud-off";

  let {
    view,
    question,
    forced,
    availability,
    settings,
    autoMode,
    focused,
    onAsk,
    onLook,
    streaming = $bindable(false),
    pending = $bindable(false),
  }: {
    view: "gate" | "ask";
    /** The scoped question (scopedQuestion); answered once it holds still. */
    question: string;
    /** Tab asked: no pause before sending. */
    forced: boolean;
    availability: AskAiAvailability | null;
    settings: RecordingSettings | null;
    autoMode: QuickRecallAutoAnswer;
    focused: boolean;
    onAsk: () => void;
    /** Quick Look on a cited item; `onClose` brings the selected row back. Absent → no eye/Space. */
    onLook?: (sources: AskAiSource[], at: number, onClose: (at: number) => void) => void;
    /** A turn is in flight (suspends the page's idle-clear). */
    streaming?: boolean;
    /** Something the idle-clear must not wipe: an unseen or in-flight answer, or a source hand-off. */
    pending?: boolean;
  } = $props();

  const PAUSE_MS = 750;
  const TITLE_MAX = 80;

  type Turn = ChatTurn & { stoppedEarly?: boolean };

  let online = $state(navigator.onLine);
  const engine = $derived(askEngine(settings));
  const notice = $derived(
    view === "ask" && availability !== null
      ? askNotice(availability.available ? null : (availability.reason ?? ""), settings, online, engine)
      : null,
  );
  const canAsk = $derived(view === "ask" && availability?.available === true && notice === null);

  let turn = $state<Turn | null>(null);
  // The live thread (the `ask_ai_update` guard; null once stopped/cancelled)
  // and the persisted one "Continue in Chat" opens (survives a Stop).
  let liveId = $state<string | null>(null);
  let threadId = $state<string | null>(null);
  // The question the current turn answers — an edit away from it re-asks.
  let asked = $state("");
  let seen = $state(false);
  let handoff = $state(false);
  // Selection: 0 = the answer, n = evidence row n. `nav` once ↑/↓ moved into
  // the list (then ←/→ walk citations instead of the caret); `cite` = the lit row.
  let sel = $state(0);
  let nav = $state(false);
  let cite = $state(0);
  let copied = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;

  const live = $derived(turn !== null && liveId !== null && (turn.phase === "thinking" || turn.phase === "streaming"));
  const sources = $derived(turn?.sources ?? []);
  const waiting = $derived(canAsk && question.trim() !== "" && question.trim() !== asked);
  const errorCopy = $derived(
    turn?.phase === "error" ? turnErrorCopy(turn.errorMessage, turn.errorKind, settings?.aiRuntime ?? null) : null,
  );

  $effect(() => {
    streaming = live;
  });
  $effect(() => {
    pending = handoff || (liveId !== null && (live || !seen));
  });
  // Seen once a settled answer was on screen while the window had focus.
  $effect(() => {
    if (focused && turn && (turn.phase === "done" || turn.phase === "error")) seen = true;
  });
  // A re-summon ends the source hand-off's grace.
  $effect(() => {
    if (focused) untrack(() => (handoff = false));
  });

  // Answer once the question holds still; an edit cancels the turn in flight.
  $effect(() => {
    const q = question.trim();
    const delay = forced ? 0 : PAUSE_MS;
    if (!canAsk || q === "" || q === asked) return;
    untrack(() => {
      void cancel();
      turn = null;
    });
    timer = setTimeout(() => void start(q), delay);
    return () => {
      if (timer !== null) clearTimeout(timer);
    };
  });

  async function start(q: string): Promise<void> {
    asked = q;
    await cancel();
    const id = crypto.randomUUID();
    liveId = id;
    threadId = id;
    seen = false;
    sel = 0;
    nav = false;
    cite = 0;
    turn = makeTurn(0, q, "thinking");
    const title = q.replace(/\s+/g, " ");
    try {
      await invoke("ask_ai_start", {
        request: {
          conversationId: id,
          question: q,
          origin: "quick_recall",
          title: title.length > TITLE_MAX ? `${title.slice(0, TITLE_MAX - 1)}…` : title,
          ...askAiClock(),
        },
      });
    } catch (error) {
      if (liveId !== id || !turn) return;
      turn.phase = "error";
      turn.errorMessage = humanizeError(error);
    }
  }

  async function cancel(): Promise<void> {
    const id = liveId;
    if (id === null) return;
    liveId = null;
    try {
      await invoke("ask_ai_cancel", { request: { conversationId: id } });
    } catch {
      // Best-effort: the turn is being abandoned.
    }
  }

  // Stop in place: the partial stays (persisted as a done turn), still
  // continuable in Chat; late updates stop matching once liveId drops. Stopped
  // before any answer, the backend discards the thread — nothing to continue.
  async function stop(): Promise<void> {
    if (!turn || !live) return;
    if (turn.blocks.length === 0) threadId = null;
    turn.phase = "done";
    turn.liveActivity = null;
    turn.stoppedEarly = true;
    await cancel();
  }

  // Exactly-next applies; a gap re-snapshots, or re-reads a turn that finished.
  async function onUpdate(event: AskAiUpdateEvent): Promise<void> {
    const t = turn;
    if (event.conversationId !== liveId || !t || event.turnIndex !== t.turnIndex) return;
    if (event.version === t.version + 1) {
      applyUpdate(t, event.update);
      t.version = event.version;
      if (event.update.op === "sources") appIcons.ensure(t.sources.map((s) => s.appName));
      return;
    }
    if (event.version <= t.version) return;
    const id = event.conversationId;
    const snap = await invoke<TurnSnapshot | null>("ask_ai_snapshot", { request: { conversationId: id } }).catch(
      () => undefined,
    );
    if (snap === undefined || liveId !== id || turn !== t) return;
    if (snap && snap.view.turnIndex === t.turnIndex) {
      adoptView(t, snap.view, snap.version);
      return;
    }
    const convo = await invoke<Conversation | null>("get_conversation", { conversationId: id }).catch(() => null);
    const fresh = convo?.turns.find((x) => x.turnIndex === t.turnIndex);
    if (fresh && liveId === id && turn === t) turn = hydrateTurn(fresh);
  }

  async function openInChat(): Promise<void> {
    if (threadId === null || turn?.phase !== "done") return;
    try {
      await invoke("open_conversation_in_chat", { conversationId: threadId });
    } catch (err) {
      await message(`Couldn't open this in Chat: ${humanizeError(err, "please try again.")}`, {
        title: "Couldn't continue in Chat",
        kind: "error",
      });
      return;
    }
    await closeCurrentWindow();
  }

  // A cited item hops to the main window's Timeline; the answer is kept for
  // the re-summon (handoff) instead of being idle-cleared behind the user.
  async function openSource(source: AskAiSource): Promise<void> {
    try {
      await invoke("open_capture_result_in_main_window", {
        kind: source.kind,
        frameId: source.frameId,
        audioSegmentId: source.audioSegmentId,
        spanStartMs: source.spanStartMs ?? null,
        alignedFrameId: source.alignedFrameId ?? null,
        query: asked,
      });
    } catch (err) {
      await search.surfaceResultHandoffFailure(err);
      return;
    }
    handoff = true;
    await closeCurrentWindow();
  }

  async function copyAnswer(): Promise<void> {
    const text = turn ? answerPlainText(turn) : "";
    if (!text) return;
    await navigator.clipboard.writeText(text).catch(() => {});
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  // Answer links open in the browser; suppress the blur-dismiss that would
  // otherwise close the panel out from under the user first.
  async function openAnswerLink(href: string): Promise<void> {
    await invoke("quick_recall_suppress_blur_dismiss").catch(() => {});
    try {
      await openUrl(href);
    } catch (err) {
      await message(`Couldn't open URL: ${humanizeError(err, "the link could not be opened")}`, {
        title: "Couldn't open link",
        kind: "error",
      });
    }
  }

  // Quick Look on source `i` (↑/↓ there walk the sources). When it closes,
  // the source it ended on is selected — or lit, when it opened from the
  // answer's citations — and scrolled into view.
  function look(i: number, fromCite: boolean): void {
    onLook?.(sources, i, (at) => {
      if (fromCite) cite = at + 1;
      else [sel, cite] = [at + 1, 0];
      requestAnimationFrame(() => document.querySelector(`[data-ask-row="${at + 1}"]`)?.scrollIntoView({ block: "nearest" }));
    });
  }

  function noticeAction(): void {
    void openSettings("intelligence");
  }

  /** The page's window capture listener routes keys here first; true = used. */
  export function handleKey(e: KeyboardEvent): boolean {
    if (e.isComposing) return false;
    const plain = !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") nav = true;
    else if (e.key.length === 1 && e.key !== " " && !e.metaKey && !e.ctrlKey) nav = false;
    if (view === "gate" || notice) {
      if (view === "gate" && e.key === "Enter" && plain && !nav) {
        onAsk();
        return true;
      }
      return false;
    }
    const n = sources.length;
    switch (e.key) {
      case "Escape":
        if (!plain || !live) return false;
        void stop();
        return true;
      case "ArrowDown":
      case "ArrowUp":
        sel = Math.max(0, Math.min(n, sel + (e.key === "ArrowDown" ? 1 : -1)));
        cite = 0;
        document.querySelector(`[data-ask-row="${sel}"]`)?.scrollIntoView({ block: "nearest" });
        return true;
      case "ArrowLeft":
      case "ArrowRight":
        if (!nav || sel !== 0 || n === 0 || !plain) return false;
        cite = (((cite || (e.key === "ArrowRight" ? 0 : 1)) - 1 + (e.key === "ArrowRight" ? 1 : -1) + n) % n) + 1;
        document.querySelector(`[data-ask-row="${cite}"]`)?.scrollIntoView({ block: "nearest" });
        return true;
      case " ": {
        const i = sel > 0 ? sel - 1 : (cite || 1) - 1;
        if (!nav || !onLook || !sources[i]) return false;
        look(i, sel === 0);
        return true;
      }
      case "Enter":
        if (sel > 0) {
          const s = sources[sel - 1];
          if (e.metaKey || e.ctrlKey) {
            if (s.frameId != null) void search.openCapturedFrameUrl(s.frameId);
          } else void openSource(s);
        } else if (turn?.phase === "done") void openInChat();
        else if (turn?.phase === "error") void start(asked);
        return true;
      case "c":
      case "C": {
        const el = search.inputEl;
        const typedSelection = el !== null && el.selectionStart !== el.selectionEnd;
        if (!(e.metaKey || e.ctrlKey) || sel !== 0 || turn?.phase !== "done" || typedSelection) return false;
        void copyAnswer();
        return true;
      }
    }
    return false;
  }

  /** The keys that work right now, for the foot. */
  export function footKeys(): [string, string][] {
    if (view === "gate") return nav ? [["↵", "open"]] : [["⇥", "ask"], ["↓", "moments"]];
    if (notice) return [["↓", "moments"], ["⇥", "just search"]];
    if (sel > 0) return [["↵", "open"], ...(onLook ? [["space", "look"] as [string, string]] : [])];
    if (waiting || live) return [["esc", live ? "stop" : "clear"]];
    if (turn?.phase === "error") return [["↵", "retry"], ["esc", "clear"]];
    if (cite > 0) return [["←→", "source"], ...(onLook ? [["space", "look"] as [string, string]] : []), ["↵", "continue in chat"]];
    return [
      ["↵", "continue in chat"],
      nav && sources.length > 0 ? ["←→", "sources"] : ["↓", "evidence"],
      ["⌘C", copied ? "copied" : "copy"],
    ];
  }

  /** The moments (search results) belong under the gate and the notices. */
  export function momentsBelow(): boolean {
    return view === "gate" || notice !== null;
  }

  onMount(() => {
    let destroyed = false;
    let unlisten: (() => void) | undefined;
    listen<AskAiUpdateEvent>("ask_ai_update", (event) => void onUpdate(event.payload)).then((fn) => {
      if (destroyed) fn();
      else unlisten = fn;
    });
    return () => {
      destroyed = true;
      unlisten?.();
    };
  });

  onDestroy(() => {
    if (timer !== null) clearTimeout(timer);
    streaming = false;
    pending = false;
    void cancel();
  });

  function when(iso: string): string {
    return new Date(iso).toLocaleString(undefined, { weekday: "short", hour: "2-digit", minute: "2-digit", hour12: false });
  }

  const fromLine = $derived.by(() => {
    const n = sources.length;
    return n === 0 ? "" : `from ${n} moment${n === 1 ? "" : "s"}`;
  });
  const timelines = $derived((turn?.blocks ?? []).filter((b) => b.kind === "timeline"));
</script>

<svelte:window ononline={() => (online = true)} onoffline={() => (online = false)} />

{#if view === "gate"}
  <button type="button" class="rc-gate" onclick={onAsk}>
    <IconSparkle width="15" height="15" aria-hidden="true" />
    <span class="rc-gate__t">Press <kbd>Tab</kbd> to ask <b>{engine.model ?? "Ask AI"}</b></span>
    <span class="rc-gate__why"
      >{autoMode === "never" || engine.local ? "Answers only when you ask" : "Cloud model — answers only when you ask"}</span
    >
  </button>
{:else if notice}
  <div class="rc-slot">
    <div class="mx-notice" data-tone={notice.tone || undefined} role="status">
      <span class="mx-notice__icon">
        {#if notice.tone === "off"}<IconOff width="15" height="15" />
        {:else if notice.tone === "warn"}<IconCloudOff width="15" height="15" />
        {:else if notice.tone === "danger"}<IconAlert width="15" height="15" />
        {:else}<IconSparkle width="15" height="15" />{/if}
      </span>
      <b class="mx-notice__title">{notice.title}</b>
      <p class="mx-notice__text">{notice.text}</p>
      {#if notice.action}
        <div class="mx-notice__acts">
          <button
            type="button"
            class="mx-btn mx-btn--sm"
            class:mx-btn--ghost={!notice.action.primary}
            onclick={noticeAction}>{notice.action.label}</button
          >
        </div>
      {/if}
    </div>
  </div>
{:else}
  <article class="rc-ans" data-sel={sel === 0 || undefined} aria-busy={live || waiting} aria-live="polite">
    <header class="rc-ans__h">
      <IconSparkle width="15" height="15" aria-hidden="true" />
      <span>Answer</span>
      {#if waiting || !turn}
        <span class="rc-livetag rc-livetag--idle">answers when you pause</span>
      {:else if turn.phase === "thinking"}
        <span class="rc-livetag">{turn.liveActivity?.label ?? "Thinking…"}</span>
      {:else if live}
        <button type="button" class="mx-btn mx-btn--ghost mx-btn--sm" onclick={() => void stop()}
          ><IconStop width="10" height="10" aria-hidden="true" />Stop<kbd>esc</kbd></button
        >
      {:else if turn.phase === "error"}
        <span class="mx-inline rc-ans__err" data-tone="danger"
          ><IconAlert width="13" height="13" aria-hidden="true" />Stopped partway<button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--sm"
            onclick={() => void start(asked)}><IconRetry width="12" height="12" aria-hidden="true" />Retry</button
          ></span
        >
      {:else}
        <span class="mono">{turn.stoppedEarly ? "stopped" : fromLine}</span>
        {#if threadId !== null}<button
          type="button"
          class="mx-btn mx-btn--sm"
          class:mx-btn--primary={sel === 0}
          onclick={() => void openInChat()}
          ><IconChat width="13" height="13" aria-hidden="true" />Continue in Chat<kbd>↵</kbd></button
        >{/if}
      {/if}
    </header>
    <div class="rc-ans__body" class:rc-ans__body--split={timelines.length > 0}>
      {#if !turn || (turn.blocks.length === 0 && turn.phase !== "error" && turn.phase !== "done")}
        <div class="rc-ans__wait">
          <div class="mx-skel" style="width:92%"></div>
          <div class="mx-skel" style="width:78%"></div>
          <div class="mx-skel" style="width:54%"></div>
        </div>
      {:else}
        <div class="rc-ans__prose">
          {#each turn.blocks as block, bi (bi)}
            {#if block.kind === "prose"}
              <AnswerProse
                source={block.markdown}
                isStreaming={live && bi === turn.blocks.length - 1}
                onOpenLink={openAnswerLink}
              />
            {:else if block.kind === "bars"}
              <div class="mx-panel rc-block">
                {#if block.title}<h5>{block.title}</h5>{/if}
                <MiniBars items={block.items} />
              </div>
            {:else if block.kind === "dossier"}
              <ul class="rc-dossier">
                {#each block.items as item, di (di)}<li>{item.statement}</li>{/each}
              </ul>
            {/if}
          {/each}
          {#if errorCopy}<p class="rc-ans__errtext">{errorCopy.text}</p>{/if}
          {#if turn.phase === "done" && turn.blocks.length === 0}
            <p class="mx-body-sm">No answer came back. Press ↵ in a moment, or ask again.</p>
          {/if}
        </div>
        {#if timelines.length > 0}
          <div class="rc-tl">
            {#each timelines as block, ti (ti)}
              {#if block.kind === "timeline"}
                {#if block.title}<h5>{block.title}</h5>{/if}
                <ol>
                  {#each block.items as item, ii (ii)}
                    <li><time>{item.start}</time><i></i><span>{item.label}</span></li>
                  {/each}
                </ol>
              {/if}
            {/each}
          </div>
        {/if}
      {/if}
    </div>
  </article>

  {#if sources.length > 0}
    <div class="rc-sec">Evidence<span class="mono">{sources.length} cited</span></div>
    {#each sources as s, i (i)}
      {@const icon = s.kind === "frame" ? appIcons.src(s.appName) : null}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <div
        class="rc-row"
        role="option"
        tabindex="-1"
        data-ask-row={i + 1}
        aria-selected={sel === i + 1}
        data-hot={cite === i + 1 || undefined}
        onclick={() => void openSource(s)}
      >
        <span class="rc-num">{i + 1}</span>
        {#if s.kind === "audio"}
          <i class="rc-app" data-src={s.sourceKind === "microphone" ? "mic" : "system"}
            >{#if s.sourceKind === "microphone"}<IconMic width="11" height="11" />{:else}<IconSpeaker
                width="11"
                height="11"
              />{/if}</i
          >
        {:else if icon}
          <img class="rc-app" src={icon} alt="" />
        {:else}
          <i class="rc-app">{(s.appName ?? "?").charAt(0).toUpperCase()}</i>
        {/if}
        <span class="rc-row__t"
          >{s.windowTitle || s.appName || (s.kind === "audio" ? "Recording" : "Screen")}<span
            >{s.kind === "audio"
              ? s.sourceKind === "microphone"
                ? "microphone"
                : "system audio"
              : (s.url ?? (s.windowTitle ? (s.appName ?? "") : ""))}</span
          ></span
        >
        <time>{when(s.startedAt)}</time>
        {#if onLook}
          <button
            type="button"
            class="mx-btn mx-btn--ghost mx-btn--icon mx-btn--sm rc-peek"
            tabindex="-1"
            aria-label="Quick Look"
            onclick={(event) => {
              event.stopPropagation();
              look(i, false);
            }}><IconEye width="14" height="14" /></button
          >
        {:else}
          <span></span>
        {/if}
      </div>
    {/each}
  {/if}
{/if}

<style>
  /* recall.html's ask parts, one rule per line where it stays readable. */
  .rc-gate {
    display: flex; align-items: center; gap: 10px; width: 100%; padding: 10px 10px 10px 14px;
    border: 0; border-radius: var(--r-lg); background: var(--mx-selected); font: inherit; text-align: left; cursor: pointer;
  }
  .rc-gate > :global(svg) { flex: none; color: var(--app-text-subtle); }
  .rc-gate__t { font: 500 var(--text-md) / 1.3 var(--font-sans); color: var(--app-text); }
  .rc-gate__t b { font-weight: 600; color: var(--app-text-strong); }
  .rc-gate__t kbd { height: 18px; margin: 0 2px; }
  .rc-gate__why { margin-left: auto; font: 400 var(--text-sm) / 1.3 var(--font-sans); color: var(--app-text-subtle); white-space: nowrap; }

  .rc-slot { flex: none; padding: 6px; border-radius: var(--r-lg); background: var(--mx-selected); }
  .rc-slot .mx-notice { max-width: none; }

  .rc-ans { flex: none; padding: 14px 16px 12px; border-radius: var(--r-lg); }
  .rc-ans[data-sel] { background: var(--mx-selected); }
  .rc-ans__h {
    display: flex; align-items: center; gap: 8px; min-height: 26px;
    font: 600 var(--text-md) / 1 var(--font-sans); color: var(--app-text-strong);
  }
  .rc-ans__h > :global(svg) { color: var(--app-accent); }
  .rc-ans__h .mono { font: 400 var(--text-sm) / 1 var(--font-mono); color: var(--app-text-subtle); }
  .rc-ans__h > .mx-btn, .rc-ans__err { margin-left: auto; }
  .rc-ans__err { gap: 10px; }
  .rc-livetag {
    display: inline-flex; align-items: center; gap: 7px; margin-left: auto;
    font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-muted);
  }
  .rc-livetag::before {
    content: ""; width: 6px; height: 6px; border-radius: 50%;
    background: var(--app-accent); box-shadow: 0 0 0 3px var(--app-accent-glow);
  }
  .rc-livetag--idle::before { background: var(--chart-grey-4); box-shadow: none; }
  .rc-ans__body { margin-top: 10px; }
  /* The timeline block beside the prose keeps the answer within the bar's cap. */
  .rc-ans__body--split { display: grid; grid-template-columns: minmax(0, 1fr) 320px; gap: 18px; align-items: start; }
  .rc-ans__prose { display: grid; gap: 10px; min-width: 0; }
  .rc-ans .rc-ans__prose :global(.answer-prose) { font-size: 14px; line-height: 1.65; }
  .rc-ans__errtext { margin: 0; font: 400 14px / 1.65 var(--font-sans); color: var(--app-text); }
  .rc-ans__wait { display: grid; gap: 8px; padding: 4px 0 2px; }
  .rc-ans__wait .mx-skel { height: 11px; }
  .rc-block { padding: 10px 12px; }
  .rc-block h5 { margin: 0 0 6px; font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle); }
  .rc-tl { padding: 10px 12px 6px; border-radius: var(--r-md); background: var(--app-surface); border: 1px solid var(--app-border); }
  .rc-tl h5 { margin: 0 0 6px; font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle); }
  .rc-tl ol { position: relative; margin: 0; padding: 0; list-style: none; }
  .rc-tl ol::before { content: ""; position: absolute; left: 68px; top: 12px; bottom: 12px; width: 1px; background: var(--app-border-strong); }
  .rc-tl li { display: grid; grid-template-columns: 54px 9px minmax(0, 1fr); gap: 0 10px; align-items: center; padding: 4px 0; }
  .rc-tl time { font: 400 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-subtle); text-align: right; font-variant-numeric: tabular-nums; }
  .rc-tl li > i { position: relative; width: 9px; height: 9px; border-radius: 50%; background: var(--app-bg); border: 1.5px solid var(--chart-grey-4); box-sizing: border-box; }
  .rc-tl li span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font: 400 var(--text-base) / 1.45 var(--font-sans); color: var(--app-text); }
  .rc-dossier { margin: 0; padding-left: 18px; color: var(--app-text); }

  .rc-sec {
    display: flex; align-items: baseline; gap: 8px; margin: 10px 10px 4px;
    font: 500 var(--text-base) / 1 var(--font-sans); color: var(--app-text-subtle);
  }
  .rc-sec .mono { margin-left: auto; font-size: var(--text-xs); font-variant-numeric: tabular-nums; }
  .rc-row {
    flex: none; display: grid; grid-template-columns: 18px 18px minmax(0, 1fr) auto 26px; gap: 10px; align-items: center;
    height: 34px; padding: 0 4px 0 10px; border-radius: var(--r-md); cursor: default; color: var(--app-text);
  }
  .rc-row:hover, .rc-row[data-hot] { background: var(--mx-wash); }
  .rc-row[aria-selected="true"] { background: var(--mx-selected); }
  .rc-peek { opacity: 0; }
  .rc-row:hover .rc-peek, .rc-row[aria-selected="true"] .rc-peek, .rc-peek:focus-visible { opacity: 1; }
  .rc-row__t {
    min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font: 500 var(--text-md) / 1 var(--font-sans); color: var(--app-text-strong);
  }
  .rc-row__t span { margin-left: 8px; font-weight: 400; color: var(--app-text-muted); }
  .rc-row time {
    min-width: 64px; text-align: right; font: 450 var(--text-sm) / 1 var(--font-mono);
    color: var(--app-text-subtle); font-variant-numeric: tabular-nums;
  }
  .rc-row[aria-selected="true"] time { color: var(--app-text); }
  .rc-num {
    display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: var(--r-xs);
    font: 500 var(--text-xs) / 1 var(--font-mono); color: var(--app-text-muted); box-shadow: inset 0 0 0 1px var(--app-border-strong);
  }
  .rc-row[data-hot] .rc-num { background: var(--app-text-strong); color: var(--app-bg); box-shadow: none; }
  .rc-app {
    --c: var(--chart-grey-4);
    display: inline-grid; place-items: center; width: 18px; height: 18px; border-radius: 5px;
    font: 700 10px / 1 var(--font-sans); font-style: normal; color: var(--c);
    background: color-mix(in srgb, var(--c) 18%, var(--app-surface));
    box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c) 30%, transparent);
  }
  .rc-app[data-src="mic"] { --c: var(--app-source-mic); }
  .rc-app[data-src="system"] { --c: var(--app-source-sysaudio); }
  img.rc-app { object-fit: contain; background: none; box-shadow: none; }
</style>
